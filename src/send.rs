//! LAN "Send file" support: pick a LAN address to advertise, then run a tiny
//! ephemeral HTTP/1.1 server that streams one prepared file to whatever device
//! scans the QR code.
//!
//! The server binds `0.0.0.0` on a free port so it is reachable from the whole
//! subnet, but the URL (and QR) advertise a concrete LAN IP so a phone can find
//! it. It lives only while the Send dialog is open — [`AppState`] aborts the
//! accept loop when the dialog closes — and it reports every completed download
//! back over the [`AppEvent`] channel so the dialog can show a running count.
//!
//! [`AppState`]: crate::app::state::AppState

use crate::app::event::AppEvent;
use crate::util::async_bridge::AppSender;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// A running send server plus the bookkeeping needed to tear it down.
pub struct SendServer {
    /// The accept loop; aborted on [`shutdown`](SendServer::shutdown).
    handle: JoinHandle<()>,
    /// A temporary archive to delete when the dialog closes — the zip we built
    /// for a multi-file / directory selection. `None` for a lone file served in
    /// place (never deleted).
    temp: Option<PathBuf>,
}

impl SendServer {
    /// Stop serving: abort the accept loop and remove any temporary archive.
    pub fn shutdown(self) {
        self.handle.abort();
        if let Some(t) = self.temp {
            let _ = std::fs::remove_file(t);
        }
    }
}

/// Start serving `path` (advertised to browsers as `download_name`), deleting
/// `temp` when the server is later shut down. Binds `0.0.0.0:0` — all interfaces
/// on a free port — so the chosen port is known synchronously for the URL. Every
/// completed GET download emits [`AppEvent::FileSent`] on `tx`.
pub fn start(
    path: PathBuf,
    download_name: String,
    temp: Option<PathBuf>,
    tx: AppSender,
) -> std::io::Result<(u16, SendServer)> {
    // Bind with std so the port is available before we return (converting to a
    // tokio listener needs a running reactor, which the caller is inside).
    let std_listener = std::net::TcpListener::bind(("0.0.0.0", 0))?;
    std_listener.set_nonblocking(true)?;
    let port = std_listener.local_addr()?.port();
    let listener = TcpListener::from_std(std_listener)?;
    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _peer)) = listener.accept().await else {
                break;
            };
            let path = path.clone();
            let name = download_name.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Ok(true) = serve_one(stream, &path, &name).await {
                    let _ = tx.send(AppEvent::FileSent).await;
                }
            });
        }
    });
    Ok((port, SendServer { handle, temp }))
}

/// Handle one connection: read the request head, then stream the file as a plain
/// `200 OK` download (Range is ignored — always the whole file). Returns
/// `Ok(true)` when a GET body was fully written (a completed download), `Ok(false)`
/// for a HEAD / favicon probe / 404, and `Err` on a socket failure.
async fn serve_one(mut stream: TcpStream, path: &Path, name: &str) -> std::io::Result<bool> {
    // Read up to the blank line that ends the request headers (with a cap so a
    // client that never sends one can't grow this unbounded).
    let mut req = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        req.extend_from_slice(&buf[..n]);
        if req.windows(4).any(|w| w == b"\r\n\r\n") || req.len() > 64 * 1024 {
            break;
        }
    }
    let head = String::from_utf8_lossy(&req);
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let is_head = method.eq_ignore_ascii_case("HEAD");

    // A browser auto-fetches /favicon.ico; 404 it so it isn't counted as a
    // download of the file.
    if target == "/favicon.ico" {
        write_status(&mut stream, "404 Not Found", b"Not found").await?;
        return Ok(false);
    }

    let mut file = match tokio::fs::File::open(path).await {
        Ok(f) => f,
        Err(_) => {
            write_status(&mut stream, "404 Not Found", b"Not found").await?;
            return Ok(false);
        }
    };
    let len = file.metadata().await?.len();
    let header = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {len}\r\n\
         Content-Disposition: attachment; filename=\"{}\"\r\n\
         Accept-Ranges: none\r\n\
         Connection: close\r\n\
         \r\n",
        header_filename(name)
    );
    stream.write_all(header.as_bytes()).await?;
    if is_head {
        stream.flush().await?;
        return Ok(false);
    }
    let mut chunk = vec![0u8; 128 * 1024];
    loop {
        let n = file.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        stream.write_all(&chunk[..n]).await?;
    }
    stream.flush().await?;
    Ok(true)
}

/// Write a tiny `text/plain` status response (used for 404s).
async fn write_status(stream: &mut TcpStream, status: &str, body: &[u8]) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(resp.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await
}

/// Sanitize a download name for a `Content-Disposition` header value: keep only
/// the base name and drop quotes / backslashes / control characters that would
/// break the header or let a name escape into a path.
fn header_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.chars().filter(|c| !c.is_control() && *c != '"' && *c != '\\').collect()
}

/// Percent-encode a file's base name for use in a URL path.
fn url_encode(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut out = String::with_capacity(base.len());
    for b in base.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The download URL to advertise (and encode in the QR): `http://<ip>:<port>/<name>`,
/// with the name percent-encoded and IPv6 addresses bracketed.
pub fn url_for(ip: IpAddr, port: u16, name: &str) -> String {
    let host = match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    };
    format!("http://{host}:{port}/{}", url_encode(name))
}

/// Pick a LAN IPv4 address to advertise. Prefers a private (RFC1918) address on
/// a non-loopback interface — the phone and this machine are almost always on the
/// same private subnet — then any routable IPv4, then the kernel's source address
/// for an outbound route, and finally loopback (still works on the same host).
pub fn lan_ip() -> IpAddr {
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        let v4: Vec<Ipv4Addr> = ifaces
            .iter()
            .filter(|i| !i.is_loopback())
            .filter_map(|i| match i.ip() {
                IpAddr::V4(v4) if !v4.is_link_local() && !v4.is_unspecified() => Some(v4),
                _ => None,
            })
            .collect();
        if let Some(ip) = v4.iter().find(|ip| ip.is_private()).copied() {
            return IpAddr::V4(ip);
        }
        if let Some(ip) = v4.first().copied() {
            return IpAddr::V4(ip);
        }
    }
    // UDP-route trick: no packet is sent — the OS just resolves the source
    // address it would route toward a public IP, i.e. the active LAN interface.
    if let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0))
        && sock.connect(("8.8.8.8", 80)).is_ok()
        && let Ok(local) = sock.local_addr()
        && !local.ip().is_loopback()
    {
        return local.ip();
    }
    IpAddr::V4(Ipv4Addr::LOCALHOST)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn url_for_formats_ipv4_and_ipv6_and_encodes_name() {
        let u = url_for(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7)), 8080, "my file.zip");
        assert_eq!(u, "http://192.168.1.7:8080/my%20file.zip");
        let u6 = url_for(IpAddr::V6(Ipv6Addr::LOCALHOST), 9000, "a.txt");
        assert_eq!(u6, "http://[::1]:9000/a.txt");
    }

    #[test]
    fn url_encode_keeps_base_name_and_escapes_specials() {
        assert_eq!(url_encode("/tmp/dir/Report (final).pdf"), "Report%20%28final%29.pdf");
        assert_eq!(url_encode("plain-name_1.0.tar.gz"), "plain-name_1.0.tar.gz");
    }

    #[test]
    fn header_filename_strips_dangerous_chars() {
        // Path separators reduce to the base name; a stray quote is dropped.
        assert_eq!(header_filename("../etc/pass\"wd"), "passwd");
        assert_eq!(header_filename("a\\b\\c.txt"), "c.txt");
        assert_eq!(header_filename("photo.jpg"), "photo.jpg");
    }

    #[test]
    fn lan_ip_is_never_unspecified() {
        // Whatever the environment, we always advertise a concrete address.
        let ip = lan_ip();
        assert!(!ip.is_unspecified());
    }

    #[tokio::test]
    async fn serves_the_file_and_reports_completion() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A temp file with known contents.
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rc-send-test-{}.bin", std::process::id()));
        let body = b"hello over the LAN \xff\x00 binary";
        tokio::fs::write(&path, body).await.unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let (port, server) = start(path.clone(), "gift.bin".into(), None, tx).unwrap();

        // Fetch it like a browser would.
        let mut sock = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        sock.write_all(b"GET /gift.bin HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await.unwrap();

        let split = resp.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&resp[..split]);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "status: {head}");
        assert!(head.contains("Content-Disposition: attachment; filename=\"gift.bin\""));
        assert_eq!(&resp[split + 4..], body, "body streamed verbatim");

        // A completed download is reported back.
        let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("event arrives")
            .expect("channel open");
        assert!(matches!(evt, AppEvent::FileSent));

        server.shutdown();
        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn favicon_probe_is_not_counted() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rc-send-fav-{}.bin", std::process::id()));
        tokio::fs::write(&path, b"data").await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let (port, server) = start(path.clone(), "d.bin".into(), None, tx).unwrap();

        let mut sock = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        sock.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).await.unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 404"));

        // No FileSent should arrive for the favicon.
        let quiet = tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await;
        assert!(quiet.is_err(), "favicon must not count as a download");

        server.shutdown();
        let _ = tokio::fs::remove_file(&path).await;
    }
}
