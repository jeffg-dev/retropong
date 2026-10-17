//! Pure, `no_std`, host-testable firmware logic for the RetroController
//! paddle controller (see `/docs/SPEC-controller.md`).
//!
//! This crate deliberately has **no** dependency on embassy or any hardware
//! access crate — only on `retrocontroller-protocol` for the shared wire
//! constants and types. Everything here is plain data in, plain data (or a
//! [`device::Effect`]) out, so it runs under `cargo test` on the host exactly
//! as it runs on the RP2350. The `firmware` binary crate wires this up to
//! real ADC/USB/flash peripherals via embassy-rp.
//!
//! Modules:
//! - [`filter`]: per-axis oversampling (12-bit ADC -> 16-bit raw) and EMA
//!   smoothing.
//! - [`hysteresis`]: output hysteresis + keep-alive deadline (SPEC §3).
//! - [`calibration`]: `CAL START`/`CAL STOP` min/max learning.
//! - [`device`]: the command handler ([`device::Device::handle`]) and the
//!   stream line formatter.
//! - [`line`]: byte-stream line reassembly for the serial protocol.
//! - [`led`]: user LED pattern selection (SPEC §6).
//! - [`buf`]: a tiny fixed-capacity `core::fmt::Write` sink used by the
//!   command handler's callers (tests here, and the firmware binary).

#![no_std]
#![deny(unsafe_code)]

pub mod buf;
pub mod calibration;
pub mod device;
pub mod filter;
pub mod hysteresis;
pub mod led;
pub mod line;

pub use device::{Device, Effect, Snapshot};
