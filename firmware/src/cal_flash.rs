//! Calibration persistence in the second-to-last 4 KiB flash sector
//! (SPEC-controller.md §4). The *last* sector is deliberately left alone: UF2
//! files produced by picotool/pico-sdk with `--abs-block` carry an
//! RP2350-E10 workaround block aimed at `0x10FFFF00`, which on a 2 MB part
//! wraps onto the last physical sector and would erase anything stored there
//! on A2 silicon.

use embassy_rp::flash::{Blocking, Flash, ERASE_SIZE};
use embassy_rp::peripherals::FLASH;
use retrocontroller_protocol::cal::Calibration;

/// XIAO RP2350's QSPI flash size (Seeed wiki; also `PICO_FLASH_SIZE_BYTES` in
/// the pico-sdk board header).
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;
/// `0x10000000 + FLASH_SIZE - 2*4096`, expressed as an offset from flash
/// start (embassy-rp's `Flash` API takes offsets, not absolute addresses):
/// the second-to-last 4 KiB sector, i.e. `0x1FE000`.
pub const CAL_OFFSET: u32 = (FLASH_SIZE - 2 * ERASE_SIZE) as u32;

pub type AppFlash<'d> = Flash<'d, FLASH, Blocking, FLASH_SIZE>;

/// Reads the calibration record. Returns `(cal, true)` if a valid record was
/// found, or `(Calibration::default(), false)` for erased/corrupt/missing
/// flash (magic/CRC mismatch or an insane span — see
/// `Calibration::from_bytes`).
pub fn load_calibration(flash: &mut AppFlash<'_>) -> (Calibration, bool) {
    let mut bytes = [0u8; Calibration::LEN];
    if flash.blocking_read(CAL_OFFSET, &mut bytes).is_ok() {
        if let Some(cal) = Calibration::from_bytes(&bytes) {
            return (cal, true);
        }
    }
    (Calibration::default(), false)
}

/// Erases and rewrites the calibration sector, then reads it back to confirm
/// the write landed correctly. Returns `false` on any flash error or if the
/// read-back doesn't match what was written.
pub fn save_calibration(flash: &mut AppFlash<'_>, cal: &Calibration) -> bool {
    let bytes = cal.to_bytes();
    if flash.blocking_erase(CAL_OFFSET, CAL_OFFSET + ERASE_SIZE as u32).is_err() {
        return false;
    }
    if flash.blocking_write(CAL_OFFSET, &bytes).is_err() {
        return false;
    }
    let mut check = [0u8; Calibration::LEN];
    if flash.blocking_read(CAL_OFFSET, &mut check).is_err() {
        return false;
    }
    check == bytes
}
