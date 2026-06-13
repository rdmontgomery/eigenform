// xterm 5.x does NOT inject its stylesheet — without this import the viewport/
// screen/rows render in normal flow and the terminal overflows the page.
import "@xterm/xterm/css/xterm.css";
import "./style.css";
import { mountShell } from "./shell.ts";

const app = document.getElementById("app");
if (app) mountShell(app);

// Dev live-reload: when the daemon (--dev) signals a frontend rebuild, reload.
// Tabs + ptys survive a reload (ptys live in the daemon; tabs reconcile on boot),
// so a full reload is safe here. Gated on the dev meta the daemon injects.
if (document.querySelector('meta[name="eigenform-dev"]')) {
  const ev = new EventSource("/api/dev/reload");
  let last = 0;
  ev.onmessage = () => {
    const now = Date.now();
    if (now - last < 300) return; // debounce esbuild's multi-file writes
    last = now;
    location.reload();
  };
}
