//! `rcctl`: the RetroController host toolkit (SPEC-controller.md §8).
//!
//! Build/flash the firmware, convert ELF -> UF2, and talk to a connected
//! controller over its CDC-ACM serial protocol or HID gamepad interface.

mod calibration_ui;
mod device;
mod firmware;
mod uf2;
mod volume;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use retrocontroller_protocol::{hid, serial, usb};
use serialport::SerialPortType;

/// pid.codes reserves 1209:0001 for private, in-house testing only; anyone
/// building from this repo should see this before shipping a device.
const PID_CODES_WARNING: &str = "note: RetroController uses the pid.codes *test* VID:PID 1209:0001 \
(reserved for private testing; do not sell or redistribute a device using it -- \
see https://pid.codes/1209/0001/ and https://pid.codes/howto/ to get your own).";

#[derive(Parser)]
#[command(
    name = "rcctl",
    version,
    about = "RetroController host toolkit: build, flash, monitor and calibrate the paddles"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Build the firmware (`cargo build --release` for thumbv8m.main-none-eabihf).
    Build {
        /// Build in release mode. This is currently the only mode rcctl
        /// builds the firmware in, so the flag is accepted (matching
        /// SPEC-controller.md §8's `rcctl build [--release]`) but has no
        /// effect either way.
        #[arg(long)]
        release: bool,
        /// Path to the firmware crate (auto-detected by default).
        #[arg(long)]
        firmware_dir: Option<PathBuf>,
    },
    /// Convert an ELF into an RP2350-flashable UF2 file.
    Uf2 {
        /// Path to the linked firmware ELF.
        elf: PathBuf,
        /// Output path (default: the ELF's path with a `.uf2` extension).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// UF2 family ID to stamp on every block of the main transfer.
        #[arg(long, value_enum, default_value_t = uf2::Family::Rp2350ArmS)]
        family: uf2::Family,
        /// Prepend the picotool-style RP2350-E10 absolute-block workaround.
        #[arg(long)]
        abs_block: bool,
    },
    /// Print information about a `.uf2` file (blocks, address range, family).
    Uf2Info {
        /// Path to the `.uf2` file.
        file: PathBuf,
    },
    /// Build (unless told not to), convert, and flash firmware to a connected controller.
    Flash {
        /// Use this ELF instead of building the firmware.
        #[arg(long)]
        elf: Option<PathBuf>,
        /// Don't build the firmware first (implies using `--elf` or the last build output).
        #[arg(long)]
        no_build: bool,
        /// Don't send BOOTSEL over serial; wait for the button combo instead.
        #[arg(long)]
        no_bootsel: bool,
        /// Use this bootloader volume instead of auto-detecting one.
        #[arg(long)]
        volume: Option<PathBuf>,
        /// Also write the generated UF2 to this path.
        #[arg(long)]
        uf2_out: Option<PathBuf>,
        /// Prepend the picotool-style RP2350-E10 absolute-block workaround.
        #[arg(long)]
        abs_block: bool,
        /// Path to the firmware crate (auto-detected by default).
        #[arg(long)]
        firmware_dir: Option<PathBuf>,
    },
    /// List RetroController serial ports, HID devices and bootloader volumes.
    Devices,
    /// Print the controller's `INFO` line.
    Info,
    /// Live bar-graph view of both knobs via the serial `STREAM` command.
    Monitor {
        /// Print raw protocol lines instead of bar graphs.
        #[arg(long)]
        raw: bool,
    },
    /// Read HID reports directly -- what games actually see.
    Test,
    /// Interactive calibration: sweep both knobs, then save.
    Calibrate,
    /// Direct calibration commands.
    Cal {
        #[command(subcommand)]
        action: CalAction,
    },
    /// Reboot into the USB (UF2) bootloader.
    Bootsel,
    /// Reboot normally.
    Reboot,
    /// Print (or install) the Linux udev rules the toolkit needs.
    Udev {
        /// Write the rules file and reload udev (needs root).
        #[arg(long)]
        install: bool,
    },
}

#[derive(Subcommand)]
enum CalAction {
    /// Toggle left/right wheel assignment, or explicitly set on/off, and save it.
    Swap {
        #[arg(value_enum, ignore_case = true)]
        on: Option<OnOffArg>,
    },
    /// Show the live calibration.
    Show,
    /// Reset to defaults (persisted).
    Reset,
    /// Set the calibration directly.
    Set {
        /// Left knob's minimum raw value.
        lmin: u16,
        /// Left knob's maximum raw value.
        lmax: u16,
        /// Right knob's minimum raw value.
        rmin: u16,
        /// Right knob's maximum raw value.
        rmax: u16,
    },
    /// Invert a knob's direction.
    Invert {
        /// Which knob to invert.
        #[arg(value_enum, ignore_case = true)]
        knob: KnobArg,
        /// The new inversion state.
        #[arg(value_enum, ignore_case = true)]
        on: OnOffArg,
    },
}

/// `rcctl cal invert`'s knob argument: `l`/`left` or `r`/`right`, checked by
/// clap before ever reaching the device (a bare `String` used to pass any
/// value locally and only fail after a serial round-trip with `ERR bad
/// argument`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum KnobArg {
    #[value(name = "l", alias = "left")]
    Left,
    #[value(name = "r", alias = "right")]
    Right,
}

impl KnobArg {
    const fn protocol_letter(self) -> &'static str {
        match self {
            KnobArg::Left => "L",
            KnobArg::Right => "R",
        }
    }
}

/// `rcctl cal invert`'s on/off argument: `1`/`on` or `0`/`off`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OnOffArg {
    #[value(name = "1", alias = "on")]
    On,
    #[value(name = "0", alias = "off")]
    Off,
}

impl OnOffArg {
    const fn protocol_digit(self) -> &'static str {
        match self {
            OnOffArg::On => "1",
            OnOffArg::Off => "0",
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Build { release: _, firmware_dir } => cmd_build(firmware_dir.as_deref()),
        Command::Uf2 { elf, out, family, abs_block } => cmd_uf2(elf, out, family, abs_block),
        Command::Uf2Info { file } => cmd_uf2_info(file),
        Command::Flash { elf, no_build, no_bootsel, volume, uf2_out, abs_block, firmware_dir } => {
            cmd_flash(elf, no_build, no_bootsel, volume, uf2_out, abs_block, firmware_dir.as_deref())
        }
        Command::Devices => cmd_devices(),
        Command::Info => cmd_info(),
        Command::Monitor { raw } => cmd_monitor(raw),
        Command::Test => cmd_test(),
        Command::Calibrate => cmd_calibrate(),
        Command::Cal { action } => cmd_cal(action),
        Command::Bootsel => cmd_bootsel(),
        Command::Reboot => cmd_reboot(),
        Command::Udev { install } => cmd_udev(install),
    }
}

// ---------------------------------------------------------------- build/uf2

fn cmd_build(firmware_dir: Option<&Path>) -> Result<()> {
    let dir = firmware::find_firmware_dir(firmware_dir)?;
    let elf = firmware::build_firmware(&dir)?;
    println!("Built {}", elf.display());
    Ok(())
}

/// Warns when `family` is one the bare (no partition table) RP2350 bootrom
/// silently drops every block of -- `rp2350-arm-ns` needs a partition table
/// to route to (`research:uf2-flash` §2, and `uf2::Family`'s own doc
/// comment: "Not directly bootable by the bootrom"). Without this, `rcctl
/// uf2 --family rp2350-arm-ns` used to exit 0 and print a success message
/// for a UF2 the bootrom accepts the *copy* of but never actually writes --
/// per the datasheet, "it will appear as if nothing has happened".
fn family_partition_table_warning(family: uf2::Family) -> Option<String> {
    (family == uf2::Family::Rp2350ArmNs).then(|| {
        format!(
            "warning: family {family} is not directly bootable by the RP2350 bootrom on a board with \
             no partition table -- the bootloader volume will accept the copy, but every block will be \
             silently ignored (the device will not reboot and nothing will be written); use \
             rp2350-arm-s unless you have a partition table that routes {family} somewhere"
        )
    })
}

fn cmd_uf2(elf: PathBuf, out: Option<PathBuf>, family: uf2::Family, abs_block: bool) -> Result<()> {
    if let Some(warning) = family_partition_table_warning(family) {
        eprintln!("{warning}");
    }
    let bytes = std::fs::read(&elf).with_context(|| format!("reading {}", elf.display()))?;
    let opts = uf2::Uf2Options { family, abs_block };
    let uf2_bytes = uf2::elf_to_uf2(&bytes, &opts)?;
    let out_path = out.unwrap_or_else(|| elf.with_extension("uf2"));
    std::fs::write(&out_path, &uf2_bytes).with_context(|| format!("writing {}", out_path.display()))?;
    println!(
        "Wrote {} ({} bytes, {} blocks, family {family})",
        out_path.display(),
        uf2_bytes.len(),
        uf2_bytes.len() / uf2::BLOCK_SIZE,
    );
    Ok(())
}

fn cmd_uf2_info(file: PathBuf) -> Result<()> {
    let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
    let blocks = uf2::parse_uf2(&bytes)?;
    let summary = uf2::summarize(&blocks);

    println!("{}: {} bytes, {} block(s)", file.display(), bytes.len(), summary.total_blocks);
    if summary.abs_blocks > 0 {
        println!("  {} RP2350-E10 absolute/ignore block(s)", summary.abs_blocks);
    }
    println!("  {} data block(s)", summary.data_blocks);
    if summary.data_blocks > 0 {
        println!(
            "  address range: 0x{:08x}..0x{:08x} ({} bytes)",
            summary.addr_min,
            summary.addr_max_end,
            summary.addr_max_end - summary.addr_min as u64,
        );
    }
    for (family, count) in &summary.families {
        println!("  family: {family} x{count}");
    }
    Ok(())
}

// -------------------------------------------------------------------- flash

fn cmd_flash(
    elf: Option<PathBuf>,
    no_build: bool,
    no_bootsel: bool,
    volume_override: Option<PathBuf>,
    uf2_out: Option<PathBuf>,
    abs_block: bool,
    firmware_dir: Option<&Path>,
) -> Result<()> {
    let elf_path = match elf {
        Some(p) => p,
        None => {
            let dir = firmware::find_firmware_dir(firmware_dir)?;
            if no_build {
                firmware::find_firmware_elf(&dir)?
            } else {
                firmware::build_firmware(&dir)?
            }
        }
    };
    println!("Firmware ELF: {}", elf_path.display());

    let elf_bytes = std::fs::read(&elf_path).with_context(|| format!("reading {}", elf_path.display()))?;
    let opts = uf2::Uf2Options { family: uf2::Family::Rp2350ArmS, abs_block };
    let uf2_bytes = uf2::elf_to_uf2(&elf_bytes, &opts)?;
    println!("Converted to UF2: {} bytes, {} block(s)", uf2_bytes.len(), uf2_bytes.len() / uf2::BLOCK_SIZE);

    if let Some(extra) = &uf2_out {
        std::fs::write(extra, &uf2_bytes).with_context(|| format!("writing {}", extra.display()))?;
        println!("Also wrote {}", extra.display());
    }

    // Whether the user explicitly forced a volume with `--volume` (as opposed
    // to us auto-detecting one), and whether it looked like an RP2350 -- both
    // feed into `flash_result` below.
    let forced_non_rp2350_volume;

    let target = match volume_override {
        Some(path) => {
            let info = std::fs::read_to_string(path.join("INFO_UF2.TXT")).with_context(|| {
                format!(
                    "{} does not look like an RP2350 bootloader volume (no INFO_UF2.TXT) -- \
                     pass the mount point of the RP2350 drive, not a plain directory",
                    path.display()
                )
            })?;
            let vol = volume::BootloaderVolume { path, info };
            forced_non_rp2350_volume = !vol.is_rp2350();
            vol
        }
        None => {
            forced_non_rp2350_volume = false;
            find_or_wait_for_volume(no_bootsel)?
        }
    };
    println!("Bootloader volume: {}", target.path.display());
    if !target.is_rp2350() {
        println!(
            "warning: this volume's INFO_UF2.TXT does not mention RP2350 (board-id={:?} model={:?}); \
             it may be a different board (e.g. an RP2040), which will silently ignore the RP2350 UF2 \
             this tool writes -- pass --volume to point at the exact RP2350 drive if this is wrong",
            target.board_id(),
            target.model(),
        );
    }
    let target_volume = target.path;

    let dest = target_volume.join("firmware.uf2");
    let synced = write_uf2_tolerant(&dest, &uf2_bytes)?;
    println!("Wrote {}", dest.display());

    let volume_gone = volume::wait_for_volume_gone(&target_volume, Duration::from_secs(5));
    if volume_gone {
        println!("Volume disappeared -- device is rebooting.");
    } else if !synced {
        // Per the datasheet (`research:uf2-flash` §5), a failed UF2 download
        // looks exactly like this: the device does not reboot and the drive
        // simply stays there, since "invalid UF2 files might not write at
        // all or only write partially" and the OS may not report a write
        // error. But a failed `sync_all` is *also* exactly what a successful
        // flash looks like when the volume was mounted by hand on headless
        // Linux (`volume::wait_for_volume_gone` can then never observe
        // "gone", since its `is_file()` check on a stale, manually-mounted
        // superblock keeps being served from cache): the device reboots
        // ~500ms after the last block and can drop the connection mid-sync
        // regardless. So this alone is not treated as a hard failure here --
        // `flash_result` below still gets a chance to see the controller
        // come back on serial before deciding.
        println!(
            "warning: writing {} did not fully sync and the bootloader volume is still present after \
             5s -- checking whether the controller comes back on serial before treating this as a \
             failure (a manually-mounted volume can make \"volume gone\" undetectable even after a \
             successful flash)",
            dest.display()
        );
    } else {
        println!(
            "(volume still present after 5s -- waiting to see if the controller comes back on serial \
             before deciding whether this is a real failure)"
        );
    }

    let port_came_back = match device::wait_for_serial_port(Duration::from_secs(5)) {
        Some(port) => {
            println!("Controller back on {}.", port.port_name);
            if let Ok(mut client) = device::SerialClient::open(&port.port_name) {
                if let Ok(resp) = client.send("INFO") {
                    println!("{resp}");
                }
            }
            true
        }
        None => {
            println!("(controller did not reappear as a serial port within 5s; it may still be booting)");
            false
        }
    };

    flash_result(volume_gone, synced, port_came_back, forced_non_rp2350_volume)
}

/// Decides `rcctl flash`'s final exit status from the observable signals a
/// UF2 download gives (`research:uf2-flash` §5): the bootloader volume
/// disappearing, or the controller reappearing as a serial port, are the only
/// two *positive* signals, and either one succeeds on its own regardless of
/// `synced` -- a failed final `sync_all` is tolerated (see `write_uf2_tolerant`
/// and the comment in `cmd_flash`) since it can happen on a perfectly
/// successful flash.
///
/// Neither positive signal appearing is exactly what a rejected or partial
/// UF2 download looks like ("it will appear as if nothing has happened since
/// the device will not reboot", and the OS may not report a write error) --
/// previously `cmd_flash` treated both of those as merely "informational" and
/// always returned `Ok(())`, so a script chaining on `rcctl flash`'s exit code
/// (as SPEC-controller.md §8 assumes "wait for the device to come back"
/// implies) could never detect a failed flash. A later fix then made
/// `cmd_flash` `bail!` immediately whenever `!synced && !volume_gone`, before
/// this function -- and therefore before `wait_for_serial_port` -- ever ran,
/// which reintroduced the same class of bug for exactly the case this
/// function is built to give a second chance to (a stale, manually-mounted
/// volume plus a `sync_all` failure): `cmd_flash` no longer takes that
/// shortcut, so `synced` is threaded through here purely for a clearer
/// failure message, never to skip this decision.
///
/// The one exception is a volume the user explicitly forced with `--volume`
/// that didn't look like an RP2350 to begin with (already warned about
/// above): we have no idea what such a board is supposed to do after being
/// written to, so this case is not escalated to a hard failure.
fn flash_result(
    volume_gone: bool,
    synced: bool,
    port_came_back: bool,
    forced_non_rp2350_volume: bool,
) -> Result<()> {
    if volume_gone || port_came_back {
        return Ok(());
    }
    if forced_non_rp2350_volume {
        println!(
            "warning: no positive sign the flash succeeded (the volume is still present and the \
             controller did not reappear on serial), but --volume pointed at a volume that didn't \
             look like an RP2350 to begin with, so this may be expected; not treating this as a failure."
        );
        return Ok(());
    }
    let sync_note = if synced {
        ""
    } else {
        " (the write to the volume also did not fully sync, though that alone can happen on a \
           successful flash too -- see README)"
    };
    bail!(
        "no positive sign the flash succeeded: the bootloader volume is still present and the \
         controller did not reappear as a serial port{sync_note}. Per the RP2350 datasheet, this is \
         exactly what a rejected or partial UF2 download looks like (the device does not reboot, and \
         the OS may not report a write error) -- check the volume and try again."
    );
}

/// True if `resp` is an `OK ...` response, case-insensitively (protocol
/// keywords are case-insensitive; see `retrocontroller_protocol::serial`).
fn is_ok_response(resp: &str) -> bool {
    resp.split_ascii_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case("OK"))
}

/// Tries to reboot a connected controller into BOOTSEL over serial; falls
/// back to asking the user to use the button combo. Either way, waits for
/// the bootloader volume to show up.
fn find_or_wait_for_volume(no_bootsel: bool) -> Result<volume::BootloaderVolume> {
    let mut rebooted = false;
    // Did we find a serial port and try (and fail) to use it, as opposed to
    // never finding one at all? These get different fallback messages.
    let mut attempted = false;

    if !no_bootsel {
        match device::find_serial_port() {
            Ok(port) => {
                attempted = true;
                match device::SerialClient::open(&port.port_name) {
                    Ok(mut client) => match client.send("BOOTSEL") {
                        Ok(resp) if is_ok_response(&resp) => {
                            println!(
                                "Sent BOOTSEL to {} ({resp}); waiting for the bootloader volume...",
                                port.port_name
                            );
                            rebooted = true;
                        }
                        Ok(resp) => println!(
                            "Controller on {} replied to BOOTSEL with `{resp}` (not OK); \
                             falling back to manual instructions.",
                            port.port_name
                        ),
                        Err(e) => {
                            println!("Could not send BOOTSEL ({e}); falling back to manual instructions.")
                        }
                    },
                    Err(e) => {
                        println!(
                            "Could not open {} ({e}); falling back to manual instructions.",
                            port.port_name
                        )
                    }
                }
            }
            Err(_) => { /* no controller on serial; fall through to manual instructions */ }
        }
    }

    let wait_secs = if rebooted { 10 } else { 60 };
    if !rebooted {
        // If a bootloader volume is already sitting there (e.g. the device
        // was already put into BOOTSEL manually, which is exactly why
        // `find_serial_port` above found nothing), don't tell the user to
        // press buttons on a board that's already waiting.
        if volume::find_volume().is_some() {
            println!("A bootloader volume is already present; waiting up to {wait_secs}s for it...");
        } else {
            let why = if attempted {
                "Could not put the controller into the bootloader over serial."
            } else {
                "No controller serial port found (or --no-bootsel given)."
            };
            println!(
                "{why}\n\
                 Hold the B (BOOT) button, tap R (RESET), then release B -- \
                 or hold B while plugging in USB -- to enter the bootloader.\n\
                 Waiting up to {wait_secs}s for the RP2350 volume..."
            );
        }
    }

    volume::wait_for_volume(Duration::from_secs(wait_secs)).ok_or_else(|| {
        anyhow::anyhow!(
            "timed out after {wait_secs}s waiting for the RP2350 bootloader volume -- on headless \
             Linux the volume may not auto-mount; try mounting it yourself, or pass \
             --volume <mountpoint> (skip detection) or --uf2-out <path> (just write the UF2 file)"
        )
    })
}

/// Writes the UF2 with a plain `File::create` + `write_all` + `sync_all`
/// (no extended attributes, which Finder/`cp` may fail on -- see
/// `research:uf2-flash` §6). I/O errors here can legitimately happen because
/// the device reboots ~500ms after the last block is written, so a failed
/// `sync_all` alone is tolerated (returns `Ok(false)`); it's `flash_result`'s
/// job to weigh that against the bootloader volume disappearing *and* the
/// controller reappearing on serial (the only observable signals a UF2 write
/// has -- see `cmd_flash`), not to treat a failed sync as a hard failure on
/// its own -- a stale, manually-mounted volume can make "volume gone"
/// undetectable even on a flash that otherwise succeeded.
fn write_uf2_tolerant(dest: &Path, bytes: &[u8]) -> Result<bool> {
    let mut file = std::fs::File::create(dest).with_context(|| format!("creating {}", dest.display()))?;
    if let Err(e) = file.write_all(bytes) {
        bail!("writing {}: {e}", dest.display());
    }
    Ok(file.sync_all().is_ok())
}

// ----------------------------------------------------------------- devices

fn cmd_devices() -> Result<()> {
    println!("{PID_CODES_WARNING}\n");

    println!("Serial ports:");
    let mut any_serial = false;
    for p in serialport::available_ports().context("listing serial ports")? {
        if let SerialPortType::UsbPort(u) = &p.port_type {
            if u.vid == usb::VID && u.pid == usb::PID {
                any_serial = true;
                println!("  {}  product={:?} serial={:?}", p.port_name, u.product, u.serial_number);
            }
        }
    }
    if !any_serial {
        println!("  (none found looking for USB {:04x}:{:04x})", usb::VID, usb::PID);
    }

    println!("HID devices:");
    let mut any_hid = false;
    match hidapi::HidApi::new() {
        Ok(api) => {
            for d in api.device_list() {
                if d.vendor_id() == usb::VID && d.product_id() == usb::PID {
                    any_hid = true;
                    println!(
                        "  {}  usage_page=0x{:02x} usage=0x{:02x} product={:?} serial={:?}",
                        d.path().to_string_lossy(),
                        d.usage_page(),
                        d.usage(),
                        d.product_string(),
                        d.serial_number(),
                    );
                }
            }
            if !any_hid {
                println!("  (none found looking for USB {:04x}:{:04x})", usb::VID, usb::PID);
            }
        }
        Err(e) => println!("  (could not enumerate HID devices: {e})"),
    }

    println!("Bootloader volumes:");
    let volumes = volume::find_volumes();
    if volumes.is_empty() {
        println!("  (none found)");
    } else {
        for v in &volumes {
            println!(
                "  {}  board-id={:?} model={:?} rp2350={}",
                v.path.display(),
                v.board_id(),
                v.model(),
                v.is_rp2350()
            );
        }
    }

    if !any_serial && !any_hid && volumes.is_empty() {
        // Enumeration itself (sysfs/uevent-based `serialport`/hidapi
        // backends, `/proc/mounts`/`/Volumes` for the bootloader volume)
        // never opens a device node, so a udev *permissions* problem cannot
        // be why nothing showed up here at all -- only later, when actually
        // opening a device this listing did find (see `rcctl info`/`test`/
        // `monitor`'s error messages, and `research:host-hid-input` §2).
        println!(
            "\nNothing found. Check the USB cable/port, and whether the board is sitting in the boot \
             ROM (it would show up as a bootloader volume instead of serial/HID, above) or unpowered."
        );
    }

    Ok(())
}

fn cmd_info() -> Result<()> {
    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    let resp = client.send("INFO")?;
    println!("{resp}");
    if !is_ok_response(&resp) {
        bail!("controller returned: {resp}");
    }
    Ok(())
}

// ----------------------------------------------------------- monitor / test

fn bar(value: u16, width: usize) -> String {
    let filled = ((value as usize) * width / hid::AXIS_MAX as usize).min(width);
    format!("{}{}", "#".repeat(filled), "-".repeat(width - filled))
}

const BAR_WIDTH: usize = 30;

fn install_ctrlc_flag() -> Result<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst)).context("installing Ctrl-C handler")?;
    Ok(stop)
}

fn cmd_monitor(raw: bool) -> Result<()> {
    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    let resp = client.send("STREAM ON")?;
    if !is_ok_response(&resp) {
        bail!("unexpected response to STREAM ON: {resp}");
    }

    let stop = install_ctrlc_flag()?;
    eprintln!("Streaming both knobs (Ctrl-C to stop)...");

    let result = (|| -> Result<()> {
        while !stop.load(Ordering::SeqCst) {
            match client.read_line()? {
                None => continue,
                Some(line) => {
                    if raw {
                        println!("{line}");
                    } else if let Some(sample) = serial::StreamSample::parse(&line) {
                        print!(
                            "\rL [{}] {:>4}   R [{}] {:>4}   ",
                            bar(sample.left, BAR_WIDTH),
                            sample.left,
                            bar(sample.right, BAR_WIDTH),
                            sample.right,
                        );
                        let _ = std::io::stdout().flush();
                    }
                }
            }
        }
        Ok(())
    })();

    let _ = client.send("STREAM OFF");
    println!();
    result
}

fn cmd_test() -> Result<()> {
    let dev = device::open_hid()?;
    let stop = install_ctrlc_flag()?;
    eprintln!("Reading HID reports -- what games see (Ctrl-C to stop)...");

    let mut last = hid::PaddleReport::default();
    while !stop.load(Ordering::SeqCst) {
        if let Some(r) = device::read_latest_report(&dev)? {
            last = r;
        }
        print!(
            "\rX [{}] {:>4}   Y [{}] {:>4}   ",
            bar(last.x, BAR_WIDTH),
            last.x,
            bar(last.y, BAR_WIDTH),
            last.y,
        );
        let _ = std::io::stdout().flush();
        std::thread::sleep(Duration::from_millis(33));
    }
    println!();
    Ok(())
}

// -------------------------------------------------------------- calibration

fn print_cal_response(resp: &str) {
    println!("{resp}");
    if let Some(cl) = serial::CalLine::parse(resp) {
        println!(
            "  left:  {}..{}{}",
            cl.cal.left.min,
            cl.cal.left.max,
            if cl.cal.left.invert { " (inverted)" } else { "" }
        );
        println!(
            "  right: {}..{}{}",
            cl.cal.right.min,
            cl.cal.right.max,
            if cl.cal.right.invert { " (inverted)" } else { "" }
        );
        println!("  swapped: {}", cl.cal.swap);
        println!("  saved: {}", cl.saved);
    }
}

fn execute_cal(action: CalAction, mut send: impl FnMut(&str) -> Result<String>) -> Result<String> {
    let cmd = match action {
        CalAction::Swap { on: Some(on) } => format!("CAL SWAP {}", on.protocol_digit()),
        CalAction::Swap { on: None } => {
            let response = send("CAL SHOW")?;
            let current = serial::CalLine::parse(&response)
                .with_context(|| format!("could not read current wheel assignment: {response}"))?;
            format!("CAL SWAP {}", u8::from(!current.cal.swap))
        }
        CalAction::Show => "CAL SHOW".to_string(),
        CalAction::Reset => "CAL RESET".to_string(),
        CalAction::Set { lmin, lmax, rmin, rmax } => format!("CAL SET {lmin} {lmax} {rmin} {rmax}"),
        CalAction::Invert { knob, on } => {
            format!("INVERT {} {}", knob.protocol_letter(), on.protocol_digit())
        }
    };
    let response = send(&cmd)?;
    if !is_ok_response(&response) {
        bail!("controller returned: {response}");
    }
    Ok(response)
}

fn cmd_cal(action: CalAction) -> Result<()> {
    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    let response = execute_cal(action, |cmd| client.send(cmd))?;
    print_cal_response(&response);
    Ok(())
}

fn cmd_calibrate() -> Result<()> {
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
    use std::time::Instant;

    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    // Install before starting learning; failure must not strand the device.
    let stop = install_ctrlc_flag()?;
    let (input_request, input_lines) = calibration_ui::input_worker();
    let tty = calibration_ui::can_redraw();
    let mut learning = false;
    let mut streaming = false;
    let mut displayed = false;
    let mut travel = [calibration_ui::Travel::default(); 2];
    println!("Sweep BOTH physical knobs slowly to each stop, in both directions, a few times.");
    println!("A0/A1 are physical inputs. Bars show raw ADC position; ADC % is observed span.");
    println!("Bars: | current, = observed travel, - unvisited ADC range.");
    println!("Observed span does not measure mechanical completion.");
    println!("Press Enter when finished sweeping (Ctrl-C cancels).");

    let learned = (|| -> Result<serial::CalLine> {
        learning = true;
        let resp = client.send("CAL START")?;
        if !is_ok_response(&resp) {
            bail!("CAL START failed: {resp}");
        }
        streaming = true;
        let resp = client.send("STREAM ON")?;
        if !is_ok_response(&resp) {
            bail!("STREAM ON failed: {resp}");
        }
        input_request.send(()).context("requesting Enter confirmation")?;
        let mut last_sample = Instant::now();
        let interval = Duration::from_millis(if tty { 100 } else { 1000 });
        let mut last_draw = Instant::now() - interval;
        loop {
            if stop.load(Ordering::SeqCst) {
                bail!("calibration interrupted; not saved");
            }
            match input_lines.try_recv() {
                Ok(Ok(Some(_))) => break,
                Ok(Ok(None)) => bail!("stdin closed; calibration not saved"),
                Ok(Err(error)) => return Err(error).context("reading Enter confirmation"),
                Err(TryRecvError::Disconnected) => bail!("confirmation reader stopped"),
                Err(TryRecvError::Empty) => {}
            }
            if let Some(line) = client.read_line()? {
                if let Some(sample) = serial::StreamSample::parse(&line) {
                    travel[0].observe(sample.left_raw);
                    travel[1].observe(sample.right_raw);
                    last_sample = Instant::now();
                } else if line.starts_with("ERR") {
                    bail!("controller returned: {line}");
                }
            }
            if last_sample.elapsed() > Duration::from_secs(3) {
                bail!("no calibration samples for 3 seconds; controller may have disconnected");
            }
            if last_draw.elapsed() >= interval {
                if tty && displayed {
                    print!("\x1b[2A");
                }
                for (axis, label) in travel.iter().zip(["A0", "A1"]) {
                    if tty {
                        print!("\r\x1b[2K");
                    }
                    println!("{}", axis.line(label));
                }
                std::io::stdout().flush().context("drawing calibration feedback")?;
                displayed = true;
                last_draw = Instant::now();
            }
        }
        let resp = calibration_ui::cleanup(&mut streaming, &mut learning, |cmd| client.send(cmd))?
            .context("missing CAL STOP response")?;
        if !is_ok_response(&resp) {
            bail!("CAL STOP failed: {resp}");
        }
        serial::CalLine::parse(&resp).with_context(|| format!("could not parse CAL STOP response: {resp}"))
    })();
    // Cleanup runs after every exit, including setup failure, EOF, and unplug.
    let cleanup = calibration_ui::cleanup(&mut streaming, &mut learning, |cmd| client.send(cmd));
    if let Err(error) = cleanup {
        eprintln!("Could not confirm calibration cleanup: {error:#}. Reconnect or reboot the controller if its learning LED remains active.");
        if learned.is_ok() {
            return Err(error);
        }
    }
    let cl = learned?;
    if stop.load(Ordering::SeqCst) {
        bail!("calibration interrupted; not saved");
    }
    println!("Learned physical ranges:");
    println!("  A0: {}..{}", cl.cal.left.min, cl.cal.left.max);
    println!("  A1: {}..{}", cl.cal.right.min, cl.cal.right.max);
    print!("Save this calibration to flash? [y/N] ");
    std::io::stdout().flush().context("displaying save prompt")?;
    input_request.send(()).context("requesting save confirmation")?;
    let answer = loop {
        if stop.load(Ordering::SeqCst) {
            bail!("calibration interrupted; not saved");
        }
        match input_lines.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(Some(answer))) => break answer,
            Ok(Ok(None)) => bail!("stdin closed; calibration not saved"),
            Ok(Err(error)) => return Err(error).context("reading save confirmation"),
            Err(RecvTimeoutError::Disconnected) => bail!("confirmation reader stopped"),
            Err(RecvTimeoutError::Timeout) => {}
        }
    };
    if stop.load(Ordering::SeqCst) {
        bail!("calibration interrupted; not saved");
    }
    if answer.trim().eq_ignore_ascii_case("y") {
        let resp = client.send("CAL SAVE")?;
        if !is_ok_response(&resp) {
            bail!("CAL SAVE failed: {resp}");
        }
        println!("{resp}");
    } else {
        println!("Not saved (calibration is live until reboot; `rcctl cal show` / `rcctl cal set` to manage it manually).");
    }
    Ok(())
}

fn cmd_bootsel() -> Result<()> {
    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    let resp = client.send("BOOTSEL")?;
    println!("{resp}");
    if !is_ok_response(&resp) {
        bail!("controller returned: {resp}");
    }
    Ok(())
}

fn cmd_reboot() -> Result<()> {
    let port = device::find_serial_port()?;
    let mut client = device::SerialClient::open(&port.port_name)?;
    let resp = client.send("REBOOT")?;
    println!("{resp}");
    if !is_ok_response(&resp) {
        bail!("controller returned: {resp}");
    }
    Ok(())
}

// --------------------------------------------------------------------- udev

/// Must sort before `73-seat-late.rules` for `TAG+="uaccess"` to take
/// effect (`research:host-hid-input` §2), hence `70-`, not `99-`.
const UDEV_RULES_PATH: &str = "/etc/udev/rules.d/70-retrocontroller.rules";

// `GROUP="input"`, not `GROUP="plugdev"`: `plugdev` is a Debian/Ubuntu
// convention and doesn't exist by default on Fedora, Arch, openSUSE etc. --
// when udev can't resolve a `GROUP=` name it logs "unknown group" and leaves
// the node's *previous* owner in place, which for `/dev/hidrawN` (there's no
// other default rule for it) is `root:root`, so the node ends up exactly as
// inaccessible as with no rule at all, and the README's
// `usermod -aG <group>,dialout` fallback then fails with "group '<group>'
// does not exist" on those distros. `input` is created by systemd's own
// default rules (it's the group already used for `/dev/input/event*`,
// `research:host-hid-input` §2) on every systemd distro, Debian/Ubuntu
// included, so it works everywhere `TAG+="uaccess"` doesn't already cover the
// node (non-systemd/non-logind setups -- see the `SUBSYSTEM=="input"` rule's
// own comment below).
const UDEV_RULES: &str = r#"# RetroController (VID:PID 1209:0001) -- generated by `rcctl udev`.
# Must sort before 73-seat-late.rules for TAG+="uaccess" to apply.

# HID (hidraw, used by hidapi / rcctl)
KERNEL=="hidraw*", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", MODE="0660", GROUP="input", TAG+="uaccess"
# Same device: evdev node for gilrs/SDL (usually already covered by
# 70-uaccess's ID_INPUT_JOYSTICK rule; kept for non-systemd setups)
SUBSYSTEM=="input", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", MODE="0660", GROUP="input", TAG+="uaccess"
# CDC-ACM serial port (rcctl's INFO/CAL/STREAM commands); keep ModemManager away
SUBSYSTEM=="tty", ATTRS{idVendor}=="1209", ATTRS{idProduct}=="0001", MODE="0660", GROUP="input", TAG+="uaccess", ENV{ID_MM_DEVICE_IGNORE}="1"
# RP2350 BOOTSEL (UF2 mass storage + PICOBOOT); RP2040's is 2e8a:0003
SUBSYSTEM=="usb", ATTRS{idVendor}=="2e8a", ATTRS{idProduct}=="000f", MODE="0660", GROUP="input", TAG+="uaccess"
"#;

// UDEV_RULES_PATH/UDEV_RULES are referenced on every platform (not just
// behind `#[cfg(target_os = "linux")] fn cmd_udev`, below) so that someone
// preparing a Linux box from a macOS host can still see the rules -- which
// also means they're never `dead_code` when cross-compiled for a non-Linux
// target, unlike an earlier version of this file that only referenced them
// from the Linux-only `cmd_udev`/`install_udev_rules` and left rustc
// warning `constant UDEV_RULES is never used` on other platforms.
#[cfg(not(target_os = "linux"))]
fn cmd_udev(install: bool) -> Result<()> {
    print!("{UDEV_RULES}");
    if install {
        println!(
            "(udev rules only apply on Linux -- copy the above to \
             {UDEV_RULES_PATH} on the target Linux machine and run \
             `udevadm control --reload-rules && udevadm trigger` there)"
        );
    } else {
        println!("(udev rules are a Linux-only concept; shown here in case you're preparing a Linux box)");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn cmd_udev(install: bool) -> Result<()> {
    print!("{UDEV_RULES}");
    if install {
        install_udev_rules()?;
    } else {
        println!("(pass --install to write this to {UDEV_RULES_PATH} and reload udev; needs root)");
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_udev_rules() -> Result<()> {
    match std::fs::write(UDEV_RULES_PATH, UDEV_RULES) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            bail!("permission denied writing {UDEV_RULES_PATH} -- try `sudo rcctl udev --install`");
        }
        Err(e) => return Err(e).with_context(|| format!("writing {UDEV_RULES_PATH}")),
    }
    println!("Wrote {UDEV_RULES_PATH}");

    for (cmd, args) in
        [("udevadm", ["control", "--reload-rules"].as_slice()), ("udevadm", ["trigger"].as_slice())]
    {
        match std::process::Command::new(cmd).args(args).status() {
            Ok(s) if s.success() => {}
            Ok(s) => println!("warning: `{cmd} {}` exited with {s}", args.join(" ")),
            Err(e) => println!("warning: could not run `{cmd} {}`: {e}", args.join(" ")),
        }
    }
    println!("Replug the controller for the new rules to take effect.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ok_response_matches_case_insensitively() {
        assert!(is_ok_response("OK BOOTSEL"));
        assert!(is_ok_response("ok bootsel"));
        assert!(is_ok_response("Ok"));
        assert!(!is_ok_response("ERR unknown command"));
        assert!(!is_ok_response(""));
        assert!(!is_ok_response("OKAY THIS LOOKS LIKE OK BUT ISN'T"));
    }

    /// Before this fix, `rcctl flash` always returned `Ok(())` regardless of
    /// whether the volume disappeared or the controller reappeared on
    /// serial, so a caller could never detect that every observable signal
    /// said the download failed (`research:uf2-flash` §5: a rejected/partial
    /// UF2 download looks exactly like nothing happening).
    #[test]
    fn flash_result_fails_when_neither_positive_signal_is_observed() {
        let err = flash_result(false, true, false, false).unwrap_err();
        assert!(err.to_string().contains("no positive sign"), "unexpected error: {err}");
    }

    #[test]
    fn flash_result_succeeds_when_volume_disappeared() {
        flash_result(true, true, false, false).expect("volume gone is a positive signal on its own");
    }

    #[test]
    fn flash_result_succeeds_when_serial_port_came_back() {
        flash_result(false, true, true, false).expect("port back is a positive signal on its own");
    }

    #[test]
    fn flash_result_succeeds_when_both_positive_signals_seen() {
        flash_result(true, true, true, false).expect("both signals together must still succeed");
    }

    /// A `--volume` the user explicitly pointed at a volume that didn't even
    /// look like an RP2350 is not escalated to a hard failure -- we don't
    /// know what such a board is supposed to do after being written to.
    #[test]
    fn flash_result_does_not_fail_for_a_forced_non_rp2350_volume() {
        flash_result(false, true, false, true)
            .expect("a forced non-RP2350 volume should not be a hard failure");
    }

    /// The exact scenario from the bug report: `sync_all` failed (e.g. the
    /// device rebooted mid-sync, or a stale manually-mounted volume made
    /// "volume gone" undetectable), yet the controller *did* reappear on
    /// serial. Before this fix, `cmd_flash` would `bail!` in this situation
    /// before `flash_result` was ever called (it never even ran
    /// `wait_for_serial_port`), reporting a successful flash as a hard
    /// failure. `flash_result` itself must treat the serial-port signal as
    /// authoritative regardless of `synced`.
    #[test]
    fn flash_result_succeeds_when_port_came_back_even_if_sync_failed() {
        flash_result(false, false, true, false)
            .expect("the controller reappearing on serial must succeed even when sync_all failed");
    }

    /// Symmetric failure case: nothing synced, the volume is still present,
    /// and the controller never reappeared -- still a real failure, and the
    /// message should mention the sync failure for a better diagnostic.
    #[test]
    fn flash_result_fails_and_mentions_sync_when_nothing_synced_or_came_back() {
        let err = flash_result(false, false, false, false).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("no positive sign"), "unexpected error: {msg}");
        assert!(msg.contains("did not fully sync"), "unexpected error: {msg}");
    }

    /// Before `KnobArg`/`OnOffArg` were `ValueEnum`s, `cal invert` took two
    /// bare `String`s, so `rcctl cal invert X 5` parsed fine locally and
    /// only failed after a serial round-trip with `ERR bad argument`. clap
    /// must now reject it up front.
    #[test]
    fn swap_toggles_current_device_assignment_on_each_invocation() {
        let mut swapped = false;
        for expected in [true, false, true] {
            let mut calls = Vec::new();
            execute_cal(CalAction::Swap { on: None }, |cmd| {
                calls.push(cmd.to_owned());
                if cmd.starts_with("CAL SWAP ") {
                    swapped = cmd.ends_with('1');
                }
                Ok(format!("OK CAL 1000 64535 1000 64535 0 0 1 {}", u8::from(swapped)))
            })
            .unwrap();
            assert_eq!(swapped, expected);
            assert_eq!(calls, ["CAL SHOW".to_owned(), format!("CAL SWAP {}", u8::from(expected))]);
        }
    }

    #[test]
    fn swap_does_not_write_if_current_assignment_cannot_be_read() {
        for response in ["ERR flash", "garbage", "OK CAL incomplete"] {
            let mut calls = Vec::new();
            assert!(execute_cal(CalAction::Swap { on: None }, |cmd| {
                calls.push(cmd.to_owned());
                Ok(response.to_owned())
            })
            .is_err());
            assert_eq!(calls, ["CAL SHOW"]);
        }
        assert!(execute_cal(CalAction::Swap { on: None }, |cmd| {
            Ok(if cmd == "CAL SHOW" { "OK CAL 1000 64535 1000 64535 0 0 1 0" } else { "ERR flash" }.into())
        })
        .is_err());
    }

    #[test]
    fn cal_swap_accepts_on_off_and_rejects_invalid_values() {
        assert!(Cli::try_parse_from(["rcctl", "cal", "swap"]).is_ok());
        for value in ["on", "off", "1", "0", "ON", "OFF"] {
            assert!(Cli::try_parse_from(["rcctl", "cal", "swap", value]).is_ok());
        }
        assert!(Cli::try_parse_from(["rcctl", "cal", "swap", "yes"]).is_err());
    }

    #[test]
    fn cal_invert_rejects_arbitrary_strings() {
        assert!(Cli::try_parse_from(["rcctl", "cal", "invert", "X", "5"]).is_err());
        assert!(Cli::try_parse_from(["rcctl", "cal", "invert", "l", "notabool"]).is_err());
    }

    #[test]
    fn cal_invert_accepts_documented_values() {
        for (knob, on) in
            [("l", "1"), ("L", "on"), ("left", "ON"), ("r", "0"), ("R", "off"), ("right", "OFF")]
        {
            Cli::try_parse_from(["rcctl", "cal", "invert", knob, on])
                .unwrap_or_else(|e| panic!("`{knob} {on}` should parse: {e}"));
        }
    }

    #[test]
    fn firmware_dir_is_scoped_to_build_and_flash() {
        Cli::try_parse_from(["rcctl", "build", "--firmware-dir", "/tmp/fw"])
            .expect("--firmware-dir should be accepted on `build`");
        Cli::try_parse_from(["rcctl", "flash", "--firmware-dir", "/tmp/fw"])
            .expect("--firmware-dir should be accepted on `flash`");
        // It used to be a top-level `global = true` flag, so it appeared
        // (uselessly) on every subcommand's help and command line, e.g.
        // `monitor`, which has nothing to do with the firmware crate.
        assert!(
            Cli::try_parse_from(["rcctl", "monitor", "--firmware-dir", "/tmp/fw"]).is_err(),
            "--firmware-dir should not be accepted on `monitor`"
        );
        assert!(
            Cli::try_parse_from(["rcctl", "udev", "--firmware-dir", "/tmp/fw"]).is_err(),
            "--firmware-dir should not be accepted on `udev`"
        );
    }

    /// Before this fix, `rcctl uf2 --family rp2350-arm-ns` exited 0 printing
    /// only `Wrote ... family rp2350-arm-ns` -- no indication that a bare
    /// (no partition table) RP2350 bootrom silently drops every block of
    /// that family (`research:uf2-flash` §2).
    #[test]
    fn warns_for_rp2350_arm_ns_family_needing_a_partition_table() {
        let warning = family_partition_table_warning(uf2::Family::Rp2350ArmNs)
            .expect("rp2350-arm-ns needs a partition table on a bare board");
        assert!(warning.contains("rp2350-arm-ns"), "unexpected warning: {warning}");
        assert!(warning.contains("partition table"), "unexpected warning: {warning}");
    }

    #[test]
    fn no_partition_table_warning_for_other_families() {
        for family in [uf2::Family::Absolute, uf2::Family::Data, uf2::Family::Rp2350ArmS] {
            assert_eq!(
                family_partition_table_warning(family),
                None,
                "family {family} should not warn about a partition table"
            );
        }
    }

    /// Before this fix, every rule used `GROUP="plugdev"`, which doesn't
    /// exist by default on Fedora/Arch/openSUSE etc.; udev then leaves the
    /// node `root:root` there and the README's `usermod -aG plugdev,...`
    /// fallback fails with "group 'plugdev' does not exist".
    #[test]
    fn udev_rules_do_not_reference_the_debian_specific_plugdev_group() {
        assert!(!UDEV_RULES.contains("plugdev"), "plugdev is not portable across distros: {UDEV_RULES}");
        assert!(UDEV_RULES.contains(r#"GROUP="input""#), "expected the portable `input` group: {UDEV_RULES}");
    }

    #[test]
    fn build_accepts_release_flag_for_spec_compatibility() {
        // SPEC-controller.md §8 documents `rcctl build [--release]`.
        Cli::try_parse_from(["rcctl", "build", "--release"]).expect("--release should be accepted");
        Cli::try_parse_from(["rcctl", "build"]).expect("--release should be optional");
    }
}
