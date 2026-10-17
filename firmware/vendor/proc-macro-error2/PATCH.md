# Local compatibility patch

This directory contains the crates.io source of `proc-macro-error2` 2.0.1,
licensed under MIT OR Apache-2.0; both upstream license files are included.
It is required by `embassy-rp` through `pio` and `pio-proc`.

The only source change makes `extern crate proc_macro` public in `src/lib.rs`.
The crate already publicly re-exports it, and current Rust reports that private
extern-crate re-export as future-incompatible (E0365). This corrects the
visibility rather than suppressing the diagnostic.

Upstream: https://github.com/GnomedDev/proc-macro-error-2
Upstream HEAD `b02d79c49898a8f4340aaa81ebd7cae3668ac6a9` still had this issue
when checked on 2026-10-17. Replace this patch with a fixed upstream release
when one is available.
