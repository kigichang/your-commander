import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { ConflictChoice, IfExists } from "../ipc/types";
import { baseName, joinPath } from "../lib/path";
import type { Dialog } from "../store/dialogs";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { ConnectDialog } from "./ConnectDialog";
import { MultiRenameDialog } from "./MultiRenameDialog";
import { FindDialog } from "./FindDialog";
import { SyncDialog } from "./SyncDialog";

export function Dialogs() {
  const dialog = useDialogs((s) => s.dialog);
  const conflict = useOps((s) => s.conflict);
  // A pending conflict outranks whatever else is open.
  if (conflict) return <ConflictDialog />;
  if (!dialog) return null;
  switch (dialog.type) {
    case "connect":
      return <ConnectDialog protocol={dialog.protocol} />;
    case "copyMove":
      return <CopyMoveDialog d={dialog} />;
    case "newFolder":
      return <NewFolderDialog d={dialog} />;
    case "rename":
      return <RenameDialog d={dialog} />;
    case "multiRename":
      return <MultiRenameDialog d={dialog} />;
    case "sync":
      return <SyncDialog d={dialog} />;
    case "find":
      return <FindDialog d={dialog} />;
    case "confirmDelete":
      return <ConfirmDeleteDialog d={dialog} />;
    case "compress":
      return <CompressDialog d={dialog} />;
  }
}

function Overlay({
  children,
  onClose,
}: {
  children: React.ReactNode;
  onClose: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);
  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog">{children}</div>
    </div>
  );
}

function ItemList({ names }: { names: string[] }) {
  const { t } = useTranslation();
  const shown = names.slice(0, 6);
  return (
    <div className="list">
      {shown.map((n) => (
        <div key={n}>{n}</div>
      ))}
      {names.length > shown.length && (
        <div>{t("dialog.andMore", { count: names.length - shown.length })}</div>
      )}
    </div>
  );
}

const IF_EXISTS: Array<{ value: IfExists; label: string }> = [
  { value: "ask", label: "dialog.ifAsk" },
  { value: "skip", label: "dialog.ifSkip" },
  { value: "replaceOlder", label: "dialog.ifReplaceOlder" },
  { value: "rename", label: "dialog.ifRename" },
];

function CopyMoveDialog({ d }: { d: Extract<Dialog, { type: "copyMove" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [dest, setDest] = useState(d.destDir);
  const [verify, setVerify] = useState(false);
  const [keepMeta, setKeepMeta] = useState(true);
  const [mask, setMask] = useState("*.*");
  const [followSymlinks, setFollowSymlinks] = useState(false);
  const [ifExists, setIfExists] = useState<IfExists>("ask");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.select(), []);

  const go = (queued: boolean) => {
    close();
    const s = usePanels.getState();
    const srcTab = s.activeTab(d.side);
    const destTab = s.activeTab(s.otherSide(d.side));
    const sameDest = dest === destTab.path;
    const label = t(d.move ? "dialog.move" : "dialog.copy");
    const request = {
      op: "copy" as const,
      src: srcTab.location,
      sources: d.items.map((e) => ({ path: e.path, isDir: e.isDir })),
      // Typing a different path only makes sense toward the other panel's
      // location; the default is exactly that panel's folder.
      dest: sameDest ? destTab.location : ({ kind: "local" } as const),
      destDir: dest,
      removeSource: d.move,
      options: {
        verify,
        keepMeta,
        mask: mask.trim() && mask.trim() !== "*.*" ? mask.trim() : null,
        followSymlinks,
        ifExists,
      },
    };
    const onDone = (done: { count: number }) =>
      useOps
        .getState()
        .toast(t(d.move ? "status.moved" : "status.copied", { count: done.count }));
    if (queued) useOps.getState().enqueue(label, request, onDone);
    else useOps.getState().run(label, request, onDone);
  };

  return (
    <Overlay onClose={close}>
      <h2>{t(d.move ? "dialog.moveTitle" : "dialog.copyTitle", { count: d.items.length })}</h2>
      <div className="body">
        <ItemList names={d.items.map((e) => e.name)} />
        <label>{t("dialog.to")}</label>
        <input
          ref={inputRef}
          value={dest}
          spellCheck={false}
          onChange={(e) => setDest(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go(false)}
        />
        <div className="copy-opts">
          <label className="check">
            <input type="checkbox" checked={verify} onChange={(e) => setVerify(e.target.checked)} />
            {t("dialog.verifyAfter")}
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={keepMeta}
              onChange={(e) => setKeepMeta(e.target.checked)}
            />
            {t("dialog.keepMeta")}
          </label>
          <label className="check mask">
            <span>{t("dialog.onlyMatching")}</span>
            <input
              value={mask}
              spellCheck={false}
              onChange={(e) => setMask(e.target.value)}
            />
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={followSymlinks}
              onChange={(e) => setFollowSymlinks(e.target.checked)}
            />
            {t("dialog.followSymlinks")}
          </label>
        </div>
        <label>{t("dialog.ifExists")}</label>
        <div className="seg">
          {IF_EXISTS.map((o) => (
            <button
              key={o.value}
              className={ifExists === o.value ? "on" : ""}
              onClick={() => setIfExists(o.value)}
            >
              {t(o.label)}
            </button>
          ))}
        </div>
      </div>
      <div className="buttons">
        <button className="btn" onClick={() => go(true)}>
          {t("dialog.addToQueue")}
        </button>
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        <button className="btn primary" onClick={() => go(false)}>
          {t(d.move ? "dialog.move" : "dialog.copy")} ↵
        </button>
      </div>
    </Overlay>
  );
}

function NewFolderDialog({ d }: { d: Extract<Dialog, { type: "newFolder" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [name, setName] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);

  const go = () => {
    if (!name.trim()) return;
    close();
    const s = usePanels.getState();
    const tab = s.activeTab(d.side);
    ipc
      .mkdir(tab.location, tab.path, name.trim())
      .then(() => {
        useOps.getState().toast(t("status.folderCreated"));
        void s.refresh(d.side, { silent: true });
      })
      .catch((e) => useOps.getState().toast(String(e), "error"));
  };

  return (
    <Overlay onClose={close}>
      <h2>{t("dialog.newFolderTitle")}</h2>
      <div className="body">
        <input
          ref={inputRef}
          value={name}
          spellCheck={false}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go()}
        />
      </div>
      <div className="buttons">
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        <button className="btn primary" onClick={go}>
          {t("dialog.create")} ↵
        </button>
      </div>
    </Overlay>
  );
}

function RenameDialog({ d }: { d: Extract<Dialog, { type: "rename" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [name, setName] = useState(d.entry.name);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    input.focus();
    const dot = d.entry.name.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : d.entry.name.length);
  }, [d.entry.name]);

  const go = () => {
    if (!name.trim() || name === d.entry.name) return close();
    close();
    const s = usePanels.getState();
    const tab = s.activeTab(d.side);
    ipc
      .rename(tab.location, d.entry.path, name.trim())
      .then(() => {
        useOps.getState().toast(t("status.renamed"));
        void s.refresh(d.side, { silent: true });
      })
      .catch((e) => useOps.getState().toast(String(e), "error"));
  };

  return (
    <Overlay onClose={close}>
      <h2>{t("dialog.renameTitle")}</h2>
      <div className="body">
        <input
          ref={inputRef}
          value={name}
          spellCheck={false}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go()}
        />
      </div>
      <div className="buttons">
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        <button className="btn primary" onClick={go}>
          {t("dialog.rename")} ↵
        </button>
      </div>
    </Overlay>
  );
}

function ConfirmDeleteDialog({ d }: { d: Extract<Dialog, { type: "confirmDelete" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);

  const run = (permanent: boolean) => {
    close();
    const s = usePanels.getState();
    const tab = s.activeTab(d.side);
    const items = d.items.map((e) => e.path);
    if (permanent || d.remote) {
      void useOps.getState().run(
        t("dialog.deletePermanent"),
        { op: "delete", location: tab.location, items },
        (done) => useOps.getState().toast(t("status.deleted", { count: done.count })),
      );
    } else {
      void useOps.getState().run(
        t("fkeys.f8"),
        { op: "trash", items },
        (done) => useOps.getState().toast(t("status.trashed", { count: done.count })),
      );
    }
  };

  return (
    <Overlay onClose={close}>
      <h2>{t("dialog.deleteTitle", { count: d.items.length })}</h2>
      <div className="body">
        <ItemList names={d.items.map((e) => e.name)} />
        {d.remote && <p>{t("dialog.remoteNoTrash")}</p>}
      </div>
      <div className="buttons">
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        {!d.remote && (
          <button className="btn" onClick={() => run(false)}>
            {t("dialog.toTrash")}
          </button>
        )}
        <button className="btn danger" onClick={() => run(true)}>
          {t("dialog.deletePermanent")}
        </button>
      </div>
    </Overlay>
  );
}

const FORMATS = ["zip", "tar", "tar.gz", "gz"] as const;

function CompressDialog({ d }: { d: Extract<Dialog, { type: "compress" }> }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [format, setFormat] = useState<(typeof FORMATS)[number]>("zip");
  const defaultStem =
    d.items.length === 1 ? d.items[0].name.replace(/\.[^.]+$/, "") : "archive";
  const [name, setName] = useState(defaultStem);

  const go = () => {
    if (!name.trim()) return;
    close();
    void useOps.getState().run(
      t("dialog.compress"),
      {
        op: "compress",
        sources: d.items.map((e) => e.path),
        dest: joinPath(d.destDir, `${name.trim()}.${format}`),
        format,
      },
      (done) =>
        useOps.getState().toast(t("status.compressed", { name: done.name ?? "" })),
    );
  };

  return (
    <Overlay onClose={close}>
      <h2>{t("dialog.compressTitle", { count: d.items.length })}</h2>
      <div className="body">
        <ItemList names={d.items.map((e) => e.name)} />
        <label>{t("dialog.archiveName")}</label>
        <input
          value={name}
          spellCheck={false}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && go()}
        />
        <label>{t("dialog.format")}</label>
        <div className="seg">
          {FORMATS.map((f) => (
            <button key={f} className={format === f ? "on" : ""} onClick={() => setFormat(f)}>
              .{f}
            </button>
          ))}
        </div>
      </div>
      <div className="buttons">
        <button className="btn" onClick={close}>
          {t("dialog.cancel")}
        </button>
        <button className="btn primary" onClick={go}>
          {t("dialog.compress")} ↵
        </button>
      </div>
    </Overlay>
  );
}

function ConflictDialog() {
  const { t } = useTranslation();
  const conflict = useOps((s) => s.conflict)!;
  const answer = (choice: ConflictChoice) => useOps.getState().answerConflict(choice);

  return (
    <Overlay onClose={() => answer("cancelOp")}>
      <h2>{t("dialog.conflictTitle")}</h2>
      <div className="body">
        <p>{t("dialog.conflictMsg", { name: baseName(conflict.dest) })}</p>
        <div className="list">
          <div>→ {conflict.src}</div>
          <div>⇒ {conflict.dest}</div>
        </div>
      </div>
      <div className="buttons">
        <button className="btn" onClick={() => answer("cancelOp")}>
          {t("dialog.cancelOp")}
        </button>
        <button className="btn" onClick={() => answer("skipAll")}>
          {t("dialog.skipAll")}
        </button>
        <button className="btn" onClick={() => answer("skip")}>
          {t("dialog.skip")}
        </button>
        <button className="btn" onClick={() => answer("keepBoth")}>
          {t("dialog.keepBoth")}
        </button>
        <button className="btn" onClick={() => answer("overwriteAll")}>
          {t("dialog.overwriteAll")}
        </button>
        <button className="btn primary" onClick={() => answer("overwrite")}>
          {t("dialog.overwrite")}
        </button>
      </div>
    </Overlay>
  );
}
