import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/fonts";
import "./styles/tokens.css";
import "./styles/base.css";
import { App } from "./App";
import { useApp } from "./state/store";
import { resolveTheme } from "./state/theme";

// Apply the saved theme before the first paint (no light/dark flash on start-up).
document.documentElement.dataset.theme = resolveTheme(
  useApp.getState().themePref,
  matchMedia("(prefers-color-scheme: dark)").matches,
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
