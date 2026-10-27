import clsx from "clsx";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { SmbConnection, SshConfigHost, SshConnection } from "../ipc/types";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";

const emptySmb: SmbConnection = {
  name: "",
  server: "",
  share: "",
  username: "",
  remember: true,
};

const emptySsh: SshConnection = {
  name: "",
  host: "",
  port: 22,
  username: "",
  identity_file: "",
  use_password: false,
  remember: true,
};

export function ConnectDialog({ protocol: initial }: { protocol: "smb" | "sftp" }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const [protocol, setProtocol] = useState(initial);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [close]);

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog wide">
        <h2>{t("connect.title")}</h2>
        <div className="seg" style={{ marginBottom: "var(--space-4)" }}>
          <button className={protocol === "smb" ? "on" : ""} onClick={() => setProtocol("smb")}>
            SMB
          </button>
          <button className={protocol === "sftp" ? "on" : ""} onClick={() => setProtocol("sftp")}>
            SFTP
          </button>
        </div>
        {protocol === "smb" ? <SmbForm /> : <SftpForm />}
      </div>
    </div>
  );
}

function SavedList<T extends { name: string }>({
  items,
  selected,
  describe,
  onPick,
  onDelete,
}: {
  items: T[];
  selected: string;
  describe: (item: T) => string;
  onPick: (item: T) => void;
  onDelete: (item: T) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="saved-list">
      <div className="headline">{t("connect.saved")}</div>
      {items.length === 0 && <div className="none">{t("connect.noSaved")}</div>}
      {items.map((item) => (
        <div
          key={item.name + describe(item)}
          className={clsx("saved", selected === item.name && "on")}
        >
          <button className="pick" onClick={() => onPick(item)}>
            <span className="label">{item.name || describe(item)}</span>
            <span className="sub">{describe(item)}</span>
          </button>
          <button
            className="close"
            title={t("connect.deleteSaved")}
            onClick={() => onDelete(item)}
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}

function SmbForm() {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const config = useSettings((s) => s.config);
  const [form, setForm] = useState<SmbConnection>(emptySmb);
  const [password, setPassword] = useState("");
  const saved = config?.smb_connections ?? [];
  const set = (patch: Partial<SmbConnection>) => setForm((f) => ({ ...f, ...patch }));

  const pick = (conn: SmbConnection) => {
    setForm(conn);
    setPassword("");
    if (conn.remember && conn.username) {
      void ipc
        .smbSavedPassword(conn.username, conn.server, conn.share)
        .then((p) => p && setPassword(p));
    }
  };

  const remove = (conn: SmbConnection) => {
    if (conn.username) void ipc.smbForgetPassword(conn.username, conn.server, conn.share);
    useSettings.getState().save({
      smb_connections: saved.filter(
        (c) => !(c.server === conn.server && c.share === conn.share && c.username === conn.username),
      ),
    });
  };

  const connect = () => {
    if (!form.server || !form.share) return;
    const name = form.name.trim() || `${form.server}/${form.share}`;
    if (form.remember) {
      const rest = saved.filter(
        (c) => !(c.server === form.server && c.share === form.share && c.username === form.username),
      );
      useSettings.getState().save({ smb_connections: [...rest, { ...form, name }] });
      if (form.username && password) {
        void ipc.smbStorePassword(form.username, form.server, form.share, password);
      }
    }
    close();
    const side = usePanels.getState().activeSide;
    void useOps.getState().run(
      "SMB",
      {
        op: "smbMount",
        server: form.server,
        share: form.share,
        username: form.username,
        password,
      },
      (done) => {
        if (done.name) {
          useOps.getState().toast(t("status.mounted", { path: done.name }));
          void usePanels.getState().navigate(side, done.name, { kind: "local" }, null);
        }
      },
    );
  };

  return (
    <div className="connect-grid">
      <SavedList
        items={saved}
        selected={form.name}
        describe={(c) => `//${c.username ? c.username + "@" : ""}${c.server}/${c.share}`}
        onPick={pick}
        onDelete={remove}
      />
      <div className="form">
        <label>{t("connect.name")}</label>
        <input value={form.name} onChange={(e) => set({ name: e.target.value })} spellCheck={false} />
        <div className="two">
          <div>
            <label>{t("connect.server")}</label>
            <input
              value={form.server}
              onChange={(e) => set({ server: e.target.value })}
              placeholder="nas.local"
              spellCheck={false}
            />
          </div>
          <div>
            <label>{t("connect.share")}</label>
            <input value={form.share} onChange={(e) => set({ share: e.target.value })} spellCheck={false} />
          </div>
        </div>
        <div className="two">
          <div>
            <label>{t("connect.username")}</label>
            <input value={form.username} onChange={(e) => set({ username: e.target.value })} spellCheck={false} />
          </div>
          <div>
            <label>{t("connect.password")}</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && connect()}
            />
          </div>
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={form.remember}
            onChange={(e) => set({ remember: e.target.checked })}
          />
          {t("connect.remember")}
        </label>
        <div className="buttons">
          <button className="btn" onClick={close}>
            {t("dialog.cancel")}
          </button>
          <button className="btn primary" disabled={!form.server || !form.share} onClick={connect}>
            {t("connect.connect")} ↵
          </button>
        </div>
      </div>
    </div>
  );
}

function SftpForm() {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const config = useSettings((s) => s.config);
  const [form, setForm] = useState<SshConnection>(emptySsh);
  const [secret, setSecret] = useState("");
  const [hosts, setHosts] = useState<SshConfigHost[]>([]);
  const [busy, setBusy] = useState(false);
  const saved = config?.ssh_connections ?? [];
  const set = (patch: Partial<SshConnection>) => setForm((f) => ({ ...f, ...patch }));

  useEffect(() => {
    void ipc.sshHosts().then(setHosts);
  }, []);

  const pick = (conn: SshConnection) => {
    setForm(conn);
    setSecret("");
    if (conn.remember && conn.use_password && conn.username) {
      void ipc.sshSavedPassword(conn.username, conn.host).then((p) => p && setSecret(p));
    }
  };

  const pickConfigHost = (alias: string) => {
    const h = hosts.find((x) => x.alias === alias);
    if (!h) return;
    set({
      name: h.alias,
      host: h.hostName,
      port: h.port,
      username: h.user,
      identity_file: h.identity,
      use_password: false,
    });
  };

  const remove = (conn: SshConnection) => {
    void ipc.sshForgetPassword(conn.username, conn.host);
    useSettings.getState().save({
      ssh_connections: saved.filter((c) => !(c.host === conn.host && c.username === conn.username)),
    });
  };

  const connect = async () => {
    if (!form.host || !form.username || busy) return;
    setBusy(true);
    try {
      const res = await ipc.sftpConnect({
        host: form.host,
        port: form.port || 22,
        username: form.username,
        usePassword: form.use_password,
        password: form.use_password ? secret : undefined,
        identityFile: form.use_password ? undefined : form.identity_file,
        passphrase: form.use_password ? undefined : secret || undefined,
      });
      const name = form.name.trim() || `${form.username}@${form.host}`;
      if (form.remember) {
        const rest = saved.filter((c) => !(c.host === form.host && c.username === form.username));
        useSettings.getState().save({ ssh_connections: [...rest, { ...form, name }] });
        if (form.use_password && secret) {
          void ipc.sshStorePassword(form.username, form.host, secret);
        }
      }
      close();
      useOps.getState().toast(t("status.connected", { host: form.host }));
      const side = usePanels.getState().activeSide;
      void usePanels
        .getState()
        .navigate(side, res.home, { kind: "sftp", connId: res.connId }, res.label);
    } catch (e) {
      setBusy(false);
      useOps.getState().toast(String(e), "error");
    }
  };

  return (
    <div className="connect-grid">
      <SavedList
        items={saved}
        selected={form.name}
        describe={(c) => `${c.username}@${c.host}:${c.port}`}
        onPick={pick}
        onDelete={remove}
      />
      <div className="form">
        {hosts.length > 0 && (
          <>
            <label>{t("connect.fromSshConfig")}</label>
            <select value="" onChange={(e) => pickConfigHost(e.target.value)}>
              <option value="">{t("connect.pickHost")}</option>
              {hosts.map((h) => (
                <option key={h.alias} value={h.alias}>
                  {h.alias} — {h.user ? h.user + "@" : ""}
                  {h.hostName}
                </option>
              ))}
            </select>
          </>
        )}
        <label>{t("connect.name")}</label>
        <input value={form.name} onChange={(e) => set({ name: e.target.value })} spellCheck={false} />
        <div className="two">
          <div>
            <label>{t("connect.host")}</label>
            <input
              value={form.host}
              onChange={(e) => set({ host: e.target.value })}
              placeholder="prod.example.com"
              spellCheck={false}
            />
          </div>
          <div>
            <label>{t("connect.port")}</label>
            <input
              value={form.port}
              onChange={(e) => set({ port: Number(e.target.value) || 22 })}
              spellCheck={false}
            />
          </div>
        </div>
        <label>{t("connect.username")}</label>
        <input value={form.username} onChange={(e) => set({ username: e.target.value })} spellCheck={false} />
        <label>{t("connect.auth")}</label>
        <div className="seg">
          <button className={!form.use_password ? "on" : ""} onClick={() => set({ use_password: false })}>
            {t("connect.authKey")}
          </button>
          <button className={form.use_password ? "on" : ""} onClick={() => set({ use_password: true })}>
            {t("connect.authPassword")}
          </button>
        </div>
        {!form.use_password && (
          <>
            <label>{t("connect.identityFile")}</label>
            <input
              value={form.identity_file}
              onChange={(e) => set({ identity_file: e.target.value })}
              placeholder={t("connect.identityHint")}
              spellCheck={false}
            />
          </>
        )}
        <label>{form.use_password ? t("connect.password") : t("connect.passphrase")}</label>
        <input
          type="password"
          value={secret}
          onChange={(e) => setSecret(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void connect()}
        />
        <label className="check">
          <input
            type="checkbox"
            checked={form.remember}
            onChange={(e) => set({ remember: e.target.checked })}
          />
          {t("connect.remember")}
        </label>
        <div className="buttons">
          <button className="btn" onClick={close}>
            {t("dialog.cancel")}
          </button>
          <button
            className="btn primary"
            disabled={!form.host || !form.username || busy}
            onClick={() => void connect()}
          >
            {busy ? t("connect.connecting") : t("connect.connect") + " ↵"}
          </button>
        </div>
      </div>
    </div>
  );
}
