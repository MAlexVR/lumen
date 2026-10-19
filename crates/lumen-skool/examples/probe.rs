//! Manual live probe (hits the network). Reads the cookie header from the
//! `SKOOL_COOKIE` environment variable only; never reads files. Lists groups
//! and, with a slug argument, the courses of that group. Downloads nothing.
//!
//!     SKOOL_COOKIE='auth_token=...; ...' cargo run -p lumen-skool --example probe [group-slug]

use lumen_skool::client::SkoolClient;
use lumen_skool::session::normalize_cookie_header;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = std::env::var("SKOOL_COOKIE")
        .map_err(|_| "set SKOOL_COOKIE to the browser cookie header")?;
    let client = SkoolClient::new(&normalize_cookie_header(&raw)?)?;
    let groups = client.groups().await?;
    for g in &groups {
        println!("{} ({} courses): {}", g.slug, g.num_courses, g.name);
    }
    if let Some(slug) = std::env::args().nth(1) {
        let listing = client.classroom(&slug).await?;
        for c in &listing.courses {
            println!(
                "  {} - {} ({} modules)",
                c.short_name, c.title, c.num_modules
            );
        }
    }
    Ok(())
}
