//! RetroController firmware for the Seeed XIAO RP2350.
//!
//! Wires the hardware-free logic in `retrocontroller-firmware-logic` to real
//! embassy-rp peripherals: two ADC channels (the paddles), two optional
//! active-low button inputs, a composite USB
//! HID gamepad + CDC-ACM serial device, the second-to-last-4K flash sector
//! (calibration), and the on-board user LED. See `README.md` for the pin map
//! and `/docs/SPEC-controller.md` for the protocol this implements.

#![no_std]
#![no_main]

mod cal_flash;
mod hid_report;
mod usb_handlers;

use core::cell::RefCell;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, Ordering};

use embassy_executor::Spawner;
use embassy_futures::join::{join, join3};
use embassy_futures::select::{select, Either};
use embassy_rp::adc::{
    Adc, Channel as AdcChannel, Config as AdcConfig, InterruptHandler as AdcInterruptHandler,
};
use embassy_rp::bind_interrupts;
use embassy_rp::flash::Flash;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver as UsbDriver, InterruptHandler as UsbInterruptHandler};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_time::{with_timeout, Duration, Instant, Ticker, Timer};
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as CdcState};
use embassy_usb::class::hid::{
    Config as HidConfig, HidBootProtocol, HidSubclass, HidWriter, State as HidState,
};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config as UsbConfig};
use panic_halt as _;
use static_cell::StaticCell;
use usbd_hid::descriptor::SerializedDescriptor;

use retrocontroller_firmware_logic::buf::FixedBuf;
use retrocontroller_firmware_logic::buttons::Buttons;
use retrocontroller_firmware_logic::device::{format_stream_line, Device, Effect, Snapshot};
use retrocontroller_firmware_logic::filter::AxisFilter;
use retrocontroller_firmware_logic::hysteresis::{Hysteresis, Report as HysReport};
use retrocontroller_firmware_logic::led::{led_on, select_led_state};
use retrocontroller_firmware_logic::line::{FedLine, LineAssembler, ERR_LINE_TOO_LONG};
use retrocontroller_protocol::hid::{PaddleReport, POLL_MS, REPORT_LEN};
use retrocontroller_protocol::serial;
use retrocontroller_protocol::usb::{MANUFACTURER, PID, PRODUCT, VID};

use crate::cal_flash::AppFlash;
use crate::hid_report::GamepadReport;
use crate::usb_handlers::{is_configured, DeviceHandler, PaddleRequestHandler};

// ---------- picotool metadata ----------
#[link_section = ".bi_entries"]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 4] = [
    embassy_rp::binary_info::rp_program_name!(c"retrocontroller-firmware"),
    embassy_rp::binary_info::rp_program_description!(
        c"RetroController paddle: USB HID gamepad + CDC-ACM serial"
    ),
    embassy_rp::binary_info::rp_cargo_version!(),
    embassy_rp::binary_info::rp_program_build_attribute!(),
];

// NOTE: embassy-rp 0.10 (feature `rp235xa`) emits the `.start_block` IMAGE_DEF
// itself (`ImageDef::secure_exe()` by default) -- do not define one manually
// unless the `imagedef-none` feature is enabled.

bind_interrupts!(struct Irqs {
    ADC_IRQ_FIFO => AdcInterruptHandler;
    USBCTRL_IRQ => UsbInterruptHandler<USB>;
});

type AppDriver<'d> = UsbDriver<'d, USB>;
type DeviceMutex = BlockingMutex<CriticalSectionRawMutex, RefCell<Device>>;
type SnapshotMutex = BlockingMutex<CriticalSectionRawMutex, RefCell<Snapshot>>;

/// Millisecond count since boot, truncated to `u32` (wraps after ~49.7 days;
/// every consumer below uses wrapping/modulo arithmetic, so a wrap just looks
/// like a very small elapsed time rather than a panic or a huge one).
fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}

/// Timestamp (ms) of the last HID report actually written to the host; read
/// by the LED task to decide "knob moving" (a report within the last 100ms).
static LAST_HID_REPORT_MS: AtomicU32 = AtomicU32::new(0);

/// Reads the RP2350's per-chip unique id.
///
/// Deviation from SPEC-controller.md §2's "RP2350 unique flash ID": embassy-rp
/// 0.10.0's `Flash::blocking_unique_id`/`blocking_jedec_id` (the QSPI flash
/// chip's unique-id command) are only implemented under the `rp2040`
/// feature — there is no such method for `rp235xa` in this crate version (it
/// doesn't compile: `no method named 'blocking_unique_id'`). RP2350 has its
/// own OTP-programmed 64-bit chip id instead (`embassy_rp::otp::get_chipid`,
/// rows 0-3), which is the platform-native equivalent: still a stable,
/// unique-per-board 64-bit value, formatted the same way as 16 uppercase hex
/// digits for `INFO`'s `uid=` field and the USB serial number string.
fn read_unique_id() -> [u8; 8] {
    embassy_rp::otp::get_chipid().unwrap_or(0).to_be_bytes()
}

/// Formats an 8-byte unique id as 16 uppercase hex ASCII bytes.
fn hex16(uid: &[u8; 8]) -> [u8; 16] {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = [0u8; 16];
    for (i, b) in uid.iter().enumerate() {
        out[i * 2] = DIGITS[(b >> 4) as usize];
        out[i * 2 + 1] = DIGITS[(b & 0x0F) as usize];
    }
    out
}

/// Bound on a single CDC-ACM response/stream-line write. embassy-rp 0.10.0's
/// `Endpoint<In>::write` only resolves once the host has taken the packet
/// (buffer-control `available`, never an error), so a host that stops
/// draining the IN pipe (backgrounded terminal, suspended process, a full tty
/// flip buffer) would otherwise stall `cdc_fut` forever: `STREAM OFF`,
/// `BOOTSEL`, `CAL SAVE` etc. sent afterwards would never be processed, and a
/// later DTR drop would never be noticed (SPEC-controller.md §5: "an
/// unattended port can never stall the device"). On expiry the write is
/// abandoned and the connection is treated as gone (see `cdc_fut`), so the
/// line assembler and `stream_on`/`learning` are reset before the next
/// session rather than leaking into it.
const CDC_WRITE_TIMEOUT: Duration = Duration::from_millis(100);

/// Writes `data` to the CDC-ACM IN endpoint as one or more <=64-byte packets
/// (`write_packet` sends exactly one packet; longer responses like `INFO`
/// need several).
async fn cdc_write_all<'d>(
    cdc: &mut CdcAcmClass<'d, AppDriver<'d>>,
    mut data: &[u8],
) -> Result<(), EndpointError> {
    while !data.is_empty() {
        let n = data.len().min(64);
        cdc.write_packet(&data[..n]).await?;
        data = &data[n..];
    }
    Ok(())
}

/// Carries out a side effect that doesn't need to finish before the response
/// is sent. `Effect::SaveCal`/`Effect::PersistCal` are handled separately,
/// inline in `cdc_fut`, *before* the response is formatted (see
/// `finish_cal_effect`): the flash write must complete first so a failure can
/// be reported as `ERR flash` instead of an `OK` the host would wrongly
/// trust.
async fn apply_effect(effect: Effect) {
    match effect {
        Effect::None | Effect::SaveCal(_) | Effect::PersistCal(_) => {}
        Effect::Bootsel => {
            Timer::after_millis(50).await;
            embassy_rp::rom_data::reset_to_usb_boot(0, 0);
        }
        Effect::Reboot => {
            Timer::after_millis(50).await;
            cortex_m::peripheral::SCB::sys_reset();
        }
    }
}

/// Attempts the flash write for a `CAL SAVE`/`CAL RESET`/`INVERT` effect and
/// writes the resulting `OK .../ERR flash` line to `resp`, completing the
/// response that `Device::handle_line` deliberately left pending (see
/// `retrocontroller_firmware_logic::device::Effect`'s docs). A no-op for any
/// other effect.
fn finish_cal_effect(
    effect: &Effect,
    flash: &mut AppFlash<'_>,
    device: &DeviceMutex,
    resp: &mut FixedBuf<160>,
) {
    match *effect {
        Effect::SaveCal(cal) => {
            let ok = cal_flash::save_calibration(flash, &cal);
            device.lock(|d| d.borrow_mut().finish_cal_save(ok, resp));
        }
        Effect::PersistCal(cal) => {
            let ok = cal_flash::save_calibration(flash, &cal);
            device.lock(|d| d.borrow_mut().finish_persist_cal(ok, resp));
        }
        Effect::None | Effect::Bootsel | Effect::Reboot => {}
    }
}

#[embassy_executor::main(executor = "embassy_rp::executor::Executor", entry = "cortex_m_rt::entry")]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // ---------- flash: boot-time identity + calibration ----------
    let mut flash: AppFlash = Flash::new_blocking(p.FLASH);
    let uid = read_unique_id();
    let (cal, saved) = cal_flash::load_calibration(&mut flash);

    static SERIAL_HEX: StaticCell<[u8; 16]> = StaticCell::new();
    let serial_hex: &'static [u8; 16] = SERIAL_HEX.init(hex16(&uid));
    let serial_str: &'static str = core::str::from_utf8(serial_hex).unwrap();

    static DEVICE_CELL: StaticCell<DeviceMutex> = StaticCell::new();
    let device: &'static DeviceMutex = DEVICE_CELL.init(BlockingMutex::new(RefCell::new(Device::new(
        env!("CARGO_PKG_VERSION"),
        uid,
        cal,
        saved,
    ))));

    static SNAPSHOT_CELL: StaticCell<SnapshotMutex> = StaticCell::new();
    let snapshot: &'static SnapshotMutex =
        SNAPSHOT_CELL.init(BlockingMutex::new(RefCell::new(Snapshot::default())));

    // ---------- ADC ----------
    let mut adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let mut ch_l = AdcChannel::new_pin(p.PIN_26, Pull::None);
    let mut ch_r = AdcChannel::new_pin(p.PIN_27, Pull::None);

    // ---------- optional buttons (D2 = GPIO28 beside A0, D3 = GPIO5 beside A1) ----------
    // Active-low against the internal pull-up, so an unwired pad just reads
    // released (see `retrocontroller_firmware_logic::buttons`).
    let btn_a0 = Input::new(p.PIN_28, Pull::Up);
    let btn_a1 = Input::new(p.PIN_5, Pull::Up);

    // ---------- user LED (GPIO25, active-low) ----------
    let mut led = Output::new(p.PIN_25, Level::High);

    // ---------- USB ----------
    let driver = UsbDriver::new(p.USB, Irqs);
    let mut usb_config = UsbConfig::new(VID, PID);
    usb_config.manufacturer = Some(MANUFACTURER);
    usb_config.product = Some(PRODUCT);
    usb_config.serial_number = Some(serial_str);
    usb_config.max_power = 100;
    usb_config.max_packet_size_0 = 64;
    usb_config.composite_with_iads = true;
    usb_config.device_class = 0xEF;
    usb_config.device_sub_class = 0x02;
    usb_config.device_protocol = 0x01;

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
    static HID_STATE: StaticCell<HidState> = StaticCell::new();
    static CDC_STATE: StaticCell<CdcState> = StaticCell::new();
    static DEV_HANDLER: StaticCell<DeviceHandler> = StaticCell::new();
    static REQ_HANDLER: StaticCell<PaddleRequestHandler> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        usb_config,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 256]),
        MSOS_DESC.init([0; 256]),
        CONTROL_BUF.init([0; 64]),
    );
    builder.handler(DEV_HANDLER.init(DeviceHandler));

    let hid_config = HidConfig {
        report_descriptor: GamepadReport::desc(),
        // Routes control-pipe GET_REPORT/GET_IDLE/GET_PROTOCOL to
        // `PaddleRequestHandler` instead of leaving them `None` (which makes
        // embassy-usb STALL them). This is independent of the IN/OUT
        // endpoints below: `Control` (the device-level `Handler`) answers
        // these purely over the control pipe.
        request_handler: Some(REQ_HANDLER.init(PaddleRequestHandler)),
        poll_ms: POLL_MS,
        // SPEC-controller.md §2: "IN endpoint 8 bytes". `HidWriter` (IN-only)
        // allocates a single interrupt IN endpoint of this size; REPORT_LEN
        // (5) fits in one packet with no ZLP needed.
        max_packet_size: 8,
        hid_subclass: HidSubclass::No,
        hid_boot_protocol: HidBootProtocol::None,
    };
    // `HidWriter` (not `HidReaderWriter`) allocates only the IN endpoint the
    // spec calls for; `HidReaderWriter` would also allocate an unspecified
    // interrupt OUT endpoint that exists only to be drained by a no-op
    // handler.
    let mut hid_writer =
        HidWriter::<_, REPORT_LEN>::new(&mut builder, HID_STATE.init(HidState::new()), hid_config);
    let mut cdc = CdcAcmClass::new(&mut builder, CDC_STATE.init(CdcState::new()), 64);

    let mut usb = builder.build();
    let usb_fut = usb.run();

    // ---------- ADC sampling + filtering + calibration-learning ----------
    let adc_fut = async {
        let mut filt_l = AxisFilter::new();
        let mut filt_r = AxisFilter::new();
        // 125us period, one sample per axis per tick -> 8 kHz/axis, matching
        // SPEC-controller.md §3.
        let mut ticker = Ticker::every(Duration::from_micros(125));
        loop {
            ticker.next().await;
            let l12 = adc.read(&mut ch_l).await.unwrap_or(0);
            let r12 = adc.read(&mut ch_r).await.unwrap_or(0);
            // Both filters are fed exactly one sample per tick, so their
            // 16-sample oversampling batches always complete together.
            let new_l = filt_l.add_sample(l12);
            let new_r = filt_r.add_sample(r12);
            if let (Some(raw_l), Some(raw_r)) = (new_l, new_r) {
                let (val_l, val_r) = device.lock(|d| {
                    let mut dev = d.borrow_mut();
                    dev.observe_raw(raw_l, raw_r);
                    dev.cal.apply(raw_l, raw_r)
                });
                snapshot.lock(|s| {
                    let mut s = s.borrow_mut();
                    (s.raw_l, s.raw_r, s.val_l, s.val_r) = (raw_l, raw_r, val_l, val_r);
                });
            }
        }
    };

    // ---------- HID report task: buttons + hysteresis + keep-alive at 1 kHz ----------
    let hid_in_fut = async {
        let mut hys_l = Hysteresis::new();
        let mut hys_r = Hysteresis::new();
        let mut buttons = Buttons::new();
        let mut ticker = Ticker::every(Duration::from_millis(1));
        loop {
            ticker.next().await;
            buttons.update(btn_a0.is_low(), btn_a1.is_low());
            let swap = device.lock(|d| d.borrow().cal.swap);
            let btn = buttons.logical(swap);
            let (snap, buttons_changed) = snapshot.lock(|s| {
                let mut s = s.borrow_mut();
                let changed = s.buttons.pressed != btn.pressed;
                s.buttons = btn;
                (*s, changed)
            });
            let t = now_ms();
            let moved_l = hys_l.update(snap.val_l, t);
            let moved_r = hys_r.update(snap.val_r, t);
            // A button edge is sent at once rather than waiting for the
            // next knob movement or keep-alive.
            if buttons_changed || moved_l.is_some() || moved_r.is_some() {
                // A keep-alive report (fired at least every
                // `hysteresis::KEEPALIVE_MS`, faster than the LED's own
                // "recently" window) must NOT count as "the knob moved":
                // only actual `Report::Changed` reports refresh
                // `LAST_HID_REPORT_MS`, otherwise the "moving" LED state
                // (SPEC-controller.md §6) is permanently true once the host
                // starts polling HID and the idle 1 Hz blink can never show.
                let changed = buttons_changed
                    || matches!(moved_l, Some(HysReport::Changed(_)))
                    || matches!(moved_r, Some(HysReport::Changed(_)));
                // Build the report from each axis's own last *reported*
                // value, not the raw snapshot: only one axis's hysteresis may
                // have fired this tick, and reading `snap.val_*` for the
                // other axis would put its still-unreported 1-LSB jitter on
                // the wire verbatim, bypassing that axis's hysteresis
                // (SPEC-controller.md §3 applies per axis). `last_reported()`
                // is only `None` before an axis has ever reported; a button
                // edge can get here first, so fall back to the snapshot.
                let val_l = hys_l.last_reported().unwrap_or(snap.val_l);
                let val_r = hys_r.last_reported().unwrap_or(snap.val_r);
                let report = PaddleReport::with_buttons(val_l, val_r, btn.pressed);
                match hid_writer.write(&report.to_bytes()).await {
                    Ok(()) => {
                        if changed {
                            LAST_HID_REPORT_MS.store(t, Ordering::Relaxed);
                        }
                    }
                    // embassy-rp 0.10.0's `Endpoint::write` only polls the
                    // endpoint's buffer-control `available` bit; it never
                    // inspects the endpoint `enable` bit and so never
                    // actually returns `EndpointError::Disabled` (verified
                    // against embassy-rp 0.10.0 src/usb.rs). Before
                    // enumeration, `write` simply blocks until the host's
                    // first SET_CONFIGURATION rewrites buffer control (which
                    // discards anything queued here and lets a pending write
                    // go out fresh), rather than failing -- this arm exists
                    // only for API completeness in case a future driver
                    // version starts returning it.
                    Err(EndpointError::Disabled) => {}
                    Err(EndpointError::BufferOverflow) => {}
                }
            }
        }
    };

    // ---------- CDC-ACM command handling + STREAM ----------
    let cdc_fut = async {
        let stream_period = Duration::from_hz(u64::from(serial::STREAM_HZ));
        loop {
            cdc.wait_connection().await;
            let mut assembler = LineAssembler::new();
            let mut resp: FixedBuf<160> = FixedBuf::new();
            let mut stream_ticker = Ticker::every(stream_period);
            // Tracks DTR so the reset below (line assembler, pending
            // response, stream, learning) runs once on the high-to-low
            // *transition*, not on every 20ms tick while DTR happens to be
            // low -- see the comment at its use below.
            let mut prev_dtr = cdc.dtr();
            'connected: loop {
                let mut pkt = [0u8; 64];
                match select(cdc.read_packet(&mut pkt), stream_ticker.next()).await {
                    Either::First(Ok(n)) => {
                        for &byte in &pkt[..n] {
                            let Some(fed) = assembler.feed(byte) else { continue };
                            resp.clear();
                            let effect = match fed {
                                FedLine::Ready(lb) => match lb.as_str() {
                                    Some(line) => {
                                        let snap = snapshot.lock(|s| *s.borrow());
                                        device.lock(|d| d.borrow_mut().handle_line(line, &snap, &mut resp))
                                    }
                                    None => {
                                        let _ = writeln!(resp, "ERR bad argument");
                                        Effect::None
                                    }
                                },
                                FedLine::TooLong => {
                                    let _ = writeln!(resp, "{ERR_LINE_TOO_LONG}");
                                    Effect::None
                                }
                            };
                            // Calibration effects must hit flash *before* the
                            // response is sent, so a failed write can be
                            // reported as `ERR flash` instead of the `OK`
                            // that was written above by `handle_line` for
                            // every other command (SPEC-controller.md §5:
                            // exactly one response line, which must reflect
                            // what actually happened).
                            finish_cal_effect(&effect, &mut flash, device, &mut resp);
                            // SPEC-controller.md §5: "only *sends* (responses
                            // and stream lines) while DTR is asserted, so an
                            // unattended port can never stall the device" --
                            // commands are still processed (and, for
                            // calibration effects, persisted) above
                            // regardless of DTR; only the write is gated.
                            // Skipping the write when DTR is low also avoids
                            // ever entering `cdc_write_all`'s blocking
                            // `write_packet` with no host pulling IN tokens.
                            if cdc.dtr() {
                                // Bounded: see `CDC_WRITE_TIMEOUT`'s docs. A
                                // timeout is treated exactly like a write
                                // error (below) -- abandon this connection
                                // rather than let a host that stopped
                                // draining IN packets stall this task
                                // forever.
                                match with_timeout(
                                    CDC_WRITE_TIMEOUT,
                                    cdc_write_all(&mut cdc, resp.as_bytes()),
                                )
                                .await
                                {
                                    Ok(Ok(())) => {}
                                    Ok(Err(_)) | Err(_) => break 'connected,
                                }
                            }
                            apply_effect(effect).await;
                        }
                    }
                    // embassy-rp 0.10.0's CDC-ACM `read_packet` (like
                    // `HidWriter`/`HidReader`, see the comment on
                    // `EndpointError::Disabled` in `hid_in_fut` above) blocks
                    // on buffer-control availability and does not return an
                    // error on a bus reset or a closed port, so in practice
                    // this arm is not reached: `cdc.wait_connection()` above
                    // runs exactly once per boot, not once per host session.
                    // What actually signals a closed port or a USB bus reset
                    // is DTR dropping (embassy-usb's cdc_acm `Control::reset`
                    // clears it on a bus reset; the host driver clears it on
                    // close), handled below.
                    Either::First(Err(_)) => break 'connected,
                    Either::Second(()) => {
                        let dtr = cdc.dtr();
                        if prev_dtr && !dtr {
                            // DTR just dropped (the host closed the port, or
                            // a USB bus reset occurred; see the comment on
                            // `Either::First(Err(_))` above). Since
                            // `cdc.wait_connection()` is not re-entered for
                            // either event, this per-connection state would
                            // otherwise persist across sessions: reset the
                            // line assembler and any pending response now, so
                            // a partial, newline-less command left over from
                            // before the reset (e.g. "INF") can't silently
                            // prefix the next session's first line and
                            // corrupt it (e.g. "INF" + "INFO\n" -> "INFINFO"
                            // -> "ERR unknown command"). This fires exactly
                            // once per high-to-low transition, not on every
                            // tick while DTR stays low -- SPEC-controller.md
                            // §5 says commands are processed regardless of
                            // DTR, so resetting the assembler on every tick
                            // would silently corrupt a command whose bytes
                            // straddle a tick while DTR was simply never
                            // asserted (a host that never calls
                            // `dtr_on_open`, e.g. macOS by default).
                            assembler = LineAssembler::new();
                            resp.clear();
                            // Stop streaming, leave any in-progress `CAL
                            // START` learning session, and resync the ticker
                            // so a later DTR-reassert doesn't trigger a burst
                            // of catch-up `S` lines for every period missed
                            // while unattended (`Ticker::next` resolves
                            // immediately for each elapsed period).
                            device.lock(|d| d.borrow_mut().on_disconnect());
                            stream_ticker.reset();
                        }
                        prev_dtr = dtr;
                        if !dtr {
                            continue 'connected;
                        }
                        let streaming = device.lock(|d| d.borrow().stream_on);
                        if streaming {
                            let snap = snapshot.lock(|s| *s.borrow());
                            let mut line: FixedBuf<64> = FixedBuf::new();
                            format_stream_line(&snap, &mut line);
                            // Bounded write: see `CDC_WRITE_TIMEOUT`'s docs.
                            match with_timeout(CDC_WRITE_TIMEOUT, cdc_write_all(&mut cdc, line.as_bytes()))
                                .await
                            {
                                Ok(Ok(())) => {}
                                Ok(Err(_)) | Err(_) => break 'connected,
                            }
                        }
                    }
                }
            }
            // Disconnected: don't keep streaming into the void, and don't
            // leave a `CAL START` session (or the assembler's state) alive
            // for whatever host connects next.
            device.lock(|d| d.borrow_mut().on_disconnect());
        }
    };

    // ---------- LED (SPEC-controller.md §6) ----------
    let led_fut = async {
        let mut ticker = Ticker::every(Duration::from_millis(5));
        loop {
            ticker.next().await;
            let t = now_ms();
            let configured = is_configured();
            let learning = device.lock(|d| d.borrow().learning);
            let moving = t.wrapping_sub(LAST_HID_REPORT_MS.load(Ordering::Relaxed)) <= 100;
            let state = select_led_state(configured, learning, moving);
            led.set_level(if led_on(state, t) { Level::Low } else { Level::High });
        }
    };

    join(usb_fut, join3(join(adc_fut, hid_in_fut), cdc_fut, led_fut)).await;
}
