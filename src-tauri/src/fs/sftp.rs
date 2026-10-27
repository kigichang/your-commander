use super::provider::FsEntry;
use crate::ops::queue::{cancelled_err, OpCtx};
use russh::client;
use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::OpenFlags;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const CHUNK: usize = 256 * 1024;

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

struct Handler;

impl client::Handler for Handler {
    type Error = russh::Error;

    // Personal tool: trust the server key (no known_hosts verification).
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(true)
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

/// Blocking connect; call from a worker thread.
pub fn connect(
    host: &str,
    port: u16,
    user: &str,
    auth: &SshAuth,
) -> Result<Arc<SftpConnection>, String> {
    runtime().block_on(async {
        let config = Arc::new(client::Config::default());
        let mut handle = client::connect(config, (host, port), Handler)
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
    })
}

fn rp(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

impl SftpConnection {
    pub fn list(&self, path: &Path) -> std::io::Result<Vec<FsEntry>> {
        runtime().block_on(async {
            let dir = self
                .session
                .read_dir(rp(path))
                .await
                .map_err(std::io::Error::other)?;
            let mut out = Vec::new();
            for entry in dir {
                let name = entry.file_name();
                let meta = entry.metadata();
                let is_dir = meta.is_dir();
                let is_symlink = meta.is_symlink();
                let full = path.join(&name);
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
    pub fn exists(&self, path: &Path) -> Option<bool> {
        runtime().block_on(async {
            self.session.metadata(rp(path)).await.ok().map(|m| m.is_dir())
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
        let is_dir = self.exists(path).ok_or_else(|| {
            format!("{}: not found", path.display())
        })?;
        if is_dir {
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

