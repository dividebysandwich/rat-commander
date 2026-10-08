//! FTP backend over suppaftp (tokio). The single control connection is shared
//! behind an async mutex; both downloads and uploads stream through a duplex
//! pipe so transfers proceed at network speed with bounded memory and accurate
//! progress.
//!
//! FTPS is the same connection secured with `AUTH TLS` before logging in (the
//! data connections follow, resuming its TLS session), its certificate checked
//! as [`super::tls`] describes.

use super::{Connection, Protocol, RemoteCreds, parse_unix_listing_line, tls};
use crate::util::{Error, Result};
use crate::vfs::membuf::{pipe_download, pipe_upload};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use std::sync::Arc;
use suppaftp::Mode;
use suppaftp::tokio::{AsyncRustlsConnector, AsyncRustlsFtpStream};
use suppaftp::types::FileType;
use tokio::sync::Mutex;

pub struct FtpFs {
    conn: Arc<Mutex<AsyncRustlsFtpStream>>,
    secure: bool,
}

pub async fn connect(creds: &RemoteCreds) -> Result<Connection> {
    let host_port = format!("{}:{}", creds.host, creds.port);
    let (roots, pin) = match creds.protocol {
        Protocol::Ftps => {
            (tls::system_roots(), tls::pins_file().and_then(|p| tls::pinned(&p, &host_port)))
        }
        _ => (suppaftp::tokio_rustls::rustls::RootCertStore::empty(), None),
    };
    connect_trusting(creds, roots, pin).await
}

/// [`connect`], an FTPS certificate trusted when `roots` vouch for it or its
/// SHA-256 is `pin`.
async fn connect_trusting(
    creds: &RemoteCreds,
    roots: suppaftp::tokio_rustls::rustls::RootCertStore,
    pin: Option<String>,
) -> Result<Connection> {
    let mut stream = AsyncRustlsFtpStream::connect((creds.host.as_str(), creds.port))
        .await
        .map_err(|e| Error::other(format!("FTP connect failed: {e}")))?;
    let secure = creds.protocol == Protocol::Ftps;
    if secure {
        let host_port = format!("{}:{}", creds.host, creds.port);
        let (config, failure) = tls::client_config(&host_port, roots, pin)?;
        let connector =
            AsyncRustlsConnector::from(suppaftp::tokio_rustls::TlsConnector::from(config));
        stream = match stream.into_secure(connector, &creds.host).await {
            Ok(s) => s,
            Err(e) => {
                let stopped_at = failure.lock().unwrap_or_else(|p| p.into_inner()).take();
                return Err(match stopped_at {
                    Some(f) => Error::UntrustedCertificate(Box::new(f)),
                    None => Error::other(format!("FTPS: TLS failed: {e}")),
                });
            }
        };
    }
    stream
        .login(&creds.user, &creds.password)
        .await
        .map_err(|e| Error::other(format!("FTP login failed: {e}")))?;
    // Passive mode (PASV) has the client open the data connection — the default,
    // and what works behind NAT/firewalls; active has the server connect back.
    stream.set_mode(if creds.passive { Mode::Passive } else { Mode::Active });
    // Binary mode so SIZE and transfers are byte-accurate. ASCII mode would
    // corrupt binary files (and skew SIZE), so a server that rejects TYPE I is a
    // hard error rather than a silent fall-back.
    stream
        .transfer_type(FileType::Binary)
        .await
        .map_err(|e| Error::other(format!("FTP: cannot set binary mode: {e}")))?;

    let root = if creds.path.trim().is_empty() {
        stream.pwd().await.unwrap_or_else(|_| "/".to_string())
    } else {
        creds.path.clone()
    };
    let label = format!("{}://{}@{}", creds.protocol.scheme_prefix(), creds.user, creds.host);
    let backend = Arc::new(FtpFs { conn: Arc::new(Mutex::new(stream)), secure });
    Ok(Connection { backend, root, label })
}

fn path_str(p: &VfsPath) -> String {
    p.posix_path()
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[async_trait::async_trait]
impl Vfs for FtpFs {
    fn scheme(&self) -> &str {
        if self.secure { "ftps" } else { "ftp" }
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: true,
            permissions: false,
            ownership: false,
            symlinks: false,
            random_access: false,
            inode: false,
            server_rename: true,
            atomic_write: false,
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let mut guard = self.conn.lock().await;
        let lines =
            guard.list(Some(&path_str(dir))).await.map_err(|e| Error::other(e.to_string()))?;
        let mut out = Vec::new();
        for line in lines {
            if let Some(p) = parse_unix_listing_line(&line) {
                out.push(VfsEntry {
                    name: p.name,
                    kind: p.kind,
                    size: p.size,
                    mtime: None,
                    atime: None,
                    ctime: None,
                    btime: None,
                    inode: None,
                    mode: p.mode,
                    uid: None,
                    gid: None,
                    symlink_target: p.symlink_target,
                    symlink_broken: false,
                    symlink_dir: false,
                });
            }
        }
        Ok(out)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let mut guard = self.conn.lock().await;
        // FTP has no stat; SIZE succeeds for files, fails for directories.
        match guard.size(&path_str(path)).await {
            Ok(size) => Ok(VfsEntry {
                name: path.file_name(),
                kind: VfsKind::File,
                size: size as u64,
                mtime: None,
                atime: None,
                ctime: None,
                btime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
                symlink_dir: false,
            }),
            Err(_) => Ok(VfsEntry {
                name: path.file_name(),
                kind: VfsKind::Dir,
                size: 0,
                mtime: None,
                atime: None,
                ctime: None,
                btime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
                symlink_dir: false,
            }),
        }
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        // Stream the data connection straight into the read pipe (holding the
        // control-connection lock for the transfer's duration), so large files
        // download chunk-by-chunk at network speed instead of buffering in RAM.
        let conn = self.conn.clone();
        let path = path_str(path);
        Ok(pipe_download(64 * 1024, move |mut w| async move {
            let mut guard = conn.lock().await;
            let mut stream = guard.retr_as_stream(&path).await.map_err(io_err)?;
            tokio::io::copy(&mut stream, &mut w).await?;
            stream.finish().await.map_err(io_err)?;
            Ok(())
        }))
    }

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let conn = self.conn.clone();
        let path = path_str(path);
        // Stream the engine's bytes straight to the FTP data connection; the
        // write side blocks at network speed, so progress tracks the upload.
        Ok(pipe_upload(64 * 1024, move |mut rx| async move {
            let mut guard = conn.lock().await;
            let mut stream = guard.put_with_stream(&path).await.map_err(io_err)?;
            tokio::io::copy(&mut rx, &mut stream).await?;
            stream.finish().await.map_err(io_err)?;
            Ok(())
        }))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        self.conn.lock().await.mkdir(path_str(path)).await.map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.conn.lock().await.rm(path_str(path)).await.map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.conn.lock().await.rmdir(path_str(path)).await.map_err(|e| Error::other(e.to_string()))
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.conn
            .lock()
            .await
            .rename(path_str(from), path_str(to))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::certs::{pem_blocks, testdata};
    use suppaftp::tokio_rustls::rustls;
    use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

    /// Answer FTP commands on `stream` until it closes: the replies a login,
    /// TLS protection, TYPE I and PWD need. Returns on AUTH TLS, for the
    /// caller to secure the connection.
    async fn converse<S: AsyncRead + AsyncWrite + Unpin>(stream: S, greet: bool) -> Option<S> {
        let mut io = BufReader::new(stream);
        if greet {
            io.get_mut().write_all(b"220 ready\r\n").await.ok()?;
        }
        let mut line = String::new();
        loop {
            line.clear();
            if io.read_line(&mut line).await.ok()? == 0 {
                return None;
            }
            let cmd = line.trim_end().to_ascii_uppercase();
            let reply: &[u8] = match cmd.split(' ').next().unwrap_or("") {
                "AUTH" => {
                    io.get_mut().write_all(b"234 go ahead\r\n").await.ok()?;
                    return Some(io.into_inner());
                }
                "PBSZ" | "PROT" | "TYPE" => b"200 ok\r\n",
                "USER" => b"331 password please\r\n",
                "PASS" => b"230 logged in\r\n",
                "PWD" => b"257 \"/srv\" is the directory\r\n",
                _ => b"502 not here\r\n",
            };
            io.get_mut().write_all(reply).await.ok()?;
        }
    }

    /// An FTP server on a throwaway port that switches to TLS with the test
    /// RSA certificate when asked.
    async fn ftps_server() -> u16 {
        let cert = pem_blocks(testdata::RSA_CERT).pop().unwrap().der;
        let key = pem_blocks(testdata::RSA_KEY).pop().unwrap().der;
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert)],
                rustls::pki_types::PrivateKeyDer::Pkcs1(key.into()),
            )
            .unwrap();
        let acceptor = suppaftp::tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Some(tcp) = converse(tcp, true).await else { return };
                    if let Ok(tls) = acceptor.accept(tcp).await {
                        converse(tls, false).await;
                    }
                });
            }
        });
        port
    }

    fn creds(port: u16) -> RemoteCreds {
        RemoteCreds {
            protocol: Protocol::Ftps,
            host: "127.0.0.1".into(),
            port,
            user: "u".into(),
            password: "p".into(),
            path: String::new(),
            passive: true,
            key_file: String::new(),
            key_passphrase: String::new(),
        }
    }

    #[tokio::test]
    async fn an_untrusted_ftps_server_asks_first_and_connects_once_pinned() {
        let port = ftps_server().await;
        let failure =
            match connect_trusting(&creds(port), rustls::RootCertStore::empty(), None).await {
                Err(Error::UntrustedCertificate(f)) => f,
                Err(e) => panic!("expected an untrusted certificate, got {e}"),
                Ok(_) => panic!("a self-signed certificate must not be accepted silently"),
            };
        assert_eq!(failure.host_port, format!("127.0.0.1:{port}"));
        assert_eq!(failure.subject, "CN=rsa.example.test");

        let conn = connect_trusting(
            &creds(port),
            rustls::RootCertStore::empty(),
            Some(failure.sha256.clone()),
        )
        .await
        .expect("the pinned certificate connects");
        assert_eq!(conn.root, "/srv");
        assert_eq!(conn.label, "ftps://u@127.0.0.1");
        assert_eq!(conn.backend.scheme(), "ftps");

        let other = "00".repeat(32);
        match connect_trusting(&creds(port), rustls::RootCertStore::empty(), Some(other)).await {
            Err(Error::UntrustedCertificate(f)) => assert!(f.pinned_other),
            _ => panic!("a different pin must not connect"),
        }
    }
}
