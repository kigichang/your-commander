import { X } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import * as ipc from "../ipc/commands";
import type { SmbConnection, SshConfigHost, SshConnection } from "../ipc/types";
import { joinPath } from "../lib/path";
import { useDialogs } from "../store/dialogs";
import { useOps } from "../store/ops";
import type { Side } from "../store/panels";
import { usePanels } from "../store/panels";
import { useSettings } from "../store/settings";
import { Icon } from "./icons";

type Protocol = "sftp" | "ftp" | "s3" | "webdav" | "smb";
const PROTOCOLS: Protocol[] = ["sftp", "ftp", "s3", "webdav", "smb"];
const READY: Set<Protocol> = new Set(["sftp", "smb"]);

type SshAuthMode = "key" | "password" | "agent";

const emptySmb: SmbConnection = {
  name: "",
  server: "",
  share: "",
  username: "",
  start_in: "",
  remember: true,
};

const emptySsh: SshConnection = {
  name: "",
  host: "",
  port: 22,
  username: "",
  identity_file: "",
  use_password: false,
  use_agent: false,
  start_in: "",
  remember: true,
};

type Saved =
  | { proto: "sftp"; ssh: SshConnection }
  | { proto: "smb"; smb: SmbConnection };

export function ConnectDialog({ protocol: initial }: { protocol: "smb" | "sftp" }) {
  const { t } = useTranslation();
  const close = useDialogs((s) => s.close);
  const config = useSettings((s) => s.config);
  const activeSide = usePanels((s) => s.activeSide);
  const [protocol, setProtocol] = useState<Protocol>(initial);
  const [side, setSide] = useState<Side>(activeSide);
  const [ssh, setSsh] = useState<SshConnection>(emptySsh);
  const [smb, setSmb] = useState<SmbConnection>(emptySmb);
  const [secret, setSecret] = useState("");
  const [hosts, setHosts] = useState<SshConfigHost[]>([]);
  const [busy, setBusy] = useState<"" | "test" | "connect">("");
  // Password prompt shown when connecting with a username but no password.
  const [askPw, setAskPw] = useState(false);
  const [pwInput, setPwInput] = useState("");
  // Share picker shown when connecting with the share left empty.
  const [shareList, setShareList] = useState<string[] | null>(null);

  useEffect(() => {
    void ipc.sshHosts().then(setHosts);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        if (askPw) setAskPw(false);
        else if (shareList) setShareList(null);
        else close();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [close, askPw, shareList]);

  const authMode: SshAuthMode = ssh.use_agent ? "agent" : ssh.use_password ? "password" : "key";
  const setAuthMode = (m: SshAuthMode) =>
    setSsh((s) => ({ ...s, use_agent: m === "agent", use_password: m === "password" }));

  const saved: Saved[] = [
    ...(config?.ssh_connections ?? []).map((c) => ({ proto: "sftp", ssh: c }) as Saved),
    ...(config?.smb_connections ?? []).map((c) => ({ proto: "smb", smb: c }) as Saved),
  ].sort((a, b) => nameOf(a).localeCompare(nameOf(b)));

  function nameOf(s: Saved): string {
    return s.proto === "sftp"
      ? s.ssh.name || `${s.ssh.username}@${s.ssh.host}`
      : s.smb.name || `${s.smb.server}/${s.smb.share}`;
  }
  function descOf(s: Saved): string {
    return s.proto === "sftp"
      ? `${s.ssh.username}@${s.ssh.host}:${s.ssh.port}`
      : `//${s.smb.username ? s.smb.username + "@" : ""}${s.smb.server}/${s.smb.share}`;
  }

  const pick = (s: Saved) => {
    setSecret("");
    setProtocol(s.proto);
    if (s.proto === "sftp") {
      setSsh(s.ssh);
      if (s.ssh.remember && s.ssh.use_password && s.ssh.username) {
        void ipc.sshSavedPassword(s.ssh.username, s.ssh.host).then((p) => p && setSecret(p));
      }
    } else {
      setSmb(s.smb);
      if (s.smb.remember && s.smb.username) {
        void ipc
          .smbSavedPassword(s.smb.username, s.smb.server, s.smb.share)
          .then((p) => p && setSecret(p));
      }
    }
  };

  const remove = (s: Saved) => {
    if (s.proto === "sftp") {
      void ipc.sshForgetPassword(s.ssh.username, s.ssh.host);
      useSettings.getState().save({
        ssh_connections: (config?.ssh_connections ?? []).filter(
          (c) => !(c.host === s.ssh.host && c.username === s.ssh.username),
        ),
      });
    } else {
      if (s.smb.username)
        void ipc.smbForgetPassword(s.smb.username, s.smb.server, s.smb.share);
      useSettings.getState().save({
        smb_connections: (config?.smb_connections ?? []).filter(
          (c) =>
            !(c.server === s.smb.server && c.share === s.smb.share && c.username === s.smb.username),
        ),
      });
    }
  };

  const toast = (text: string, kind?: "info" | "error") => useOps.getState().toast(text, kind);

  const save = (pwOverride?: string, shareOverride?: string) => {
    const pw = pwOverride ?? secret;
    const share = shareOverride ?? smb.share;
    if (protocol === "sftp") {
      if (!ssh.host || !ssh.username) return;
      const name = ssh.name.trim() || `${ssh.username}@${ssh.host}`;
      const rest = (config?.ssh_connections ?? []).filter(
        (c) => !(c.host === ssh.host && c.username === ssh.username),
      );
      useSettings.getState().save({ ssh_connections: [...rest, { ...ssh, name }] });
      if (ssh.use_password && secret) void ipc.sshStorePassword(ssh.username, ssh.host, secret);
    } else if (protocol === "smb") {
      if (!smb.server || !share) return;
      const name = smb.name.trim() || `${smb.server}/${share}`;
      const rest = (config?.smb_connections ?? []).filter(
        (c) => !(c.server === smb.server && c.share === share && c.username === smb.username),
      );
      useSettings.getState().save({ smb_connections: [...rest, { ...smb, share, name }] });
      if (smb.username && pw)
        void ipc.smbStorePassword(smb.username, smb.server, share, pw);
    }
    toast(t("connect.saved2"));
  };

  const sftpParams = () => ({
    host: ssh.host,
    port: ssh.port || 22,
    username: ssh.username,
    usePassword: ssh.use_password,
    useAgent: ssh.use_agent,
    password: ssh.use_password ? secret : undefined,
    identityFile: authMode === "key" ? ssh.identity_file : undefined,
    passphrase: authMode === "key" ? secret || undefined : undefined,
    startIn: ssh.start_in || undefined,
  });

  const test = async () => {
    if (busy) return;
    setBusy("test");
    try {
      if (protocol === "sftp") await ipc.sftpTest(sftpParams());
      else await ipc.smbTest(smb.server);
      toast(t("connect.testOk"));
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy("");
    }
  };

  // Share left empty: ask the server for its share list and let the user pick.
  const mountSmb = async (password: string) => {
    const share = smb.share.trim();
    if (share) {
      doMount(password, share);
      return;
    }
    setBusy("connect");
    try {
      const shares = await ipc.smbShares(smb.server, smb.username, password);
      if (shares.length === 0) {
        toast(t("connect.noShares"), "error");
      } else {
        setShareList(shares);
      }
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy("");
    }
  };

  const doMount = (password: string, share: string) => {
    if (smb.remember) save(password, share);
    close();
    void useOps.getState().run(
      "SMB",
      {
        op: "smbMount",
        server: smb.server,
        share,
        username: smb.username,
        password,
      },
      (done) => {
        if (done.name) {
          toast(t("status.mounted", { path: done.name }));
          const startIn = smb.start_in.replace(/^\/+/, "");
          const target = startIn ? joinPath(done.name, startIn) : done.name;
          void usePanels.getState().navigate(side, target, { kind: "local" }, null);
        }
      },
    );
  };

  const connect = async () => {
    if (busy) return;
    setBusy("connect");
    try {
      if (protocol === "sftp") {
        const res = await ipc.sftpConnect(sftpParams());
        if (ssh.remember) save();
        close();
        toast(t("status.connected", { host: ssh.host }));
        void usePanels
          .getState()
          .navigate(side, res.home, { kind: "sftp", connId: res.connId }, res.label);
      } else {
        // No password typed: fall back to the Keychain, then ask for one —
        // never hand NetFS a passwordless URL that fails or double-prompts.
        let pw = secret;
        if (!pw && smb.username) {
          pw = (await ipc.smbSavedPassword(smb.username, smb.server, smb.share)) ?? "";
          if (pw) setSecret(pw);
        }
        if (!pw && smb.username) {
          setPwInput("");
          setAskPw(true);
          setBusy("");
          return;
        }
        await mountSmb(pw);
      }
    } catch (e) {
      toast(String(e), "error");
      setBusy("");
    }
  };

  const submitPw = () => {
    setAskPw(false);
    setSecret(pwInput);
    void mountSmb(pwInput);
  };

  const canSubmit =
    protocol === "sftp" ? !!(ssh.host && ssh.username) : !!smb.server;

  return (
    <div className="overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div className="dialog wide">
        <h2>{t("connect.title")}</h2>
        <div className="connect-grid">
          <div className="saved-list">
            <div className="headline">{t("connect.saved")}</div>
            {saved.length === 0 && <div className="none">{t("connect.noSaved")}</div>}
            {saved.map((s) => (
              <div key={s.proto + nameOf(s) + descOf(s)} className="saved">
                <button className="pick" onClick={() => pick(s)}>
                  <span className="label">
                    {nameOf(s)} <em className="proto">{s.proto.toUpperCase()}</em>
                  </span>
                  <span className="sub">{descOf(s)}</span>
                </button>
                <button className="close" title={t("connect.deleteSaved")} onClick={() => remove(s)}>
                  <Icon of={X} size={12} />
                </button>
              </div>
            ))}
            <button
              className="tab-new new-conn"
              onClick={() => {
                setSsh(emptySsh);
                setSmb(emptySmb);
                setSecret("");
              }}
            >
              + {t("connect.new")}
            </button>
          </div>
          <div className="form">
            <div className="seg proto-seg">
              {PROTOCOLS.map((p) => (
                <button
                  key={p}
                  className={protocol === p ? "on" : ""}
                  disabled={!READY.has(p)}
                  title={READY.has(p) ? "" : t("status.comingSoon")}
                  onClick={() => READY.has(p) && setProtocol(p)}
                >
                  {p.toUpperCase()}
                </button>
              ))}
            </div>

            {protocol === "sftp" ? (
              <>
                {hosts.length > 0 && (
                  <>
                    <label>{t("connect.fromSshConfig")}</label>
                    <select
                      value=""
                      onChange={(e) => {
                        const h = hosts.find((x) => x.alias === e.target.value);
                        if (h)
                          setSsh((s) => ({
                            ...s,
                            name: h.alias,
                            host: h.hostName,
                            port: h.port,
                            username: h.user,
                            identity_file: h.identity,
                            use_password: false,
                            use_agent: false,
                          }));
                      }}
                    >
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
                <input
                  value={ssh.name}
                  spellCheck={false}
                  onChange={(e) => setSsh((s) => ({ ...s, name: e.target.value }))}
                />
                <div className="two">
                  <div>
                    <label>{t("connect.host")}</label>
                    <input
                      value={ssh.host}
                      spellCheck={false}
                      placeholder="prod.example.com"
                      onChange={(e) => setSsh((s) => ({ ...s, host: e.target.value }))}
                    />
                  </div>
                  <div>
                    <label>{t("connect.port")}</label>
                    <input
                      value={ssh.port}
                      onChange={(e) =>
                        setSsh((s) => ({ ...s, port: Number(e.target.value) || 22 }))
                      }
                    />
                  </div>
                </div>
                <label>{t("connect.username")}</label>
                <input
                  value={ssh.username}
                  spellCheck={false}
                  onChange={(e) => setSsh((s) => ({ ...s, username: e.target.value }))}
                />
                <label>{t("connect.auth")}</label>
                <div className="seg">
                  {(["key", "password", "agent"] as SshAuthMode[]).map((m) => (
                    <button
                      key={m}
                      className={authMode === m ? "on" : ""}
                      onClick={() => setAuthMode(m)}
                    >
                      {t(`connect.auth_${m}`)}
                    </button>
                  ))}
                </div>
                {authMode === "key" && (
                  <>
                    <label>{t("connect.identityFile")}</label>
                    <input
                      value={ssh.identity_file}
                      spellCheck={false}
                      placeholder={t("connect.identityHint")}
                      onChange={(e) => setSsh((s) => ({ ...s, identity_file: e.target.value }))}
                    />
                  </>
                )}
                {authMode !== "agent" && (
                  <>
                    <label>
                      {authMode === "password" ? t("connect.password") : t("connect.passphrase")}
                    </label>
                    <input
                      type="password"
                      value={secret}
                      onChange={(e) => setSecret(e.target.value)}
                      onKeyDown={(e) => e.key === "Enter" && void connect()}
                    />
                  </>
                )}
                <label>{t("connect.startIn")}</label>
                <input
                  value={ssh.start_in}
                  spellCheck={false}
                  placeholder="/var/www"
                  onChange={(e) => setSsh((s) => ({ ...s, start_in: e.target.value }))}
                />
                <label className="check">
                  <input
                    type="checkbox"
                    checked={ssh.remember}
                    onChange={(e) => setSsh((s) => ({ ...s, remember: e.target.checked }))}
                  />
                  {t("connect.remember")}
                </label>
              </>
            ) : (
              <>
                <label>{t("connect.name")}</label>
                <input
                  value={smb.name}
                  spellCheck={false}
                  onChange={(e) => setSmb((s) => ({ ...s, name: e.target.value }))}
                />
                <div className="two">
                  <div>
                    <label>{t("connect.server")}</label>
                    <input
                      value={smb.server}
                      spellCheck={false}
                      placeholder="nas.local"
                      onChange={(e) => setSmb((s) => ({ ...s, server: e.target.value }))}
                    />
                  </div>
                  <div>
                    <label>{t("connect.share")}</label>
                    <input
                      value={smb.share}
                      spellCheck={false}
                      placeholder={t("connect.shareHint")}
                      onChange={(e) => setSmb((s) => ({ ...s, share: e.target.value }))}
                    />
                  </div>
                </div>
                <div className="two">
                  <div>
                    <label>{t("connect.username")}</label>
                    <input
                      value={smb.username}
                      spellCheck={false}
                      onChange={(e) => setSmb((s) => ({ ...s, username: e.target.value }))}
                    />
                  </div>
                  <div>
                    <label>{t("connect.password")}</label>
                    <input
                      type="password"
                      value={secret}
                      onChange={(e) => setSecret(e.target.value)}
                      onKeyDown={(e) => e.key === "Enter" && void connect()}
                    />
                  </div>
                </div>
                <label>{t("connect.startIn")}</label>
                <input
                  value={smb.start_in}
                  spellCheck={false}
                  placeholder="photos/2026"
                  onChange={(e) => setSmb((s) => ({ ...s, start_in: e.target.value }))}
                />
                <label className="check">
                  <input
                    type="checkbox"
                    checked={smb.remember}
                    onChange={(e) => setSmb((s) => ({ ...s, remember: e.target.checked }))}
                  />
                  {t("connect.remember")}
                </label>
              </>
            )}

            <div className="buttons">
              <button className="btn" disabled={!canSubmit || busy !== ""} onClick={() => void test()}>
                {busy === "test" ? t("connect.testing") : t("connect.test")}
              </button>
              <button
                className="btn"
                disabled={protocol === "smb" ? !(smb.server && smb.share) : !canSubmit}
                onClick={() => save()}
              >
                {t("connect.save")}
              </button>
              <span className="pane-pick">
                <span className="seg">
                  <button className={side === 0 ? "on" : ""} onClick={() => setSide(0)}>
                    {t("connect.paneLeft")}
                  </button>
                  <button className={side === 1 ? "on" : ""} onClick={() => setSide(1)}>
                    {t("connect.paneRight")}
                  </button>
                </span>
              </span>
              <button
                className="btn primary"
                disabled={!canSubmit || busy !== ""}
                onClick={() => void connect()}
              >
                {busy === "connect" ? t("connect.connecting") : t("connect.connect") + " ↵"}
              </button>
            </div>
          </div>
        </div>
        {shareList && (
          <div
            className="overlay"
            onMouseDown={(e) => e.target === e.currentTarget && setShareList(null)}
          >
            <div className="dialog">
              <h2>{t("connect.pickShare")}</h2>
              <div className="settings-note">{`//${smb.server}`}</div>
              <div className="share-list">
                {shareList.map((share) => (
                  <button
                    key={share}
                    className="pick"
                    onClick={() => {
                      setSmb((s) => ({ ...s, share }));
                      setShareList(null);
                      doMount(secret, share);
                    }}
                  >
                    {share}
                  </button>
                ))}
              </div>
              <div className="buttons">
                <button className="btn" onClick={() => setShareList(null)}>
                  {t("dialog.cancel")}
                </button>
              </div>
            </div>
          </div>
        )}
        {askPw && (
          <div
            className="overlay"
            onMouseDown={(e) => e.target === e.currentTarget && setAskPw(false)}
          >
            <div className="dialog">
              <h2>{t("connect.pwTitle")}</h2>
              <div className="settings-note">
                {`${smb.username}@${smb.server}/${smb.share}`}
              </div>
              <label>{t("connect.password")}</label>
              <input
                type="password"
                autoFocus
                value={pwInput}
                onChange={(e) => setPwInput(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && submitPw()}
              />
              <label className="check">
                <input
                  type="checkbox"
                  checked={smb.remember}
                  onChange={(e) => setSmb((s) => ({ ...s, remember: e.target.checked }))}
                />
                {t("connect.remember")}
              </label>
              <div className="buttons">
                <button className="btn" onClick={() => setAskPw(false)}>
                  {t("dialog.cancel")}
                </button>
                <button className="btn primary" onClick={submitPw}>
                  {t("connect.connect")}
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
