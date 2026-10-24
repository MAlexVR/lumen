//! Manual check: serves `<root>` on the loopback media server for N seconds and
//! prints its base URL. Usage: `serve <root> [seconds]`.

use std::time::Duration;

use lumen_media::MediaServer;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = args.next().ok_or("usage: serve <root> [seconds]")?;
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(60);
    let server = MediaServer::start(root).await?;
    println!("{}", server.base_url());
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    server.shutdown().await;
    Ok(())
}
