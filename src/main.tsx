import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initI18n } from "./i18n";
import { getConfig } from "./ipc/commands";
import { applyPlatformDataset } from "./lib/platform";
import { setBootConfig } from "./store/settings";
import "./styles/base.css";

applyPlatformDataset();
getConfig()
  .catch(() => null)
  .then((config) => {
    setBootConfig(config);
    initI18n(config?.language ?? null);
    ReactDOM.createRoot(document.getElementById("root")!).render(
      <React.StrictMode>
        <App />
      </React.StrictMode>,
    );
  });
