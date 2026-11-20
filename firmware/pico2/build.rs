// Build script for the `pico2` crate, run by cargo on the host before the
// crate is compiled. It only prints directives to stdout, which cargo reads:
//
// * `cargo:rustc-link-search=<dir>` adds this crate's directory
//   (`firmware/pico2/`) to the linker's search path. The application's
//   `.cargo/config.toml` (the one `templates/pico2` generates) passes
//   `-C link-arg=-Tlink.ld` to every link, and the linker resolves that bare
//   filename against the search path — this is what lets `link.ld` live next
//   to this crate instead of in every application. Search paths from a
//   dependency's build script also apply when the final binary (the
//   application) is linked.
// * `cargo:rerun-if-changed=link.ld` makes cargo rerun this script — and so
//   relink — when the linker script changes; otherwise edits to `link.ld`
//   would not take effect until some Rust source changed too.
fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-search={dir}");
    println!("cargo:rerun-if-changed=link.ld");
}
