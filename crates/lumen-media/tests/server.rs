#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use lumen_media::MediaServer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Minimal HTTP/1.1 client so tests control Host and the raw request target.
async fn raw(
    addr: std::net::SocketAddr,
    method: &str,
    target: &str,
    host: &str,
    extra: &[(&str, &str)],
) -> Reply {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let split = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end");
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Reply {
        status,
        headers,
        body: buf[split + 4..].to_vec(),
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    server: MediaServer,
}

impl Fixture {
    async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("lib");
        fs::create_dir_all(root.join("Skool/G/01 - Curso")).unwrap();
        fs::write(root.join("Skool/G/01 - Curso/\u{c1}.mp4"), b"0123456789").unwrap();
        fs::write(root.join("Skool/G/d.html"), b"<p>x</p>").unwrap();
        fs::write(tmp.path().join("secret.txt"), b"secret").unwrap();
        let server = MediaServer::start(&root).await.unwrap();
        Self {
            _tmp: tmp,
            root,
            server,
        }
    }
    fn host(&self) -> String {
        self.server.addr().to_string()
    }
    fn token_path(&self) -> String {
        let base = self.server.base_url();
        base[base.find("/m/").unwrap()..].to_string()
    }
    async fn get(&self, rel: &str) -> Reply {
        let target = format!("{}/{}", self.token_path(), rel);
        raw(self.server.addr(), "GET", &target, &self.host(), &[]).await
    }
}

const VIDEO: &str = "Skool/G/01%20-%20Curso/%C3%81.mp4";

#[tokio::test]
async fn base_url_is_loopback_with_random_token_per_run() {
    let a = Fixture::new().await;
    let b = Fixture::new().await;
    let base = a.server.base_url();
    assert!(base.starts_with("http://127.0.0.1:"), "{base}");
    let token = base.rsplit('/').next().unwrap();
    assert_eq!(token.len(), 48);
    assert!(token.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(
        base.rsplit('/').next(),
        b.server.base_url().rsplit('/').next()
    );
    assert!(a.server.addr().ip().is_loopback());
}

#[tokio::test]
async fn serves_file_with_security_headers_and_no_cors() {
    let f = Fixture::new().await;
    let r = f.get(VIDEO).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"0123456789");
    assert_eq!(r.header("content-type"), Some("video/mp4"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert_eq!(r.header("accept-ranges"), Some("bytes"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert!(r
        .headers
        .iter()
        .all(|(k, _)| !k.to_ascii_lowercase().starts_with("access-control-")));
}

#[tokio::test]
async fn html_is_served_without_script_execution() {
    let f = Fixture::new().await;
    let r = f.get("Skool/G/d.html").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("content-security-policy"),
        Some("script-src 'none'")
    );
    assert_eq!(r.header("content-type"), Some("text/html; charset=utf-8"));
}

#[tokio::test]
async fn range_requests_return_partial_content() {
    let f = Fixture::new().await;
    let target = format!("{}/{}", f.token_path(), VIDEO);
    let ask = |range: &'static str| {
        let t = target.clone();
        let f = &f;
        async move { raw(f.server.addr(), "GET", &t, &f.host(), &[("Range", range)]).await }
    };
    let r = ask("bytes=2-5").await;
    assert_eq!(r.status, 206);
    assert_eq!(r.body, b"2345");
    assert_eq!(r.header("content-range"), Some("bytes 2-5/10"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    let r = ask("bytes=-3").await;
    assert_eq!((r.status, r.body.as_slice()), (206, &b"789"[..]));
    let r = ask("bytes=7-").await;
    assert_eq!((r.status, r.body.as_slice()), (206, &b"789"[..]));
    let r = ask("bytes=50-").await;
    assert_eq!(r.status, 416);
}

#[tokio::test]
async fn head_returns_headers_without_body() {
    let f = Fixture::new().await;
    let target = format!("{}/{}", f.token_path(), VIDEO);
    let r = raw(f.server.addr(), "HEAD", &target, &f.host(), &[]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.header("content-length"), Some("10"));
    assert!(r.body.is_empty());
}

#[tokio::test]
async fn wrong_host_is_forbidden_to_stop_dns_rebinding() {
    let f = Fixture::new().await;
    let target = format!("{}/{}", f.token_path(), VIDEO);
    let port = f.server.addr().port();
    for host in [
        "evil.example".to_string(),
        format!("localhost:{port}"),
        format!("evil.example:{port}"),
        "127.0.0.1".to_string(),
        format!("127.0.0.1:{}", port.wrapping_add(1)),
    ] {
        let r = raw(f.server.addr(), "GET", &target, &host, &[]).await;
        assert_eq!(r.status, 403, "{host}");
        assert!(r.body.is_empty() || !String::from_utf8_lossy(&r.body).contains("0123"));
    }
}

#[tokio::test]
async fn missing_or_wrong_token_is_not_found() {
    let f = Fixture::new().await;
    let host = f.host();
    let a = f.server.addr();
    let tok = f.token_path();
    let wrong = format!("{}0", tok);
    for target in [
        format!("/{VIDEO}"),
        format!("/m/{VIDEO}"),
        format!("/m/{}/{VIDEO}", "0".repeat(48)),
        format!("{wrong}/{VIDEO}"),
        tok.clone(),
        format!("{tok}/"),
    ] {
        assert_eq!(
            raw(a, "GET", &target, &host, &[]).await.status,
            404,
            "{target}"
        );
    }
}

#[tokio::test]
async fn only_get_and_head_are_allowed() {
    let f = Fixture::new().await;
    let target = format!("{}/{}", f.token_path(), VIDEO);
    for m in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
        let r = raw(
            f.server.addr(),
            m,
            &target,
            &f.host(),
            &[("Origin", "http://evil.example")],
        )
        .await;
        assert_eq!(r.status, 405, "{m}");
        assert_eq!(r.header("allow"), Some("GET, HEAD"));
        assert!(r.header("access-control-allow-origin").is_none());
    }
}

#[tokio::test]
async fn traversal_attempts_never_reach_files_outside_root() {
    let f = Fixture::new().await;
    for rel in [
        "../secret.txt",
        "..%2fsecret.txt",
        "%2e%2e/secret.txt",
        "%2e%2e%2fsecret.txt",
        "Skool/../../secret.txt",
        "Skool/G/..%5c..%5c..%5csecret.txt",
        "Skool%00/x",
        "%2fsecret.txt",
        "Skool/G/%2e%2e/%2e%2e/%2e%2e/secret.txt",
    ] {
        let r = f.get(rel).await;
        assert_ne!(r.status, 200, "{rel}");
        assert!(
            !String::from_utf8_lossy(&r.body).contains("secret"),
            "{rel}"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_and_sibling_prefix_are_blocked() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new().await;
    let parent = f.root.parent().unwrap();
    fs::create_dir_all(parent.join("lib-evil")).unwrap();
    fs::write(parent.join("lib-evil/secret.txt"), b"evil").unwrap();
    symlink(parent.join("secret.txt"), f.root.join("out.txt")).unwrap();
    symlink(parent.join("lib-evil"), f.root.join("evil")).unwrap();
    for rel in ["out.txt", "evil/secret.txt"] {
        assert_eq!(f.get(rel).await.status, 404, "{rel}");
    }
}

#[tokio::test]
async fn directories_are_not_served() {
    let f = Fixture::new().await;
    assert_eq!(f.get("Skool/G").await.status, 404);
    assert_eq!(f.get("Skool/G/").await.status, 404);
}

#[tokio::test]
async fn root_can_be_changed_at_runtime() {
    let f = Fixture::new().await;
    let other = tempfile::tempdir().unwrap();
    fs::write(other.path().join("x.txt"), b"other").unwrap();
    assert_eq!(f.get("x.txt").await.status, 404);
    f.server.set_root(other.path());
    let r = f.get("x.txt").await;
    assert_eq!((r.status, r.body.as_slice()), (200, &b"other"[..]));
    assert_eq!(f.get(VIDEO).await.status, 404);
}

#[tokio::test]
async fn shutdown_stops_accepting_connections() {
    let f = Fixture::new().await;
    let addr = f.server.addr();
    assert_eq!(f.get(VIDEO).await.status, 200);
    f.server.shutdown().await;
    assert!(TcpStream::connect(addr).await.is_err());
}

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

async fn thumb_fixture() -> (Fixture, std::path::PathBuf) {
    let f = Fixture::new().await;
    let dir = f._tmp.path().join("thumbs");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{HASH}.jpg")), b"\xff\xd8jpeg").unwrap();
    f.server.set_thumb_dir(&dir);
    (f, dir)
}

fn thumb_target(f: &Fixture, name: &str) -> String {
    let base = f.server.thumb_base_url();
    let path = base.split_once("127.0.0.1:").unwrap().1;
    let path = &path[path.find('/').unwrap()..];
    format!("{path}/{name}")
}

#[tokio::test]
async fn thumbnails_are_served_from_the_cache_dir_with_safe_headers() {
    let (f, _dir) = thumb_fixture().await;
    assert!(f.server.thumb_base_url().contains("/t/"));
    let r = raw(
        f.server.addr(),
        "GET",
        &thumb_target(&f, &format!("{HASH}.jpg")),
        &f.server.addr().to_string(),
        &[],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"\xff\xd8jpeg");
    assert_eq!(r.header("content-type"), Some("image/jpeg"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert!(r.header("access-control-allow-origin").is_none());
    assert!(r.header("cache-control").unwrap().contains("max-age"));
    let head = raw(
        f.server.addr(),
        "HEAD",
        &thumb_target(&f, &format!("{HASH}.jpg")),
        &f.server.addr().to_string(),
        &[],
    )
    .await;
    assert_eq!((head.status, head.body.len()), (200, 0));
}

#[tokio::test]
async fn thumbnail_requests_need_host_token_method_and_a_plain_cache_file_name() {
    let (f, dir) = thumb_fixture().await;
    let host = f.server.addr().to_string();
    let good = thumb_target(&f, &format!("{HASH}.jpg"));
    assert_eq!(
        raw(f.server.addr(), "GET", &good, "evil.test", &[])
            .await
            .status,
        403
    );
    let wrong_token = good.replacen("/t/", "/t/x", 1);
    assert_eq!(
        raw(f.server.addr(), "GET", &wrong_token, &host, &[])
            .await
            .status,
        404
    );
    assert_eq!(
        raw(f.server.addr(), "POST", &good, &host, &[]).await.status,
        405
    );
    fs::write(dir.join("notahash.jpg"), "x").unwrap();
    fs::write(dir.join(format!("{HASH}.png")), "x").unwrap();
    fs::write(dir.join(format!("{HASH}.part.jpg")), "x").unwrap();
    fs::write(f._tmp.path().join("secret.jpg"), "secret").unwrap();
    for name in [
        "notahash.jpg".to_string(),
        format!("{HASH}.png"),
        format!("{HASH}.part.jpg"),
        "..%2Fsecret.jpg".to_string(),
        "%2e%2e/secret.jpg".to_string(),
        format!("sub/{HASH}.jpg"),
        String::new(),
        HASH.to_uppercase() + ".jpg",
    ] {
        let r = raw(f.server.addr(), "GET", &thumb_target(&f, &name), &host, &[]).await;
        assert_eq!(r.status, 404, "{name:?}");
        assert!(!String::from_utf8_lossy(&r.body).contains("secret"));
    }
    // The media route does not serve the cache and the thumbnail route does not serve the library.
    let media = format!("{}/thumbs/{HASH}.jpg", f.token_path());
    assert_eq!(
        raw(f.server.addr(), "GET", &media, &host, &[]).await.status,
        404
    );
    let lib_via_thumbs = thumb_target(&f, "a%20b/v.mp4");
    assert_eq!(
        raw(f.server.addr(), "GET", &lib_via_thumbs, &host, &[])
            .await
            .status,
        404
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_thumbnail_is_never_followed() {
    let (f, dir) = thumb_fixture().await;
    let other = "f".repeat(64);
    fs::write(f._tmp.path().join("outside.jpg"), "outside").unwrap();
    std::os::unix::fs::symlink(
        f._tmp.path().join("outside.jpg"),
        dir.join(format!("{other}.jpg")),
    )
    .unwrap();
    let r = raw(
        f.server.addr(),
        "GET",
        &thumb_target(&f, &format!("{other}.jpg")),
        &f.server.addr().to_string(),
        &[],
    )
    .await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn without_a_thumbnail_dir_the_route_is_empty() {
    let f = Fixture::new().await;
    let r = raw(
        f.server.addr(),
        "GET",
        &thumb_target(&f, &format!("{HASH}.jpg")),
        &f.server.addr().to_string(),
        &[],
    )
    .await;
    assert_eq!(r.status, 404);
}
