use super::provider::FsEntry;
use crate::ops::queue::{cancelled_err, OpCtx};
use russh::client;
use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK: usize = 256 * 1024;

/// Symlink targets stat'ed at once while listing a folder. Each is a round
/// trip; one after another, a folder of links (/usr/bin, node_modules/.bin)
/// costs a full RTT per link on a slow connection.
const STAT_CONCURRENCY: usize = 16;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Runtime::new().expect("tokio runtime"))
}

pub enum SshAuth {
    Password(String),
    Key {
        path: Option<PathBuf>,
        passphrase: Option<String>,
    },
    /// Every identity the running ssh-agent offers, in order.
    Agent,
}

/// Why `connect` refused or failed, so the UI can offer the right next
/// step: trust a first-seen host key, or warn that a known one changed.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ConnectError {
    /// Not in ~/.ssh/known_hosts and `trust_host_key` was off.
    #[serde(rename_all = "camelCase")]
    UnknownHostKey { key_type: String, fingerprint: String },
    /// known_hosts has a different key of the same type for this host —
    /// a reinstalled server or a man in the middle. Never auto-accepted.
    #[serde(rename_all = "camelCase")]
    HostKeyChanged { key_type: String, fingerprint: String },
    Other { message: String },
}

impl From<String> for ConnectError {
    fn from(message: String) -> Self {
        ConnectError::Other { message }
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::UnknownHostKey { key_type, fingerprint } => {
                write!(f, "Unknown host key ({key_type} {fingerprint})")
            }
            ConnectError::HostKeyChanged {
                key_type,
                fingerprint,
            } => write!(
                f,
                "Host key changed ({key_type} {fingerprint}); known_hosts has a different key"
            ),
            ConnectError::Other { message } => f.write_str(message),
        }
    }
}

/// Verdict of the known_hosts check, handed back out of the handler.
type Verdict = Arc<Mutex<Option<ConnectError>>>;

fn known_hosts_file() -> PathBuf {
    super::local::home_dir().join(".ssh").join("known_hosts")
}

/// Accept the server key when `known_hosts` records it for host:port.
/// A first-seen key is recorded when `trust_new`, else refused with
/// `UnknownHostKey`; a different recorded key is always refused.
fn judge_host_key(
    known_hosts: &Path,
    host: &str,
    port: u16,
    key: &PublicKey,
    trust_new: bool,
) -> Result<(), ConnectError> {
    let key_type = key.algorithm().to_string();
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    match check_known_hosts_path(host, port, key, known_hosts) {
        Ok(true) => Ok(()),
        Ok(false) if trust_new => learn_known_hosts_path(host, port, key, known_hosts)
            .map_err(|e| format!("{}: {e}", known_hosts.display()).into()),
        Ok(false) => Err(ConnectError::UnknownHostKey {
            key_type,
            fingerprint,
        }),
        // russh's line number skips comment lines, so it is not reported.
        Err(russh::keys::Error::KeyChanged { .. }) => Err(ConnectError::HostKeyChanged {
            key_type,
            fingerprint,
        }),
        Err(e) => Err(format!("{}: {e}", known_hosts.display()).into()),
    }
}

struct Handler {
    known_hosts: PathBuf,
    host: String,
    port: u16,
    /// Record a first-seen key in known_hosts instead of refusing it
    /// (the user confirmed its fingerprint).
    trust_new: bool,
    verdict: Verdict,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => PublicKey::from(cert.public_key().clone()),
        };
        match judge_host_key(&self.known_hosts, &self.host, self.port, &key, self.trust_new) {
            Ok(()) => Ok(true),
            Err(verdict) => {
                *self.verdict.lock().unwrap() = Some(verdict);
                Ok(false)
            }
        }
    }
}

/// A live SFTP session. Dropping the last Arc closes the connection.
pub struct SftpConnection {
    pub label: String,
    pub home: PathBuf,
    session: SftpSession,
    _handle: client::Handle<Handler>,
}

fn default_key_candidates() -> Vec<PathBuf> {
    let ssh_dir = super::local::home_dir().join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| ssh_dir.join(n))
        .collect()
}

/// Blocking connect; call from a worker thread. The server's host key must
/// match ~/.ssh/known_hosts; a first-seen key is refused with
/// `ConnectError::UnknownHostKey` unless `trust_host_key` is set, in which
/// case it is recorded there.
pub fn connect(
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
    trust_host_key: bool,
) -> Result<Arc<SftpConnection>, ConnectError> {
    connect_checked(&known_hosts_file(), host, port, user, auth, trust_host_key)
}

/// `connect` against a given known_hosts file.
fn connect_checked(
    known_hosts: &Path,
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
    trust_host_key: bool,
) -> Result<Arc<SftpConnection>, ConnectError> {
    let verdict: Verdict = Arc::new(Mutex::new(None));
    let handler = Handler {
        known_hosts: known_hosts.to_path_buf(),
        host: host.to_string(),
        port,
        trust_new: trust_host_key,
        verdict: verdict.clone(),
    };
    let result = runtime().block_on(connect_with(handler, host, port, user, auth));
    // A key the handler rejected surfaces as a generic russh error; the
    // handler's own verdict says why.
    match result {
        Err(e) => Err(verdict.lock().unwrap().take().unwrap_or_else(|| e.into())),
        Ok(conn) => Ok(conn),
    }
}

async fn connect_with(
    handler: Handler,
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
) -> Result<Arc<SftpConnection>, String> {
    {
        let config = Arc::new(client::Config::default());
        let mut handle = client::connect(config, (host, port), handler)
            .await
            .map_err(|e| e.to_string())?;

        let authed = match auth {
            SshAuth::Password(p) => handle
                .authenticate_password(user, p)
                .await
                .map_err(|e| e.to_string())?
                .success(),
            SshAuth::Key { path, passphrase } => {
                let candidates = match path {
                    Some(p) => vec![p.clone()],
                    None => default_key_candidates(),
                };
                let mut ok = false;
                let mut last_err: Option<String> = None;
                for key_path in candidates {
                    if !key_path.exists() {
                        continue;
                    }
                    let key = match load_secret_key(&key_path, passphrase.as_deref()) {
                        Ok(k) => k,
                        Err(e) => {
                            last_err = Some(format!("{}: {e}", key_path.display()));
                            continue;
                        }
                    };
                    let alg = handle
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|e| e.to_string())?
                        .flatten();
                    let res = handle
                        .authenticate_publickey(
                            user,
                            PrivateKeyWithHashAlg::new(Arc::new(key), alg),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    if res.success() {
                        ok = true;
                        break;
                    }
                }
                if !ok && let Some(err) = last_err {
                    return Err(err);
                }
                ok
            }
            SshAuth::Agent => {
                let mut agent = russh::keys::agent::client::AgentClient::connect_env()
                    .await
                    .map_err(|e| format!("ssh-agent: {e}"))?;
                let identities = agent
                    .request_identities()
                    .await
                    .map_err(|e| format!("ssh-agent: {e}"))?;
                if identities.is_empty() {
                    return Err("ssh-agent has no identities".to_string());
                }
                let mut ok = false;
                for identity in identities {
                    let alg = handle
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|e| e.to_string())?
                        .flatten();
                    let key = identity.public_key().into_owned();
                    let result = handle
                        .authenticate_publickey_with(user, key, alg, &mut agent)
                        .await
                        .map_err(|e| e.to_string())?;
                    if result.success() {
                        ok = true;
                        break;
                    }
                }
                ok
            }
        };
        if !authed {
            return Err("Authentication failed".to_string());
        }

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|e| e.to_string())?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| e.to_string())?;
        let session = SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| e.to_string())?;
        let home = session
            .canonicalize(".")
            .await
            .map_err(|e| e.to_string())?;

        Ok(Arc::new(SftpConnection {
            label: format!("{user}@{host}"),
            home: PathBuf::from(home),
            session,
            _handle: handle,
        }))
    }
}

fn rp(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

impl SftpConnection {
    pub fn list(&self, path: &Path) -> std::io::Result<Vec<FsEntry>> {
        use futures::stream::{self, StreamExt};
        runtime().block_on(async {
            let dir = self
                .session
                .read_dir(rp(path))
                .await
                .map_err(std::io::Error::other)?;
            // READDIR attrs are an lstat: follow symlinks so a link to a
            // directory can be entered; broken links keep the link's data.
            // The stats run concurrently (SFTP multiplexes requests by id);
            // `buffered` keeps the server's order.
            let resolved: Vec<_> = stream::iter(dir.map(|entry| async move {
                let name = entry.file_name();
                let mut meta = entry.metadata();
                let is_symlink = meta.is_symlink();
                let full = path.join(&name);
                if is_symlink
                    && let Ok(target) = self.session.metadata(rp(&full)).await
                {
                    meta = target;
                }
                (name, full, is_symlink, meta)
            }))
            .buffered(STAT_CONCURRENCY)
            .collect()
            .await;
            let mut out = Vec::with_capacity(resolved.len());
            for (name, full, is_symlink, meta) in resolved {
                let is_dir = meta.is_dir();
                let ext = if is_dir {
                    String::new()
                } else {
                    full.extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default()
                };
                let is_hidden = name.starts_with('.');
                out.push(FsEntry {
                    name,
                    path: full,
                    is_dir,
                    is_symlink,
                    size: meta.size.unwrap_or(0),
                    modified: meta.mtime.map(|s| s as u64 * 1000),
                    ext,
                    is_hidden,
                });
            }
            Ok(out)
        })
    }

    /// None if the path does not exist, Some(is_dir) otherwise.
    /// A symlink counts as existing even when broken (lstat), and is a
    /// directory only when its target is one.
    pub fn exists(&self, path: &Path) -> Option<bool> {
        runtime().block_on(async {
            let meta = self.session.symlink_metadata(rp(path)).await.ok()?;
            if !meta.is_symlink() {
                return Some(meta.is_dir());
            }
            let target = self.session.metadata(rp(path)).await;
            Some(target.map(|m| m.is_dir()).unwrap_or(false))
        })
    }

    /// Where a remote symlink points (verbatim, possibly relative).
    pub fn read_link(&self, path: &Path) -> Result<String, String> {
        runtime().block_on(async {
            self.session
                .read_link(rp(path))
                .await
                .map_err(|e| format!("{}: {e}", path.display()))
        })
    }

    /// Create a remote symlink at `link` pointing to `target`.
    pub fn make_symlink(&self, link: &Path, target: &str) -> Result<(), String> {
        runtime().block_on(async {
            // OpenSSH's sftp-server swaps SYMLINK's arguments relative to the
            // spec (target comes first on the wire); russh-sftp follows the
            // spec, so pass them swapped for the de-facto standard server.
            self.session
                .symlink(target, rp(link))
                .await
                .map_err(|e| format!("{}: {e}", link.display()))
        })
    }

    pub fn mkdir(&self, path: &Path) -> Result<(), String> {
        runtime().block_on(async {
            self.session
                .create_dir(rp(path))
                .await
                .map_err(|e| format!("{}: {e}", path.display()))
        })
    }

    pub fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        runtime().block_on(async {
            self.session
                .rename(rp(from), rp(to))
                .await
                .map_err(|e| format!("{}: {e}", from.display()))
        })
    }

    pub fn remove_recursive(&self, path: &Path) -> Result<(), String> {
        // lstat, not stat: a symlink is removed as a file, never recursed
        // into — deleting a link must not delete what it points to.
        let meta = runtime()
            .block_on(self.session.symlink_metadata(rp(path)))
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() && !meta.is_symlink() {
            let entries = self
                .list(path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            for entry in entries {
                self.remove_recursive(&entry.path)?;
            }
            runtime().block_on(async {
                self.session
                    .remove_dir(rp(path))
                    .await
                    .map_err(|e| format!("{}: {e}", path.display()))
            })
        } else {
            runtime().block_on(async {
                self.session
                    .remove_file(rp(path))
                    .await
                    .map_err(|e| format!("{}: {e}", path.display()))
            })
        }
    }

    /// Read up to `max_bytes` of a remote file into memory (previews).
    pub fn read_limited(&self, remote: &Path, max_bytes: u64) -> Result<Vec<u8>, String> {
        runtime().block_on(async {
            let mut file = self
                .session
                .open_with_flags(rp(remote), OpenFlags::READ)
                .await
                .map_err(|e| format!("{}: {e}", remote.display()))?;
            let mut out = Vec::new();
            let mut chunk = vec![0u8; CHUNK];
            while (out.len() as u64) < max_bytes {
                let n = file
                    .read(&mut chunk)
                    .await
                    .map_err(|e| format!("{}: {e}", remote.display()))?;
                if n == 0 {
                    break;
                }
                let take = (n as u64).min(max_bytes - out.len() as u64) as usize;
                out.extend_from_slice(&chunk[..take]);
            }
            Ok(out)
        })
    }

    /// Upload one local file, with progress and cancellation via `ctx`.
    pub fn upload_file(&self, ctx: &mut OpCtx, local: &Path, remote: &Path) -> Result<(), String> {
        let mut reader =
            std::fs::File::open(local).map_err(|e| format!("{}: {e}", local.display()))?;
        runtime().block_on(async {
            let mut file = self
                .session
                .open_with_flags(
                    rp(remote),
                    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                )
                .await
                .map_err(|e| format!("{}: {e}", remote.display()))?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if ctx.cancelled() {
                    return Err(cancelled_err());
                }
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| format!("{}: {e}", local.display()))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n])
                    .await
                    .map_err(|e| format!("{}: {e}", remote.display()))?;
                ctx.done_bytes += n as u64;
                ctx.report(&local.to_string_lossy());
            }
            file.shutdown()
                .await
                .map_err(|e| format!("{}: {e}", remote.display()))?;
            Ok(())
        })
    }

    /// Download one remote file, with progress and cancellation via `ctx`.
    pub fn download_file(
        &self,
        ctx: &mut OpCtx,
        remote: &Path,
        local: &Path,
    ) -> Result<(), String> {
        let mut writer =
            std::fs::File::create(local).map_err(|e| format!("{}: {e}", local.display()))?;
        runtime().block_on(async {
            let mut file = self
                .session
                .open_with_flags(rp(remote), OpenFlags::READ)
                .await
                .map_err(|e| format!("{}: {e}", remote.display()))?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if ctx.cancelled() {
                    drop(writer);
                    let _ = std::fs::remove_file(local);
                    return Err(cancelled_err());
                }
                let n = file
                    .read(&mut buf)
                    .await
                    .map_err(|e| format!("{}: {e}", remote.display()))?;
                if n == 0 {
                    break;
                }
                writer
                    .write_all(&buf[..n])
                    .map_err(|e| format!("{}: {e}", local.display()))?;
                ctx.done_bytes += n as u64;
                ctx.report(&remote.to_string_lossy());
            }
            Ok(())
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEMJYw2vj0FQnOUFOObQzNqgAoI5bVDkFHOmVVMlLmUW";
    const KEY_B: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAOvqqQNLgZt9ReHZcnUhZTqtvVaJ+eO+TwoSPIGf98S";

    fn key(s: &str) -> PublicKey {
        PublicKey::from_openssh(s).unwrap()
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_kh_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unknown_key_is_refused_unless_trusted_then_recorded() {
        let kh = tmp("new").join("known_hosts");
        let err = judge_host_key(&kh, "box", 22, &key(KEY_A), false).unwrap_err();
        match err {
            ConnectError::UnknownHostKey {
                key_type,
                fingerprint,
            } => {
                assert_eq!(key_type, "ssh-ed25519");
                assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!kh.exists(), "refusing must not write known_hosts");

        judge_host_key(&kh, "box", 22, &key(KEY_A), true).unwrap();
        let recorded = std::fs::read_to_string(&kh).unwrap();
        assert!(recorded.trim_start().starts_with("box ssh-ed25519 "), "{recorded}");
        // Known now, no trust flag needed.
        judge_host_key(&kh, "box", 22, &key(KEY_A), false).unwrap();
        // Same host on another port is a different entry.
        assert!(judge_host_key(&kh, "box", 2222, &key(KEY_A), false).is_err());
    }

    #[test]
    fn changed_key_is_refused_even_when_trusting_new() {
        let kh = tmp("changed").join("known_hosts");
        std::fs::write(&kh, format!("# comment\nother {KEY_B}\nbox {KEY_A}\n")).unwrap();
        for trust in [false, true] {
            match judge_host_key(&kh, "box", 22, &key(KEY_B), trust).unwrap_err() {
                ConnectError::HostKeyChanged { key_type, .. } => {
                    assert_eq!(key_type, "ssh-ed25519")
                }
                other => panic!("{other:?}"),
            }
        }
        // Nothing was appended.
        assert_eq!(std::fs::read_to_string(&kh).unwrap().lines().count(), 3);
    }

    /// Against a real sshd: `YC_TEST_SSHD_PORT` (on 127.0.0.1) and
    /// `YC_TEST_SSH_KEY` (private key accepted for the current user).
    #[test]
    fn host_key_flow_against_live_sshd() {
        let (Ok(port), Ok(key_path)) = (
            std::env::var("YC_TEST_SSHD_PORT"),
            std::env::var("YC_TEST_SSH_KEY"),
        ) else {
            eprintln!("skipped: set YC_TEST_SSHD_PORT and YC_TEST_SSH_KEY");
            return;
        };
        let port: u16 = port.parse().unwrap();
        let user = std::env::var("USER").unwrap();
        let auth = SshAuth::Key {
            path: Some(PathBuf::from(key_path)),
            passphrase: None,
        };
        let kh = tmp("live").join("known_hosts");
        let connect = |trust: bool| {
            connect_checked(&kh, "127.0.0.1", port, &user, &auth, trust)
        };
        let fail = |trust: bool| match connect(trust) {
            Err(e) => e,
            Ok(_) => panic!("connect succeeded with trust={trust}"),
        };

        let fingerprint = match fail(false) {
            ConnectError::UnknownHostKey { fingerprint, .. } => fingerprint,
            other => panic!("{other:?}"),
        };
        assert!(!kh.exists());

        let conn = connect(true).unwrap_or_else(|e| panic!("{e}"));
        assert!(conn.home.is_absolute());
        drop(conn);
        let recorded = std::fs::read_to_string(&kh).unwrap();
        assert!(recorded.contains(&format!("[127.0.0.1]:{port} ssh-")), "{recorded}");
        let known = key(recorded.trim().split(' ').skip(1).collect::<Vec<_>>().join(" ").as_str());
        assert_eq!(known.fingerprint(HashAlg::Sha256).to_string(), fingerprint);

        connect(false).unwrap_or_else(|e| panic!("{e}"));

        // The server now presents a key other than the recorded one.
        std::fs::write(&kh, format!("[127.0.0.1]:{port} {KEY_B}\n")).unwrap();
        for trust in [false, true] {
            assert!(matches!(fail(trust), ConnectError::HostKeyChanged { .. }));
        }
    }

    /// Against a real sshd (`YC_TEST_SSHD_PORT`, `YC_TEST_SSH_KEY`) and a
    /// folder on it (`YC_TEST_SFTP_DIR`) holding `realdir/`, links `dl*`
    /// to it, broken links `broken*` and plain files `f*`.
    #[test]
    fn list_resolves_symlinks_against_live_sshd() {
        let (Ok(port), Ok(key_path), Ok(dir)) = (
            std::env::var("YC_TEST_SSHD_PORT"),
            std::env::var("YC_TEST_SSH_KEY"),
            std::env::var("YC_TEST_SFTP_DIR"),
        ) else {
            eprintln!("skipped: set YC_TEST_SSHD_PORT, YC_TEST_SSH_KEY, YC_TEST_SFTP_DIR");
            return;
        };
        let auth = SshAuth::Key {
            path: Some(PathBuf::from(key_path)),
            passphrase: None,
        };
        let kh = tmp("list").join("known_hosts");
        let user = std::env::var("USER").unwrap();
        let conn = connect_checked(&kh, "127.0.0.1", port.parse().unwrap(), &user, &auth, true)
            .unwrap_or_else(|e| panic!("{e}"));
        let t = std::time::Instant::now();
        let entries = conn.list(Path::new(&dir)).unwrap();
        eprintln!("listed {} entries in {:?}", entries.len(), t.elapsed());

        let local: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries.len(), local.len());
        for e in &entries {
            let n = e.name.as_str();
            if n.starts_with("dl") {
                assert!(e.is_symlink && e.is_dir, "{n}: link to a folder is enterable");
            } else if n.starts_with("broken") {
                assert!(e.is_symlink && !e.is_dir, "{n}: broken link keeps its own data");
            } else if n == "realdir" {
                assert!(!e.is_symlink && e.is_dir);
            } else {
                assert!(!e.is_symlink && !e.is_dir, "{n}");
            }
        }
    }

    #[test]
    fn connect_error_serializes_with_kind_tag() {
        let e = ConnectError::HostKeyChanged {
            key_type: "ssh-ed25519".into(),
            fingerprint: "SHA256:x".into(),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["kind"], "hostKeyChanged");
        assert_eq!(json["keyType"], "ssh-ed25519");
        assert_eq!(json["fingerprint"], "SHA256:x");
        let o: ConnectError = "boom".to_string().into();
        assert_eq!(serde_json::to_value(&o).unwrap()["message"], "boom");
    }
}
