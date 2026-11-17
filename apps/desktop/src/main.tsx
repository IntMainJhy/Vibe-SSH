import { createRoot } from "react-dom/client";
import { report } from "./api";
import { App } from "./App";
import "./styles.css";
import "./workbench.css";

async function start() {
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window))
    await import("./dev-mock");
  addEventListener("error", (e) =>
    report(`error: ${e.message} at ${e.filename}:${e.lineno}`),
  );
  addEventListener("unhandledrejection", (e) =>
    report(`rejection: ${e.reason?.message ?? e.reason}`),
  );
  addEventListener("securitypolicyviolation", (e) =>
    report(`blocked: ${e.violatedDirective} ${e.blockedURI}`),
  );
  createRoot(document.getElementById("root")!).render(<App />);
}

void start();
