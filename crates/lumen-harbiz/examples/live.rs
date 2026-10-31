//! Live smoke test against Harbiz (read-only): `cargo run -p lumen-harbiz --example live -- <creds-file>`
//! where the file has `user=<email>` and `password=<secret>` lines. Prints counts only.

use std::sync::Arc;

use lumen_harbiz::{login, HarbizApi, HarbizClient, Login, WsConnector};

fn field(contents: &str, key: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().to_string())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: live <creds-file>")?;
    let contents = std::fs::read_to_string(path)?;
    let input = Login::new(
        &field(&contents, "user").ok_or("no user= line")?,
        &field(&contents, "password").ok_or("no password= line")?,
    )?;
    let connector = Arc::new(WsConnector::default());
    let session = login(connector.as_ref(), &input).await?;
    println!("login ok");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64;
    let parsed = HarbizClient::new(connector, session).workouts(now).await?;
    let done = parsed.workouts.iter().filter(|w| w.completed).count();
    println!(
        "workouts={} completed={} photoCheckins={} unsupported={}",
        parsed.workouts.len(),
        done,
        parsed.photo_checkins,
        parsed.unsupported_events
    );
    if let (Some(first), Some(last)) = (parsed.workouts.first(), parsed.workouts.last()) {
        println!("range {} -> {}", first.date, last.date);
    }
    Ok(())
}
