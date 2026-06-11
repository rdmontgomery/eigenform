import "./style.css";
import { mountShell } from "./shell.ts";

const app = document.getElementById("app");
if (app) mountShell(app);
