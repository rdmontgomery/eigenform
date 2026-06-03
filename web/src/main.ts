// woland slice 1: a faithful pty rendered in the browser via xterm.js over a websocket.
// This pane is ground truth; the semantic re-render (slice 2) will sit beside it.

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

const proto = location.protocol === "https:" ? "wss" : "ws";
const ws = new WebSocket(`${proto}://${location.host}/pty`);
ws.binaryType = "arraybuffer";

function sendResize() {
  fit.fit();
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
  }
}

ws.onopen = () => {
  sendResize();
  term.onData((data) => ws.send(JSON.stringify({ type: "stdin", data })));
  term.focus();
};

ws.onmessage = (ev) => {
  if (ev.data instanceof ArrayBuffer) {
    term.write(new Uint8Array(ev.data));
  } else {
    term.write(ev.data as string);
  }
};

ws.onclose = () => term.write("\r\n\x1b[2m[woland: pty disconnected]\x1b[0m\r\n");

window.addEventListener("resize", sendResize);

// Right pane: the semantic re-render. Show the requested session, else the most recent.
async function loadTranscript() {
  const frame = document.getElementById("transcript") as HTMLIFrameElement;
  const params = new URLSearchParams(location.search);
  let uuid = params.get("session");
  if (!uuid) {
    try {
      const res = await fetch("/api/recent");
      if (res.ok) uuid = (await res.text()).trim();
    } catch {
      /* no daemon transcript available */
    }
  }
  if (uuid) frame.src = `/session/${encodeURIComponent(uuid)}`;
}
loadTranscript();
