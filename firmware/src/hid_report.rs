//! The HID gamepad input report (SPEC-controller.md §2.1): two 16-bit
//! absolute axes (`0..=4095`) plus an 8-bit button mask (bits 0/1: the
//! optional left/right buttons; the rest always 0).
//!
//! `usbd_hid`'s `gen_hid_descriptor` macro generates X and Y as two separate
//! 16-bit `Input` items (each its own `Report Count 1`), rather than one
//! `Report Count 2` item as SPEC §2.1's illustrative descriptor bytes show.
//! Both encodings describe the identical report shape — a 5-byte little
//! endian `x:u16 y:u16 buttons:u8` with axes `0..=4095` — so this is a
//! byte-level descriptor difference only, not a functional one; see the
//! firmware README for the actual descriptor bytes this produces.

//! Only [`GamepadReport::desc`] (the generated descriptor bytes) is actually
//! used: the wire report itself is written with
//! `retrocontroller_protocol::hid::PaddleReport::to_bytes()` (the protocol
//! crate's own canonical byte layout), via `HidWriter::write`, rather than
//! `write_serialize`-ing this struct, so the two representations can't drift
//! apart.

use usbd_hid::descriptor::generator_prelude::*;

#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = GAMEPAD) = {
        (collection = PHYSICAL, usage = POINTER) = {
            (usage_page = GENERIC_DESKTOP, logical_min = 0, logical_max = 4095) = {
                (usage = X,) = { #[item_settings(data,variable,absolute)] x=input; };
                (usage = Y,) = { #[item_settings(data,variable,absolute)] y=input; };
            };
        };
        (usage_page = BUTTON, usage_min = BUTTON_1, usage_max = BUTTON_8) = {
            #[packed_bits = 8] #[item_settings(data,variable,absolute)] buttons=input;
        };
    }
)]
#[allow(dead_code)]
pub struct GamepadReport {
    pub x: u16,
    pub y: u16,
    pub buttons: u8,
}
