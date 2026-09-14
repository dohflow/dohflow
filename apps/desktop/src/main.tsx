import React from "react";
import ReactDOM from "react-dom/client";

// Self-hosted variable fonts (no CDN — honors the no-network / Tauri-CSP
// non-negotiable) and the design-system tokens (personal-cfo-x99h).
// ADR 0063: IBM Plex Sans is the default face and holds all numerals; Nunito is
// opt-in display. Both ship with the app — it is offline-first and must never
// depend on a font CDN.
import "@fontsource-variable/ibm-plex-sans/index.css";
import "@fontsource-variable/nunito/index.css";
import "./styles/globals.css";

import App from "./App";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Root element #root not found");
}

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
