//! Locating and building the firmware crate.
//!
//! The firmware lives in its own Cargo workspace (`firmware/`, excluded from
//! the root workspace because of the `no_std`/custom-target split -- see the
//! comment in `firmware/Cargo.toml`), so building it means shelling out to a
//! second `cargo build` with `--manifest-path firmware/Cargo.toml`.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

/// Rust target triple the firmware is built for.
pub const FIRMWARE_TARGET: &str = "thumbv8m.main-none-eabihf";
/// `[package] name` in `firmware/Cargo.toml`, and thus the default binary name.
pub const FIRMWARE_BIN_NAME: &str = "retrocontroller-firmware";

/// Finds the `firmware/` directory, either from an explicit override or by
/// walking up from the current directory and from `rcctl`'s own source tree
/// looking for `firmware/Cargo.toml`.
pub fn find_firmware_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        if dir.join("Cargo.toml").is_file() {
            return Ok(dir.to_path_buf());
        }
        bail!("--firmware-dir {} does not contain a Cargo.toml", dir.display());
    }

    let mut starts = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    // CARGO_MANIFEST_DIR is baked in at compile time as `tools/rcctl`; its
    // grandparent is the repo root, so this works even when the current
    // directory is somewhere else in the tree (or outside it entirely, as
    // long as the source checkout itself hasn't moved).
    starts.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    for start in starts {
        let mut dir = start.as_path();
        loop {
            let candidate = dir.join("firmware");
            if candidate.join("Cargo.toml").is_file() {
                return Ok(candidate);
            }
            match dir.parent() {
                Some(parent) => dir = parent,
                None => break,
            }
        }
    }

    bail!(
        "could not find firmware/Cargo.toml by walking up from the current directory \
         or rcctl's own source tree; pass --firmware-dir <path>"
    )
}

/// Returns `true` if `rustc`'s sysroot has the given target's standard
/// library installed. Best-effort: any failure to even run `rustc` is
/// treated as "unknown" (`Ok(true)`, i.e. don't nag) rather than an error,
/// since this is only used for a friendly warning.
pub fn is_target_installed(target: &str) -> Result<bool> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .context("running `rustc --print sysroot`")?;
    if !output.status.success() {
        bail!("`rustc --print sysroot` exited with {}", output.status);
    }
    let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(Path::new(&sysroot).join("lib").join("rustlib").join(target).is_dir())
}

/// Runs `cargo build --release --target <FIRMWARE_TARGET>` from *inside*
/// `firmware_dir`, streaming the child's stdout/stderr straight to ours, then
/// locates the resulting ELF.
///
/// Two things matter here together: Cargo discovers `.cargo/config.toml`
/// from the process's *current working directory*, not from
/// `--manifest-path` -- so running this from the repo root with only
/// `--manifest-path firmware/Cargo.toml` (and no cwd change) makes Cargo miss
/// `firmware/.cargo/config.toml`'s `[build] target`, silently falling back to
/// a *host* build of the no_std firmware. `.current_dir(firmware_dir)` fixes
/// that. `--target` is passed explicitly too, both as a belt-and-braces
/// safeguard against that same failure mode (it now takes precedence
/// regardless of `.cargo/config.toml`) and so [`find_firmware_elf`]'s
/// `target/<FIRMWARE_TARGET>/release/` path assumption holds by construction
/// rather than by accident.
pub fn build_firmware(firmware_dir: &Path) -> Result<PathBuf> {
    match is_target_installed(FIRMWARE_TARGET) {
        Ok(false) => {
            eprintln!(
                "warning: rust target `{FIRMWARE_TARGET}` does not appear to be installed.\n\
                 warning:   rustup target add {FIRMWARE_TARGET}"
            );
        }
        Ok(true) => {}
        Err(e) => eprintln!("warning: could not check installed targets: {e}"),
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg(FIRMWARE_TARGET)
        .current_dir(firmware_dir)
        .status()
        .with_context(|| {
            format!(
                "running `cargo build --release --target {FIRMWARE_TARGET}` in {}",
                firmware_dir.display()
            )
        })?;
    if !status.success() {
        bail!("firmware build failed ({status})");
    }

    find_firmware_elf(firmware_dir)
}

/// Locates the built firmware ELF under `<firmware_dir>/target/<target>/release/`.
/// Prefers the expected binary name; if that's not there (e.g. the package
/// was renamed), falls back to the single extensionless file in that
/// directory.
pub fn find_firmware_elf(firmware_dir: &Path) -> Result<PathBuf> {
    let release_dir = firmware_dir.join("target").join(FIRMWARE_TARGET).join("release");
    let expected = release_dir.join(FIRMWARE_BIN_NAME);
    if expected.is_file() {
        return Ok(expected);
    }

    if !release_dir.is_dir() {
        bail!("no firmware build output found at {} -- run `rcctl build` first", release_dir.display());
    }

    let mut candidates = Vec::new();
    for entry in
        std::fs::read_dir(&release_dir).with_context(|| format!("reading {}", release_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        // Cargo always writes a zero-length `.cargo-lock` file here, which
        // is itself extensionless (`Path::extension()` treats a name
        // starting with `.` as having no stem, hence no extension) and would
        // otherwise always be miscounted as a second candidate. Skip
        // dotfiles generally, since cargo/tooling output directories only
        // ever put its own bookkeeping files there.
        let is_dotfile = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
        if path.is_file() && path.extension().is_none() && !is_dotfile {
            candidates.push(path);
        }
    }

    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(anyhow!(
            "no firmware ELF found in {} (expected `{FIRMWARE_BIN_NAME}`); did the build succeed?",
            release_dir.display()
        )),
        _ => Err(anyhow!(
            "found {} extensionless files in {}, expected exactly one firmware ELF: {:?}",
            candidates.len(),
            release_dir.display(),
            candidates
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch directory under the OS temp dir for one test.
    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        std::env::temp_dir().join(format!("rcctl-firmware-test-{name}-{}-{nanos}", std::process::id()))
    }

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Cargo always writes a zero-length `.cargo-lock` bookkeeping file into
    /// a target output directory; it is itself extensionless
    /// (`Path::extension()` returns `None` for a name that starts with `.`),
    /// so the fallback used to always find 2 "extensionless" candidates and
    /// fail even when there was exactly one real (renamed) firmware ELF.
    #[test]
    fn fallback_skips_cargo_lock_dotfile() {
        let firmware_dir = unique_temp_dir("skips-dotfile");
        let _cleanup = Cleanup(firmware_dir.clone());
        let release_dir = firmware_dir.join("target").join(FIRMWARE_TARGET).join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join(".cargo-lock"), b"").unwrap();
        std::fs::write(release_dir.join("my-renamed-firmware"), b"\x7fELF...").unwrap();

        let found = find_firmware_elf(&firmware_dir).expect("should find the one real candidate");
        assert_eq!(found.file_name().unwrap(), "my-renamed-firmware");
    }

    /// `build_firmware` must run cargo with its cwd set to `firmware_dir`
    /// (not rcctl's own cwd) and pass `--target` explicitly, so Cargo picks
    /// up `firmware/.cargo/config.toml`'s `[build] target` regardless of
    /// where rcctl itself was invoked from. Verified with a fake `cargo` on
    /// `PATH` that records its cwd and args instead of actually building
    /// anything (a real firmware build is far too slow/heavy for a unit
    /// test, and isn't the point here: before this fix, running `cargo build
    /// --release --manifest-path <dir>/Cargo.toml` with no `.current_dir`
    /// and no `--target` silently built the no_std firmware for the *host*
    /// target whenever rcctl ran from outside `firmware/`).
    #[test]
    fn build_firmware_runs_cargo_from_within_firmware_dir_with_explicit_target() {
        let firmware_dir = unique_temp_dir("cargo-cwd-and-target");
        let _cleanup = Cleanup(firmware_dir.clone());
        std::fs::create_dir_all(&firmware_dir).unwrap();
        std::fs::write(firmware_dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();

        let release_dir = firmware_dir.join("target").join(FIRMWARE_TARGET).join("release");
        std::fs::create_dir_all(&release_dir).unwrap();

        // A fake `cargo` that: records its cwd + args to `call.txt` (in the
        // firmware dir, so the test can find it without an env var), then
        // writes the expected ELF into the release dir cargo would have
        // produced it in, then exits 0.
        let fake_bin_dir = unique_temp_dir("cargo-cwd-and-target-bin");
        let _cleanup_bin = Cleanup(fake_bin_dir.clone());
        std::fs::create_dir_all(&fake_bin_dir).unwrap();
        let call_log = firmware_dir.join("call.txt");
        let fake_cargo = fake_bin_dir.join("cargo");
        std::fs::write(
            &fake_cargo,
            format!(
                "#!/bin/sh\npwd > {log}\necho \"$@\" >> {log}\ntouch {bin}\n",
                log = call_log.display(),
                bin = release_dir.join(FIRMWARE_BIN_NAME).display(),
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake_cargo, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        // Put the fake cargo first on PATH; keep the real PATH after it so
        // `rustc --print sysroot` (called by `is_target_installed`) still
        // works.
        let old_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{old_path}", fake_bin_dir.display());
        let old_path_guard = std::env::var_os("PATH");
        // SAFETY: rcctl's tests run single-threaded-enough that a plain env
        // var mutation here is acceptable for this narrowly-scoped check;
        // restored via the RAII guard below regardless of how the test exits.
        unsafe { std::env::set_var("PATH", &new_path) };
        struct RestorePath(Option<std::ffi::OsString>);
        impl Drop for RestorePath {
            fn drop(&mut self) {
                match &self.0 {
                    Some(v) => unsafe { std::env::set_var("PATH", v) },
                    None => unsafe { std::env::remove_var("PATH") },
                }
            }
        }
        let _restore = RestorePath(old_path_guard);

        let found = build_firmware(&firmware_dir).expect("fake cargo build should succeed");
        assert_eq!(found, release_dir.join(FIRMWARE_BIN_NAME));

        let log = std::fs::read_to_string(&call_log).expect("fake cargo should have run and logged");
        let mut lines = log.lines();
        let logged_cwd = lines.next().expect("logged cwd");
        let logged_args = lines.next().expect("logged args");

        assert_eq!(
            std::fs::canonicalize(logged_cwd).unwrap(),
            std::fs::canonicalize(&firmware_dir).unwrap(),
            "cargo must run with cwd == firmware_dir, not rcctl's own cwd"
        );
        assert!(
            logged_args.contains(&format!("--target {FIRMWARE_TARGET}")),
            "cargo must be invoked with an explicit --target: {logged_args:?}"
        );
        assert!(
            !logged_args.contains("--manifest-path"),
            "manifest-path is redundant with cwd: {logged_args:?}"
        );
    }

    /// With genuinely ambiguous candidates the fallback must still refuse to
    /// guess -- and the dotfile must not show up in that error either.
    #[test]
    fn fallback_still_errors_on_multiple_real_candidates() {
        let firmware_dir = unique_temp_dir("multiple-candidates");
        let _cleanup = Cleanup(firmware_dir.clone());
        let release_dir = firmware_dir.join("target").join(FIRMWARE_TARGET).join("release");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join(".cargo-lock"), b"").unwrap();
        std::fs::write(release_dir.join("a"), b"\x7fELF...").unwrap();
        std::fs::write(release_dir.join("b"), b"\x7fELF...").unwrap();

        let err = find_firmware_elf(&firmware_dir).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("expected exactly one firmware ELF"), "unexpected error: {msg}");
        assert!(!msg.contains(".cargo-lock"), "dotfile should have been filtered out of the error: {msg}");
    }
}
