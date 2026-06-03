//! eigen-daemon: the woland backend — pty manager + http/ws server.
//!
//! Slice 1: the pty bridge. Spawn an arbitrary command in a pty and stream its stdio.
//! The bridge drives ANY command; real `claude --resume` is launched only by the user,
//! never by tests or the agent. See `docs/plans/2026-06-03-woland-design.md`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Deserialize;

/// What the daemon runs when a terminal connects. For slice 1 this is a fixed command
/// (a shell for the demo, a dummy in tests) — NOT arbitrary exec from the request.
#[derive(Clone, Debug)]
pub struct Config {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Directory of static frontend assets to serve at `/`. None = API only.
    pub web_dir: Option<PathBuf>,
}

/// Build the woland HTTP/WS router. `GET /pty` upgrades to a websocket bridged to a pty;
/// `web_dir`, if set, is served as static files at `/`.
pub fn app(config: Config) -> Router {
    let mut router = Router::new().route("/pty", get(pty_ws));
    if let Some(web_dir) = &config.web_dir {
        let index = web_dir.join("index.html");
        router = router.fallback_service(
            tower_http::services::ServeDir::new(web_dir)
                .fallback(tower_http::services::ServeFile::new(index)),
        );
    }
    router.with_state(Arc::new(config))
}

/// Bind `addr` and serve woland until the process is killed.
pub async fn serve(addr: std::net::SocketAddr, config: Config) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app(config)).await?;
    Ok(())
}

async fn pty_ws(ws: WebSocketUpgrade, State(cfg): State<Arc<Config>>) -> Response {
    ws.on_upgrade(move |socket| bridge(socket, cfg))
}

/// Control messages from the browser. Output flows the other way as raw binary frames.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Control {
    Stdin { data: String },
    Resize { cols: u16, rows: u16 },
}

/// Bridge one websocket to a freshly-spawned pty: pty output → binary frames, client
/// control messages → stdin / resize.
async fn bridge(socket: WebSocket, cfg: Arc<Config>) {
    let args: Vec<&str> = cfg.args.iter().map(String::as_str).collect();
    let mut pty = match Pty::spawn(&cfg.program, &args, cfg.cwd.as_deref(), (80, 24)) {
        Ok(p) => p,
        Err(_) => return,
    };
    let reader = match pty.reader() {
        Ok(r) => r,
        Err(_) => return,
    };

    let (mut sink, mut stream) = socket.split();

    // Blocking pty reads live on a dedicated thread, forwarded over a channel.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let send_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if sink.send(Message::Binary(bytes)).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(t) => match serde_json::from_str::<Control>(&t) {
                Ok(Control::Stdin { data }) => {
                    let _ = pty.write_input(data.as_bytes());
                }
                Ok(Control::Resize { cols, rows }) => {
                    let _ = pty.resize(cols, rows);
                }
                Err(_) => {}
            },
            Message::Binary(b) => {
                let _ = pty.write_input(&b);
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    send_task.abort();
}

/// A command running in a pseudo-terminal: stream its output via [`Pty::reader`], send
/// input via [`Pty::write_input`], and follow the terminal size via [`Pty::resize`].
pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    #[allow(dead_code)]
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
}

impl Pty {
    /// Spawn `program` with `args` in a pty of `(cols, rows)`, optionally in `cwd`.
    pub fn spawn(
        program: &str,
        args: &[&str],
        cwd: Option<&Path>,
        size: (u16, u16),
    ) -> anyhow::Result<Pty> {
        let (cols, rows) = size;
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(program);
        for arg in args {
            cmd.arg(arg);
        }
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }

        let child = pair.slave.spawn_command(cmd)?;
        // Close the slave handle in the parent so EOF propagates when the child exits.
        drop(pair.slave);
        let writer = pair.master.take_writer()?;

        Ok(Pty {
            master: pair.master,
            child,
            writer,
        })
    }

    /// A fresh reader over the pty's output. Reads block until data or EOF.
    pub fn reader(&self) -> anyhow::Result<Box<dyn Read + Send>> {
        Ok(self.master.try_clone_reader()?)
    }

    /// Send bytes to the child's stdin.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    /// Tell the pty the terminal was resized to `(cols, rows)`.
    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}
