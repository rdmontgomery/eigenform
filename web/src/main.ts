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

// (Re)connect the center pty. query = "" (default shell), "?session=<uuid>" (resume), or
// "?new=<cwd>" (fresh claude). Only a real connection spawns anything.
function connectPty(query = "") {
  if (ws) { ws.onclose = null; ws.close(); }
  if (onData) { onData.dispose(); onData = null; }
  term.reset();

  const proto = location.protocol === "https:" ? "wss" : "ws";
  const sock = new WebSocket(`${proto}://${location.host}/pty${query}`);
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
    if (ev.data instanceof ArrayBuffer) {
      term.write(new Uint8Array(ev.data));
      return;
    }
    // Text frame = a control message from the daemon (e.g. a new session's uuid).
    try {
      const msg = JSON.parse(ev.data as string);
      if (msg.type === "session" && typeof msg.uuid === "string") onSessionBorn(msg.uuid);
    } catch {
      /* ignore non-JSON */
    }
  };
  sock.onclose = () => term.write("\r\n\x1b[2m[woland: pty disconnected]\x1b[0m\r\n");
}

const manuscript = () => document.getElementById("manuscript") as HTMLElement;

// Render the Manuscript in-page (not an iframe) so it can become a writing surface.
// Re-fetch the fragment on each SSE 'change'; stay pinned to the leaf if we were near it.
async function renderManuscript(uuid: string) {
  const m = manuscript();
  const nearBottom = m.scrollHeight - m.scrollTop - m.clientHeight < 80;
  const prev = m.scrollTop;
  try {
    m.innerHTML = await (await fetch(`/api/session/${encodeURIComponent(uuid)}`)).text();
    m.scrollTop = nearBottom ? m.scrollHeight : prev;
  } catch {
    m.innerHTML = `<div class="placeholder">could not load session</div>`;
  }
}

function followManuscript(uuid: string) {
  void renderManuscript(uuid);
  if (es) es.close();
  es = new EventSource(`/api/watch/${encodeURIComponent(uuid)}`);
  es.onmessage = () => void renderManuscript(uuid);
}

// Send a prompt to the live pty. claude's TUI only submits on a discrete Enter, separate
// from the input text. Bracketed paste (ESC[200~ … ESC[201~) frames the text as a paste —
// so claude inserts it literally (multi-line and all), and the trailing \r after the
// close marker is an unambiguous Enter. The marker provides the separation, so this is one
// atomic write with no timing hack.
function sendPrompt(text: string): boolean {
  if (!ws || ws.readyState !== WebSocket.OPEN) return false;
  const sock = ws;
  // Bracketed paste so the (possibly multi-line) text inserts literally with no escape
  // leak. The Enter must arrive as a SEPARATE read for claude's TUI to treat it as a
  // discrete keypress and submit — same reason `tmux send-keys` sends keys separately.
  sock.send(JSON.stringify({ type: "stdin", data: `\x1b[200~${text}\x1b[201~` }));
  setTimeout(() => {
    if (sock.readyState === WebSocket.OPEN) {
      sock.send(JSON.stringify({ type: "stdin", data: "\r" }));
    }
  }, 60);
  return true;
}

// The leaf input: type into the Manuscript, pipe to the live session's pty (claude).
function setupLeafInput() {
  const input = document.getElementById("leaf-input") as HTMLTextAreaElement;
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (input.value && sendPrompt(input.value)) {
        input.value = "";
      }
    }
  });
}

function enableLeafInput(label: string) {
  const input = document.getElementById("leaf-input") as HTMLTextAreaElement;
  input.disabled = false;
  input.placeholder = `write to ${label} — Enter to send, Shift+Enter for newline`;
  input.focus();
}

// The "+ new session" control: a directory input with a datalist of known project cwds.
async function loadProjectDirs() {
  const datalist = document.getElementById("project-dirs")!;
  try {
    const dirs: string[] = await (await fetch("/api/projects")).json();
    datalist.replaceChildren();
    for (const d of dirs) {
      const opt = document.createElement("option");
      opt.value = d;
      datalist.append(opt);
    }
  } catch {
    /* none */
  }
}

function setupNewSession() {
  const btn = document.getElementById("new-session-btn") as HTMLButtonElement;
  const form = document.getElementById("new-session-form") as HTMLFormElement;
  const input = document.getElementById("new-session-dir") as HTMLInputElement;
  btn.addEventListener("click", () => {
    form.hidden = !form.hidden;
    if (!form.hidden) input.focus();
  });
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const cwd = input.value.trim();
    if (cwd) {
      startNewSession(cwd);
      form.hidden = true;
      input.value = "";
    }
  });
  void loadProjectDirs();
}

let activeSession: string | null = null;

function highlightSession(uuid: string | null) {
  document.querySelectorAll<HTMLElement>(".session-item").forEach((el) => {
    el.classList.toggle("active", el.dataset.uuid === uuid);
  });
}

function selectSession(uuid: string) {
  activeSession = uuid;
  connectPty(`?session=${encodeURIComponent(uuid)}`);
  followManuscript(uuid);
  enableLeafInput(uuid.slice(0, 8));
  highlightSession(uuid);
}

// Start a fresh claude session in `cwd`. The uuid doesn't exist yet; the daemon detects
// the new JSONL and reports it via onSessionBorn, which then follows it.
function startNewSession(cwd: string) {
  activeSession = null;
  connectPty(`?new=${encodeURIComponent(cwd)}`);
  enableLeafInput("new session");
  highlightSession(null);
  const m = manuscript();
  const note = document.createElement("div");
  note.className = "placeholder";
  note.textContent = `starting a new session in ${cwd} …`;
  m.replaceChildren(note);
}

// The daemon found the new session's JSONL — bind everything to it.
function onSessionBorn(uuid: string) {
  activeSession = uuid;
  followManuscript(uuid);
  enableLeafInput(uuid.slice(0, 8));
  void loadSidebar().then(() => highlightSession(uuid));
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
setupLeafInput();
setupNewSession();
devLiveReload();
fetch("/api/recent")
  .then((r) => (r.ok ? r.text() : ""))
  .then((u) => {
    const uuid = u.trim();
    if (uuid) followManuscript(uuid); // read-only preview until a session is selected
  })
  .catch(() => {});
