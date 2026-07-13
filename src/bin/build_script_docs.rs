#[path = "../script_docs_import.rs"]
mod script_docs_import;

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let source = args.next().map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("usage: build_script_docs <script_test directory> [output sqlite3]");
        std::process::exit(2);
    });
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docs/script_docs.sqlite3"));
    if let Err(error) = script_docs_import::build_database(&source, &output) {
        eprintln!("script documentation import failed: {error}");
        std::process::exit(1);
    }
    println!("wrote {}", output.display());
}
