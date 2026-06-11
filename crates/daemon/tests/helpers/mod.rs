/// Minimal HTTP GET without pulling in a client crate: raw request over TCP.
/// `base` is e.g. `"http://127.0.0.1:PORT"` and `path` is e.g. `"/term/"`.
pub async fn http_get(base: &str, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = base.strip_prefix("http://").unwrap();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf);
    text.split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default()
}
