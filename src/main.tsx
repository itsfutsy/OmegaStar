import React from "react";
import ReactDOM from "react-dom/client";
// The opsz file carries both the weight and the optical-size axes, so titles can use Inter Display (--display).
import "@fontsource-variable/inter/opsz.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "./design/tokens.css";
import "./design/base.css";
import { applyChrome } from "./lib/chrome";
import { applySideWidth } from "./lib/sidebar";
import { applyTheme } from "./lib/theme";
import App from "./App";

applyTheme();
applyChrome();
applySideWidth();

// A file dropped anywhere but the chat must not navigate the window to it.
for (const type of ["dragover", "drop"] as const) {
  window.addEventListener(type, (e) => { if (e.dataTransfer?.types.includes("Files")) e.preventDefault(); });
}

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);
// `npm run dev` with ?preview=parts: the shared parts on one page (src/preview/Parts.tsx). Vite drops this from builds.
if (import.meta.env.DEV && new URLSearchParams(location.search).get("preview") === "parts") {
  void import("./preview/Parts").then(({ Parts }) => root.render(<React.StrictMode><Parts /></React.StrictMode>));
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}
