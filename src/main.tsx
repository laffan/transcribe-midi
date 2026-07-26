import React from "react";
import ReactDOM from "react-dom/client";

import { App } from "./App";
import { logger } from "./lib/console";
import { applyTheme, loadTheme } from "./lib/theme";
import "./styles/global.css";

// Apply the stored theme before first paint so there is no light-to-dark flash.
applyTheme(loadTheme());

// Anything that escapes a component ends up in the app console rather than only in the
// webview's devtools, which are awkward to reach on iOS.
window.addEventListener("error", (event) => {
  logger.error("Unhandled error", event.message);
});
window.addEventListener("unhandledrejection", (event) => {
  logger.error("Unhandled promise rejection", String(event.reason));
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
