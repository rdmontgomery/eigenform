// woland: three panes — the forest (pick), the live pty (resume), the re-render (follow).
// Selecting a session resumes it via `claude --resume` (your token-spending click), and
// points the right pane at the same session, following it live over SSE.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

const term = new Terminal({
  fontFamily: "ui-monospace, Menlo, Consolas, monospace",
  fontSize: 13,
  cursorBlink: true,
  theme: { background: "#0b0b0e", foreground: "#e6e6e6" },
});
const fit = new FitAddon();
term.loadAddon(fit);
term.open(document.getElementById("terminal")!);
fit.fit();

let ws: WebSocket | null = null;
let onData: { dispose(): void } | null = null;
let es: EventSource | null = null;

function sendResize() {
  fit.fit();
  if (ws && ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
  }
}
window.addEventListener("resize", sendResize);

// (Re)connect the center pty. No session = the daemon's default shell (no tokens);
// a session = `claude --resume <uuid>`.
function connectPty(session?: string) {
  if (ws) { ws.onclose = null; ws.close(); }
  if (onData) { onData.dispose(); onData = null; }
  term.reset();

  const proto = location.protocol === "https:" ? "wss" : "ws";
  const q = session ? `?session=${encodeURIComponent(session)}` : "";
  const sock = new WebSocket(`${proto}://${location.host}/pty${q}`);
  sock.binaryType = "arraybuffer";
  ws = sock;

  sock.onopen = () => {
    sendResize();
    onData = term.onData((d) => {
      if (sock.readyState === WebSocket.OPEN) {
        sock.send(JSON.stringify({ type: "stdin", data: d }));
      }
    });
    term.focus();
  };
  sock.onmessage = (ev) => {
    if (ev.data instanceof ArrayBuffer) term.write(new Uint8Array(ev.data));
    else term.write(ev.data as string);
  };
  sock.onclose = () => term.write("\r\n\x1b[2m[woland: pty disconnected]\x1b[0m\r\n");
}

const frame = () => document.getElementById("transcript") as HTMLIFrameElement;

// Point the right pane at a session and follow it live: each SSE 'change' reloads the
// transcript, restoring scroll so a growing session doesn't jump under you.
function followTranscript(uuid: string) {
  frame().src = `/session/${encodeURIComponent(uuid)}`;
  if (es) es.close();
  es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => {
    const f = frame();
    const y = f.contentWindow ? f.contentWindow.scrollY : 0;
    f.onload = () => {
      f.contentWindow?.scrollTo(0, y);
      f.onload = null;
    };
    f.src = `/session/${encodeURIComponent(uuid)}`;
  };
}

let activeSession: string | null = null;

function selectSession(uuid: string) {
  activeSession = uuid;
  connectPty(uuid);
  followTranscript(uuid);
  document.querySelectorAll<HTMLElement>(".session-item").forEach((el) => {
    el.classList.toggle("active", el.dataset.uuid === uuid);
  });
}

// Dev live-reload: when the daemon injects the dev meta, listen for bundle changes and
// refresh — UNLESS a session is live (a full reload would drop the claude pty and respawn
// it on the next click). Never auto-respawns claude.
function devLiveReload() {
  if (!document.querySelector('meta[name="eigen-dev"]')) return;
  const es = new EventSource("/api/dev/reload");
  let last = 0;
  es.onmessage = () => {
    const now = Date.now();
    if (now - last < 300) return; // debounce esbuild's multi-file writes
    last = now;
    if (activeSession) {
      console.warn("[woland dev] frontend changed — refresh manually to keep the live session");
      return;
    }
    location.reload();
  };
}

interface SessionItem {
  uuid: string;
  title: string;
  cwd: string;
  recency: string;
}

async function loadSidebar() {
  const list = document.getElementById("sessions")!;
  try {
    const items: SessionItem[] = await (await fetch("/api/sessions")).json();
    list.replaceChildren();
    for (const it of items) {
      const btn = document.createElement("button");
      btn.className = "session-item";
      btn.dataset.uuid = it.uuid;
      const project = it.cwd.split("/").filter(Boolean).pop() ?? it.cwd;
      const title = document.createElement("span");
      title.className = "title";
      title.textContent = it.title;
      const meta = document.createElement("span");
      meta.className = "meta";
      meta.textContent = `${project} · ${it.uuid.slice(0, 8)}`;
      btn.append(title, meta);
      btn.onclick = () => selectSession(it.uuid);
      list.append(btn);
    }
  } catch {
    list.textContent = "no sessions";
  }
}

// Startup: shell in the center (no tokens), the most recent session in the right pane,
// the forest in the sidebar. Pick a session to resume it.
connectPty();
loadSidebar();
devLiveReload();
fetch("/api/recent")
  .then((r) => (r.ok ? r.text() : ""))
  .then((u) => {
    const uuid = u.trim();
    if (uuid) followTranscript(uuid);
  })
  .catch(() => {});
