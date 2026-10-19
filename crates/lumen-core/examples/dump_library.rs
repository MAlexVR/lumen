//! Prints the scanned library as JSON (the same shape the `library_scan`
//! command returns). Used to regenerate the UI preview fixture:
//! `cargo run -p lumen-core --example dump_library -- <root>`

use std::process::ExitCode;

use lumen_core::LibraryScanner;

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: dump_library <library-root>");
        return ExitCode::FAILURE;
    };
    let library = match LibraryScanner::new(root).scan() {
        Ok(library) => library,
        Err(err) => {
            eprintln!("scan failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    match serde_json::to_string_pretty(&library) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cannot serialize: {err}");
            ExitCode::FAILURE
        }
    }
}
