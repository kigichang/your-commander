import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { Dialogs } from "./components/Dialogs";
import { FKeyBar } from "./components/FKeyBar";
import { OpsUI } from "./components/OpsUI";
import { Panel } from "./components/Panel";
import { Toolbar } from "./components/Toolbar";
import { useKeyboard } from "./keymap/useKeyboard";
import { useKeymap } from "./store/keymapStore";
import type { Side } from "./store/panels";
import { usePanels } from "./store/panels";
import { useSettings } from "./store/settings";

export default function App() {
  useKeyboard();

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void (async () => {
      await useSettings.getState().load();
      useKeymap.getState().load();
      await usePanels.getState().init();
      // Directory watcher pings: watchId is the panel side.
      unlisten = await listen<number>("fs:changed", (e) => {
        void usePanels.getState().refresh(e.payload as Side, { silent: true });
      });
    })();
    return () => unlisten?.();
  }, []);

  return (
    <div className="app">
      <header className="titlebar" data-tauri-drag-region>
        <Toolbar />
      </header>
      <main className="panes">
        <Panel side={0} />
        <Panel side={1} />
      </main>
      <OpsUI />
      <FKeyBar />
      <Dialogs />
    </div>
  );
}
