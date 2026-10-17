//! Copies `memory.x` into `OUT_DIR` and passes the linker flags `cortex-m-rt`
//! needs. Based on the upstream `embassy-rp` `examples/rp235x/build.rs`.
//!
//! These link args live here, not in `.cargo/config.toml`'s `rustflags`, to
//! avoid passing `--nmagic`/`-Tlink.x` twice (once from each place would
//! duplicate the arguments on the linker command line).

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    File::create(out.join("memory.x")).unwrap().write_all(include_bytes!("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
}
