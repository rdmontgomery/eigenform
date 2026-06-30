use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

/// The default port `eigenform daemon` binds on; `eigenform ptys` targets the same default.
const DEFAULT_PORT: u16 = 4317;

#[derive(Parser, Debug)]
#[command(name = "eigenform", version, about = "control surface over Claude Code sessions")]
struct Cli {
    /// With no subcommand, `eigenform` starts the daemon and opens the browser.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// inspect skills available at hierarchical resolution levels
    Skills {
        #[command(subcommand)]
        action: SkillsAction,
    },
    /// inspect Claude Code auto-memory at the project / cross-project levels
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// unified config inventory: skills + memory across resolution layers, each
    /// annotated with a token estimate. Defaults to the current context; pass
    /// --all-projects for a machine-wide inventory.
    Inspect {
        /// override the working directory used to compute the repo layer
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// inventory every recorded project instead of just the current context
        #[arg(long)]
        all_projects: bool,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
    /// context surgery on a session JSONL: fork, rewind, inject (prints the new uuid)
    Surgery {
        #[command(subcommand)]
        action: SurgeryAction,
    },
    /// inspect sessions
    Sessions {
        #[command(subcommand)]
        action: SessionsAction,
    },
    /// the live Forest: sessions corroborated from disk (liveness × state × activity),
    /// the same snapshot woland's Forest shows
    Forest {
        /// only the live sessions (drop the recents)
        #[arg(long)]
        live: bool,
    },
    /// list live ptys held by the daemon (CLI mirror of GET /api/pty)
    Ptys {
        /// daemon port to query
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// list new-session directory candidates: recent cwds + workspace subdirs
    /// (CLI mirror of GET /api/candidates — computed locally, no daemon needed)
    Candidates {
        /// code root for the new-session launcher (default: ~/projects if it exists)
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
    /// run the eigenform daemon (serves the browser app + pty terminal at localhost)
    Daemon {
        /// port to bind on localhost
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
        /// command to run in the pty (default: $SHELL, else bash). NOT claude unless you ask.
        #[arg(long)]
        cmd: Option<String>,
        /// directory of the legacy woland build to serve at /woland (default: ./web if built)
        #[arg(long)]
        web: Option<PathBuf>,
        /// directory of the eigenform (webterm) build to serve at / (default: ./webterm if
        /// built, else the build baked into the binary)
        #[arg(long)]
        term: Option<PathBuf>,
        /// code root for the new-session launcher (default: ~/projects)
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// dev mode: inject the live-reload hook and serve /api/dev/reload
        #[arg(long)]
        dev: bool,
        /// open the app in a browser once the daemon is up
        #[arg(long)]
        open: bool,
    },
    /// stop the running background daemon (and the sessions it hosts)
    Stop {
        /// daemon port to stop
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// report whether a background daemon is running
    Status {
        /// daemon port to check
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum SessionsAction {
    /// render a session as a turn-tree (resolve by uuid, prefix, or path)
    Show {
        /// session uuid, unique prefix, or a path to the JSONL
        session: String,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
    /// diff two sessions side-by-side (source vs fork, aligned by turn uuid)
    Diff {
        /// source session: uuid, prefix, or path
        a: String,
        /// fork session: uuid, prefix, or path
        b: String,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
    /// list recent sessions (current project, last 7 days, by default)
    List {
        /// recency window: e.g. 7d, 24h, 30m, 2w, or `all`
        #[arg(long)]
        since: Option<String>,
        /// list across every project, not just the current one
        #[arg(long)]
        all_projects: bool,
        /// list the project at this cwd instead of the current directory
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = RenderFormat::Text)]
        render: RenderFormat,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum RenderFormat {
    Text,
    Json,
    Html,
}

#[derive(Subcommand, Debug)]
enum SurgeryAction {
    /// fork a session so it ends at a chosen turn (optionally editing that turn)
    Fork {
        /// source session: uuid, unique prefix, or a path
        session: String,
        /// uuid of the turn to fork at
        #[arg(long)]
        at: String,
        /// replace the turn's content with this file's text (edit-then-fork)
        #[arg(long)]
        edit: Option<PathBuf>,
        /// also print a source→fork diff to stderr
        #[arg(long)]
        diff: bool,
    },
    /// rewind a session to a chosen turn (prefix only)
    Rewind {
        /// source session: uuid, unique prefix, or a path
        session: String,
        /// uuid of the turn to rewind to
        #[arg(long)]
        to: String,
    },
    /// inject a synthetic turn after a chosen turn, as the new resume head
    Inject {
        /// source session: uuid, unique prefix, or a path
        session: String,
        /// uuid of the turn to inject after
        #[arg(long)]
        at: String,
        /// role of the injected turn
        #[arg(long = "as", value_enum)]
        as_role: RoleArg,
        /// file whose text becomes the injected turn's content
        #[arg(long)]
        content: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum RoleArg {
    User,
    Assistant,
    System,
}

impl From<RoleArg> for eigenform_surgery::Role {
    fn from(r: RoleArg) -> Self {
        match r {
            RoleArg::User => eigenform_surgery::Role::User,
            RoleArg::Assistant => eigenform_surgery::Role::Assistant,
            RoleArg::System => eigenform_surgery::Role::System,
        }
    }
}

#[derive(Subcommand, Debug)]
enum MemoryAction {
    /// memory entries for one project (defaults to current cwd)
    Tree {
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// memory entries across every project
    List {
        #[arg(long)]
        all_projects: bool,
    },
}

#[derive(Subcommand, Debug)]
enum SkillsAction {
    /// show the full override stack for one resolution context
    Tree {
        /// override the working directory used to compute the repo layer
        #[arg(long)]
        cwd: Option<PathBuf>,
    },
    /// inventory of skills across every Claude Code project on this machine
    List {
        /// scan every project under ~/.claude/projects/ in addition to global+plugins
        #[arg(long)]
        all_projects: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let Some(cmd) = cli.cmd else {
        // Zero-arg `eigenform`: the one-command launch. Reuse a running daemon if one
        // is up, else start it detached so the terminal is freed and the hosted
        // sessions survive closing it. `eigenform daemon` is still the foreground form.
        return launch(DEFAULT_PORT);
    };
    match cmd {
        Cmd::Skills { action } => match action {
            SkillsAction::Tree { cwd } => skills_tree(cwd),
            SkillsAction::List { all_projects } => skills_list(all_projects),
        },
        Cmd::Memory { action } => match action {
            MemoryAction::Tree { cwd } => memory_tree(cwd),
            MemoryAction::List { all_projects } => memory_list(all_projects),
        },
        Cmd::Inspect { cwd, all_projects, render } => inspect_cmd(cwd, all_projects, render),
        Cmd::Surgery { action } => surgery(action),
        Cmd::Ptys { port } => ptys_list(port),
        Cmd::Candidates { workspace } => candidates_list(workspace),
        Cmd::Daemon { port, cmd, web, term, workspace, dev, open } => daemon(port, cmd, web, term, workspace, dev, open),
        Cmd::Stop { port } => stop(port),
        Cmd::Status { port } => status(port),
        Cmd::Sessions { action } => match action {
            SessionsAction::Show { session, render } => sessions_show(session, render),
            SessionsAction::Diff { a, b, render } => sessions_diff(a, b, render),
            SessionsAction::List {
                since,
                all_projects,
                cwd,
                render,
            } => sessions_list(since, all_projects, cwd, render),
        },
        Cmd::Forest { live } => forest_list(live),
    }
}

/// Print the live pty roster — the CLI mirror of GET /api/pty.
///
/// One row per pty:  id  state  uuid-or-—  cwd  age
/// Age is derived from `lastActivity` when present, else `spawnedAt`.
/// Daemon unreachable → friendly message + nonzero exit (no panic, no backtrace).
fn ptys_list(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}/api/pty");
    let body: serde_json::Value = match ureq::get(&url).call() {
        Ok(resp) => resp
            .into_json()
            .with_context(|| format!("parsing JSON from {url}"))?,
        Err(ureq::Error::Transport(_)) => {
            eprintln!("daemon not running on :{port}");
            std::process::exit(1);
        }
        Err(e) => {
            return Err(anyhow::anyhow!("HTTP error from {url}: {e}"));
        }
    };

    let rows = body.as_array().with_context(|| "expected JSON array from /api/pty")?;
    if rows.is_empty() {
        // No ptys registered — print nothing (not an error).
        return Ok(());
    }

    let now = chrono::Utc::now();
    for row in rows {
        let id = row["id"].as_str().unwrap_or("?");
        let state = row["state"].as_str().unwrap_or("?");
        let uuid = row["uuid"].as_str().unwrap_or("—");
        let cwd = row["cwd"].as_str().unwrap_or("?");
        // Prefer lastActivity for age, fall back to spawnedAt.
        let age_ts = row["lastActivity"]
            .as_str()
            .or_else(|| row["spawnedAt"].as_str())
            .unwrap_or("");
        let age = if age_ts.is_empty() {
            "?".to_string()
        } else {
            match chrono::DateTime::parse_from_rfc3339(age_ts) {
                Ok(t) => {
                    let then = t.with_timezone(&chrono::Utc);
                    ago(then, now)
                }
                Err(_) => "?".to_string(),
            }
        };
        println!("{id:<4}  {state:<8}  {uuid:<36}  {cwd}  {age}");
    }
    Ok(())
}

/// Print the launcher candidate list — the CLI mirror of GET /api/candidates.
///
/// One row per candidate: `<path>  [recent]` (the `[recent]` tag only on recent rows).
/// Ordering is identical to the daemon route: recents (de-duplicated, recency order)
/// first, then any workspace subdir not already in the recent set.
///
/// Computed entirely from disk — no daemon required.
fn candidates_list(workspace: Option<PathBuf>) -> Result<()> {
    // Recents: de-duplicated cwds from recent sessions, in recency order.
    // Dedup delegated to eigenform_projects::unique_cwds (shared with daemon's candidates_route).
    let recents: Vec<PathBuf> = {
        let dir = projects_dir()?;
        match eigenform_forest::list(&dir, eigenform_forest::Scope::AllProjects, None, chrono::Utc::now()) {
            Ok(sessions) => {
                eigenform_projects::unique_cwds(sessions.into_iter().map(|s| s.cwd))
            }
            Err(_) => vec![],
        }
    };

    // Absolutize user-supplied --workspace so it resolves correctly regardless of CWD.
    let workspace = workspace.map(|p| {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        absolutize(&cwd, p)
    });

    // Subdirs: immediate children of the workspace root.
    // Workspace resolution: explicit --workspace, else ~/projects if it exists, else None.
    let workspace_root = workspace.or_else(|| {
        home_dir()
            .map(|h| h.join("projects"))
            .filter(|p| p.is_dir())
    });
    let subdirs: Vec<PathBuf> = match &workspace_root {
        Some(root) => eigenform_projects::immediate_subdirs(root).unwrap_or_default(),
        None => vec![],
    };

    let candidates = eigenform_projects::merge_candidates(&recents, &subdirs);
    for c in &candidates {
        if c.recent {
            println!("{}  [recent]", c.path.display());
        } else {
            println!("{}", c.path.display());
        }
    }
    Ok(())
}

/// Print the corroborated live Forest — the CLI mirror of woland's Forest surface.
fn forest_list(live_only: bool) -> Result<()> {
    let now = chrono::Utc::now();
    let rows = eigenform_forest::live_forest(&projects_dir()?, &sessions_dir()?, &state_dir()?, now);
    for r in rows {
        if live_only && !r.live {
            continue;
        }
        let badge = match r.state {
            eigenform_forest::SessionState::Ready => "● ready  ",
            eigenform_forest::SessionState::Working => "◐ working",
            eigenform_forest::SessionState::Recent => "  recent ",
        };
        let ago = ago(r.recency, now);
        let total: u32 = r.spark.iter().sum();
        let title = r.title.unwrap_or_else(|| "(untitled)".to_string());
        let title: String = title.chars().take(50).collect();
        println!(
            "{badge}  {ago:>4}  {title:<50}  {}  ~{total}tok",
            r.cwd.display()
        );
    }
    Ok(())
}

/// Coarse relative time for the CLI Forest (mirrors the browser's fmtAgo).
fn ago(then: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let s = (now - then).num_seconds().max(0);
    if s < 45 {
        "now".to_string()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}

fn require_text(render: RenderFormat) -> Result<()> {
    match render {
        RenderFormat::Text => Ok(()),
        RenderFormat::Json | RenderFormat::Html => anyhow::bail!(
            "--render {:?} is not supported yet; the json/html schema is deferred until we \
             render in the browser. Use --render text.",
            render
        ),
    }
}

fn projects_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".claude/projects"))
}

fn sessions_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".claude/sessions"))
}

fn state_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .context("could not determine home directory")?
        .join(".eigenform/state"))
}

/// Resolve a `<uuid|prefix|path>` argument to a session file path. A literal existing
/// file wins; otherwise resolve as a uuid/prefix machine-wide, printing candidates on
/// ambiguity.
fn resolve_session(session: &str) -> Result<PathBuf> {
    if PathBuf::from(session).is_file() {
        return Ok(PathBuf::from(session));
    }
    let dir = projects_dir()?;
    match eigenform_forest::resolve(&dir, session) {
        Ok(p) => Ok(p),
        Err(eigenform_forest::ResolveError::Ambiguous(candidates)) => {
            eprintln!("`{session}` is ambiguous — {} sessions match:", candidates.len());
            for c in &candidates {
                let title = eigenform_forest::session_ref(c)
                    .title
                    .unwrap_or_else(|| "(untitled)".to_string());
                eprintln!("  {}  {}", &c.uuid[..c.uuid.len().min(8)], title);
            }
            anyhow::bail!("specify more of the uuid");
        }
        Err(e) => Err(anyhow::anyhow!("{e}")),
    }
}

fn sessions_show(session: String, render: RenderFormat) -> Result<()> {
    require_text(render)?;
    let path = resolve_session(&session)?;
    let src = load_session(&path)?;
    let view = eigenform_render::session_view(&src);
    print!("{}", eigenform_render::render_text(&view));
    Ok(())
}

fn sessions_list(
    since: Option<String>,
    all_projects: bool,
    cwd: Option<PathBuf>,
    render: RenderFormat,
) -> Result<()> {
    require_text(render)?;
    let dir = projects_dir()?;

    let scope = if all_projects {
        eigenform_forest::Scope::AllProjects
    } else {
        let here = match cwd {
            Some(c) => c,
            None => env::current_dir().context("could not read current dir")?,
        };
        eigenform_forest::Scope::Project(here)
    };

    let window = parse_since(since.as_deref())?;
    let now = chrono::Utc::now();
    let sessions = eigenform_forest::list(&dir, scope, window, now).map_err(|e| anyhow::anyhow!("{e}"))?;
    print!("{}", eigenform_render::render_text(&eigenform_render::sessions_view(&sessions, now, all_projects)));
    Ok(())
}

/// Parse a recency window: `None` defaults to 7 days; `all` means no window; otherwise
/// `<n><unit>` with unit m/h/d/w.
fn parse_since(since: Option<&str>) -> Result<Option<chrono::Duration>> {
    let Some(s) = since else {
        return Ok(Some(chrono::Duration::days(7)));
    };
    if s == "all" {
        return Ok(None);
    }
    let (num, unit) = s.split_at(s.len() - 1);
    let n: i64 = num
        .parse()
        .with_context(|| format!("invalid --since `{s}` (try 7d, 24h, 30m, 2w, or all)"))?;
    let dur = match unit {
        "m" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        "w" => chrono::Duration::weeks(n),
        _ => anyhow::bail!("invalid --since unit in `{s}` (use m, h, d, w, or all)"),
    };
    Ok(Some(dur))
}

fn surgery(action: SurgeryAction) -> Result<()> {
    // `diff_src` is set when the op should print a source→fork diff to stderr.
    let (session_path, result, diff_src) = match action {
        SurgeryAction::Fork {
            session,
            at,
            edit,
            diff,
        } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let new = match edit {
                Some(edit_path) => {
                    let text = read_text(&edit_path)?;
                    eigenform_surgery::edit_then_fork(&src, &at, &text)
                }
                None => eigenform_surgery::fork_at(&src, &at),
            };
            (path, new, diff.then_some(src))
        }
        SurgeryAction::Rewind { session, to } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let new = eigenform_surgery::rewind_to(&src, &to);
            (path, new, None)
        }
        SurgeryAction::Inject {
            session,
            at,
            as_role,
            content,
        } => {
            let path = resolve_session(&session)?;
            let src = load_session(&path)?;
            let text = read_text(&content)?;
            let new = eigenform_surgery::inject(&src, &at, as_role.into(), &text);
            (path, new, None)
        }
    };

    let new = result.map_err(|e| anyhow::anyhow!("{e}"))?;
    let projects_dir = session_path
        .parent()
        .context("source session has no parent directory")?;
    let uuid = eigenform_surgery::write(&new, projects_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    println!("{uuid}");
    if let Some(src) = diff_src {
        eprint!("{}", eigenform_render::render_text(&eigenform_render::fork_diff_view(&src, &new)));
    }
    Ok(())
}

fn daemon(
    port: u16,
    cmd: Option<String>,
    web: Option<PathBuf>,
    term: Option<PathBuf>,
    workspace: Option<PathBuf>,
    dev: bool,
    open: bool,
) -> Result<()> {
    let cwd = env::current_dir().context("could not read current dir")?;

    // The pty command: explicit --cmd, else the user's shell, else bash. Not claude
    // unless the user asks for it explicitly.
    let program = cmd
        .or_else(|| env::var("SHELL").ok())
        .unwrap_or_else(|| "bash".to_string());

    // Absolutize user-supplied paths so tower-http's ServeDir doesn't double-resolve
    // them against the process CWD per request.
    let web = web.map(|p| absolutize(&cwd, p));
    let term = term.map(|p| absolutize(&cwd, p));
    let workspace = workspace.map(|p| absolutize(&cwd, p));

    // eigenform (the root app): explicit --term, else ./webterm if built. When neither
    // exists, term_dir stays None and the daemon serves the build baked into the binary
    // (feature `embed-assets`); a dev binary built without that feature serves API only.
    let term_dir = term.or_else(|| {
        let candidate = cwd.join("webterm");
        candidate.join("dist/main.js").is_file().then_some(candidate)
    });

    // woland (legacy, paused): explicit --web always; otherwise only in dev, where ./web
    // (if built) is mounted at /woland. The normal launch path never surfaces it — running
    // `eigenform` from this repo shouldn't auto-mount or advertise the paused workbench.
    let web_dir = web.or_else(|| {
        if !dev {
            return None;
        }
        let candidate = cwd.join("web");
        candidate.join("dist/main.js").is_file().then_some(candidate)
    });

    // Workspace root: explicit --workspace, else ~/projects if it exists, else None.
    let workspace_root = workspace.or_else(|| {
        home_dir()
            .map(|h| h.join("projects"))
            .filter(|p| p.is_dir())
    });

    if term_dir.is_none() && cfg!(not(feature = "embed-assets")) {
        // Only meaningful for dev binaries; an installed (embedded) binary always has the app.
        eprintln!(
            "note: no eigenform build found (run `just build`); serving the API only"
        );
    }

    let serving_embedded = term_dir.is_none();

    let config = eigenform_daemon::Config {
        program,
        args: Vec::new(),
        cwd: Some(cwd),
        web_dir,
        term_dir,
        projects_dir: Some(projects_dir()?),
        sessions_dir: Some(sessions_dir()?),
        state_dir: Some(state_dir()?),
        workspace_root,
        dev,
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let url = format!("http://{addr}");
    println!(
        "eigenform → {url}{}{}",
        if dev { "  (dev: live-reload on)" } else { "" },
        if serving_embedded { "  (embedded build)" } else { "" },
    );
    if config.web_dir.is_some() {
        println!("woland (paused) → {url}/woland");
    }

    if open {
        open_browser(&url);
    }

    let rt = tokio::runtime::Runtime::new().context("starting tokio runtime")?;
    rt.block_on(eigenform_daemon::serve(addr, config))
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Health/identity of a running daemon, from `GET /api/health`.
struct DaemonHealth {
    pid: u32,
    version: String,
}

/// Probe `/api/health`; `Some` iff an *eigenform* daemon answers on `port` (a different
/// app on the port, or nothing, yields `None`).
fn health_probe(port: u16) -> Option<DaemonHealth> {
    let url = format!("http://127.0.0.1:{port}/api/health");
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_millis(500))
        .call()
        .ok()?;
    let v: serde_json::Value = resp.into_json().ok()?;
    if v.get("app").and_then(|a| a.as_str()) != Some("eigenform") {
        return None; // something else is on this port
    }
    Some(DaemonHealth {
        pid: v.get("pid").and_then(|p| p.as_u64())? as u32,
        version: v
            .get("version")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Zero-arg `eigenform`: reuse a running daemon, else start one detached and open the app.
fn launch(port: u16) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}");
    if health_probe(port).is_some() {
        println!("eigenform → {url}  (already running) — opening browser");
        open_browser(&url);
        return Ok(());
    }

    // Nothing of ours is answering. Confirm the port is actually free — if another
    // process holds it, say so rather than spawn a daemon that can't bind.
    match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => drop(listener), // free; release it for the child to claim
        Err(_) => anyhow::bail!(
            "port {port} is in use by another process — try `eigenform daemon --port <other>`"
        ),
    }

    spawn_detached_daemon(port)?;

    // Wait for the child to bind + answer (an embedded binary boots fast; allow ~6s).
    let healthy = (0..30).find_map(|_| {
        if let Some(h) = health_probe(port) {
            return Some(h);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        None
    });
    match healthy {
        Some(h) => {
            println!("eigenform → {url}  (daemon started, pid {})", h.pid);
            open_browser(&url);
            Ok(())
        }
        None => anyhow::bail!(
            "started the daemon but it never became healthy on {url} — see {}",
            daemon_log_path()?.display()
        ),
    }
}

/// Spawn `eigenform daemon --port <port>` fully detached: its own process group, stdio
/// redirected to a log file, no controlling terminal. The parent returns at once and the
/// daemon (plus the sessions it hosts) outlives the launching shell.
fn spawn_detached_daemon(port: u16) -> Result<()> {
    let exe = std::env::current_exe().context("could not find the eigenform binary")?;
    let log_path = daemon_log_path()?;
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("could not open daemon log {}", log_path.display()))?;
    let log_err = log.try_clone().context("could not duplicate the log handle")?;

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon").arg("--port").arg(port.to_string());
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::from(log));
    cmd.stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // New process group → not a job of the launching shell, so closing the terminal
        // (SIGHUP to the shell's foreground group) doesn't reach the daemon.
        cmd.process_group(0);
    }
    cmd.spawn().context("could not start the background daemon")?;
    Ok(())
}

fn daemon_log_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("daemon.log"))
}

/// `eigenform stop` — terminate the running daemon. Its child ptys get SIGHUP as the pty
/// masters close, so the hosted claude sessions exit with it.
fn stop(port: u16) -> Result<()> {
    let Some(h) = health_probe(port) else {
        println!("eigenform → not running on port {port}");
        return Ok(());
    };
    send_term(h.pid)?;
    for _ in 0..25 {
        if health_probe(port).is_none() {
            println!("eigenform → stopped (pid {})", h.pid);
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    anyhow::bail!("sent SIGTERM to pid {} but it's still answering on port {port}", h.pid)
}

/// `eigenform status` — one line on whether a daemon is up.
fn status(port: u16) -> Result<()> {
    match health_probe(port) {
        Some(h) => println!(
            "eigenform → running on http://127.0.0.1:{port}  (pid {}, v{})",
            h.pid, h.version
        ),
        None => println!("eigenform → not running on port {port}"),
    }
    Ok(())
}

/// Send SIGTERM to `pid` via the POSIX `kill` utility (avoids a libc dependency;
/// eigenform targets unix — macOS / Linux / WSL).
fn send_term(pid: u32) -> Result<()> {
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .context("could not invoke `kill`")?;
    if !status.success() {
        anyhow::bail!("`kill -TERM {pid}` failed");
    }
    Ok(())
}

/// Best-effort: open `url` in the user's browser. The URL is always printed, so a
/// failure here is silent — common on headless hosts and bare WSL without wslu.
fn open_browser(url: &str) {
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("open", &[])]
    } else if cfg!(target_os = "windows") {
        &[("cmd", &["/c", "start", ""])]
    } else {
        // Linux/WSL: wslview (wslu) hands off to the Windows browser; xdg-open otherwise.
        &[("wslview", &[]), ("xdg-open", &[])]
    };
    for (bin, prefix) in candidates {
        let mut c = std::process::Command::new(bin);
        c.args(prefix.iter().copied()).arg(url);
        c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if c.spawn().is_ok() {
            return;
        }
    }
}

fn sessions_diff(a: String, b: String, render: RenderFormat) -> Result<()> {
    require_text(render)?;
    let source = load_session(&resolve_session(&a)?)?;
    let fork = load_session(&resolve_session(&b)?)?;
    print!("{}", eigenform_render::render_text(&eigenform_render::fork_diff_view(&source, &fork)));
    Ok(())
}

fn load_session(path: &PathBuf) -> Result<eigenform_surgery::Session> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading session {path:?}"))?;
    eigenform_surgery::Session::parse_str(&contents).map_err(|e| anyhow::anyhow!("{e}"))
}

fn read_text(path: &PathBuf) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))
}

fn skills_tree(cwd_override: Option<PathBuf>) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    let cwd = match cwd_override {
        Some(p) => p,
        None => env::current_dir().context("could not read current dir")?,
    };

    let roots = eigenform_skills::canonical_roots(&home, &cwd);
    let found = eigenform_skills::scan_many(&roots)
        .with_context(|| format!("scanning skills under home={:?} cwd={:?}", home, cwd))?;

    print!("{}", eigenform_skills::render_tree(&found));
    Ok(())
}

/// `eigenform inspect` — the unified config inventory. Skills + memory across
/// resolution layers, token-budgeted, projected through the render crate's View IR
/// to text or json (html is deferred until the browser consumes it).
fn inspect_cmd(cwd_override: Option<PathBuf>, all_projects: bool, render: RenderFormat) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    let data = if all_projects {
        eigenform_inspect::collect_all_projects(&home).map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        let cwd = match cwd_override {
            Some(p) => p,
            None => env::current_dir().context("could not read current dir")?,
        };
        eigenform_inspect::collect(&home, &cwd).map_err(|e| anyhow::anyhow!("{e}"))?
    };
    match render {
        RenderFormat::Text => {
            print!("{}", eigenform_render::render_text(&eigenform_render::inspect_view(&data)))
        }
        RenderFormat::Json => println!("{}", eigenform_render::inspect_json(&data)),
        RenderFormat::Html => anyhow::bail!(
            "--render html for inspect is deferred until the browser consumes it; use text or json"
        ),
    }
    Ok(())
}

fn skills_list(all_projects: bool) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;

    // The single-project view is the common case: default to the current context
    // (mirrors `skills tree`) rather than rejecting the no-flag invocation.
    if !all_projects {
        return skills_tree(None);
    }

    let projects_dir = home.join(".claude/projects");
    let projects = eigenform_projects::enumerate_projects(&projects_dir)
        .with_context(|| format!("enumerating projects in {:?}", projects_dir))?;

    let cwds: Vec<PathBuf> = projects.iter().map(|p| p.cwd.clone()).collect();
    let roots = eigenform_skills::all_projects_roots(&home, &cwds);
    let found = eigenform_skills::scan_many(&roots)
        .with_context(|| "scanning skills across all projects")?;

    println!("# projects discovered: {}", projects.len());
    for p in &projects {
        println!("#   {} -> {}", p.dir_name, p.cwd.display());
    }
    println!();
    print!("{}", eigenform_skills::render_tree(&found));
    Ok(())
}

fn memory_tree(cwd_override: Option<PathBuf>) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    let cwd = match cwd_override {
        Some(p) => p,
        None => env::current_dir().context("could not read current dir")?,
    };
    let projects_dir = home.join(".claude/projects");

    let project = eigenform_projects::project_for_cwd(&projects_dir, &cwd)
        .with_context(|| format!("looking up project for cwd {:?}", cwd))?;
    let Some(project) = project else {
        println!("MEMORY");
        println!("======");
        println!();
        println!("(no memory: no Claude Code project recorded for {:?})", cwd);
        return Ok(());
    };

    let memory_dir = projects_dir.join(&project.dir_name).join("memory");
    let entries = eigenform_memory::scan_memory_dir(&memory_dir)
        .with_context(|| format!("scanning memory in {:?}", memory_dir))?;

    println!("# project: {} -> {}", project.dir_name, project.cwd.display());
    println!("# memory dir: {}", memory_dir.display());
    println!();
    print!("{}", eigenform_memory::render_memory_tree(&entries));
    Ok(())
}

fn memory_list(all_projects: bool) -> Result<()> {
    let home = home_dir().context("could not determine home directory")?;
    // Default to the current project's memory rather than rejecting the no-flag case.
    if !all_projects {
        return memory_tree(None);
    }
    let projects_dir = home.join(".claude/projects");
    let projects = eigenform_projects::enumerate_projects(&projects_dir)
        .with_context(|| format!("enumerating projects in {:?}", projects_dir))?;

    println!("# projects discovered: {}", projects.len());
    println!();

    for p in &projects {
        let memory_dir = projects_dir.join(&p.dir_name).join("memory");
        let entries = eigenform_memory::scan_memory_dir(&memory_dir)
            .with_context(|| format!("scanning {:?}", memory_dir))?;

        println!("================================================================");
        println!("project: {} -> {}", p.dir_name, p.cwd.display());
        println!("================================================================");
        print!("{}", eigenform_memory::render_memory_tree(&entries));
        println!();
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(PathBuf::from)
}

/// Return `p` unchanged if it is already absolute; otherwise join it onto `cwd`.
fn absolutize(cwd: &std::path::Path, p: PathBuf) -> PathBuf {
    if p.is_absolute() { p } else { cwd.join(p) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // ── absolutize ──────────────────────────────────────────────────────────

    #[test]
    fn absolutize_relative_is_joined_to_cwd() {
        let cwd = PathBuf::from("/home/user/myproject");
        let rel = PathBuf::from("webterm");
        assert_eq!(absolutize(&cwd, rel), PathBuf::from("/home/user/myproject/webterm"));
    }

    #[test]
    fn absolutize_absolute_is_unchanged() {
        let cwd = PathBuf::from("/home/user/myproject");
        let abs = PathBuf::from("/opt/assets/webterm");
        assert_eq!(absolutize(&cwd, abs.clone()), abs);
    }

    #[test]
    fn absolutize_dot_relative_is_joined() {
        let cwd = PathBuf::from("/home/user/myproject");
        let dot = PathBuf::from("./webterm");
        assert_eq!(absolutize(&cwd, dot), PathBuf::from("/home/user/myproject/webterm"));
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(y, mo, d, h, m, s).unwrap()
    }

    #[test]
    fn ago_under_45s_is_now() {
        let now = utc(2026, 6, 11, 12, 0, 0);
        assert_eq!(ago(utc(2026, 6, 11, 11, 59, 30), now), "now");
        assert_eq!(ago(utc(2026, 6, 11, 12, 0, 0), now), "now"); // zero delta
    }

    #[test]
    fn ago_minutes() {
        let now = utc(2026, 6, 11, 12, 0, 0);
        assert_eq!(ago(utc(2026, 6, 11, 11, 57, 0), now), "3m");
    }

    #[test]
    fn ago_hours() {
        let now = utc(2026, 6, 11, 12, 0, 0);
        assert_eq!(ago(utc(2026, 6, 11, 9, 30, 0), now), "2h");
    }

    #[test]
    fn ago_days() {
        let now = utc(2026, 6, 11, 12, 0, 0);
        assert_eq!(ago(utc(2026, 6, 9, 12, 0, 0), now), "2d");
    }
}
