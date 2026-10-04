//! Baboon, a Halo tag editor: the application library behind the `Baboon`
//! binary and the build tools in `src/bin`.
//! It holds the module tree; `main.rs` only calls [`run`], and process
//! startup lives in `app::shell::startup`.

/// `include_str!` with a path from the package root, so a file can move
/// without its includes changing.
macro_rules! include_root_str {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path))
    };
}

/// `include_bytes!` with a path from the package root; see `include_root_str!`.
macro_rules! include_root_bytes {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path))
    };
}

mod app;
mod core;
#[cfg(test)]
mod test_kits;

/// Start the application and run it until its window closes.
pub fn run() -> anyhow::Result<()> {
    app::run()
}
