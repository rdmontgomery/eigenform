// xterm 5.x does NOT inject its stylesheet — without this import the viewport/
// screen/rows render in normal flow and the terminal overflows the page.
import "@xterm/xterm/css/xterm.css";
import "./style.css";
import { mountShell } from "./shell.ts";

const app = document.getElementById("app");
if (app) mountShell(app);
