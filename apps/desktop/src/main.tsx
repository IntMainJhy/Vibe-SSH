import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

async function start() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) await import("./dev-mock");
  createRoot(document.getElementById("root")!).render(<App />);
}

void start();
