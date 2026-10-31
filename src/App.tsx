import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import { ContextMenu } from "./components/ContextMenu";
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

const clampSplit = (r: number) => Math.min(0.8, Math.max(0.2, r));

export default function App() {
  useKeyboard();
  const panesRef = useRef<HTMLElement>(null);
  const splitRatio = useSettings((s) => s.config?.split_ratio ?? 0.5);

  // Drag writes the CSS var directly (no re-render per mousemove); the
  // ratio is persisted once on mouseup.
  const onSplitDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const el = panesRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    let moved = false;
    const ratioAt = (x: number) => clampSplit((x - rect.left) / rect.width);
    const move = (ev: MouseEvent) => {
      moved = true;
      el.style.setProperty("--split", String(ratioAt(ev.clientX)));
    };
    const up = (ev: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      if (moved) useSettings.getState().save({ split_ratio: ratioAt(ev.clientX) });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const onSplitReset = () => {
    panesRef.current?.style.setProperty("--split", "0.5");
    useSettings.getState().save({ split_ratio: 0.5 });
  };

  // Suppress the webview's browser context menu (Reload/Inspect…); text
  // inputs keep theirs for paste and spelling.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (target?.closest("input, textarea, [contenteditable]")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onContextMenu);
    return () => window.removeEventListener("contextmenu", onContextMenu);
  }, []);

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
      <main
        className="panes"
        ref={panesRef}
        style={{ "--split": splitRatio } as React.CSSProperties}
      >
        <Panel side={0} />
        <div className="split" onMouseDown={onSplitDown} onDoubleClick={onSplitReset} />
        <Panel side={1} />
      </main>
      <OpsUI />
      <FKeyBar />
      <Dialogs />
      <ContextMenu />
    </div>
  );
}
