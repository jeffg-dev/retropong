//! Finding the RP2350 ROM bootloader's mass-storage volume.
//!
//! Detection is by the presence of `INFO_UF2.TXT` in a candidate directory
//! (per `research:uf2-flash` §5 and SPEC-controller.md §7), not by volume
//! label alone, since the label can be OTP white-labelled.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Poll interval used by [`wait_for_volume`] / [`wait_for_volume_gone`].
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// A detected bootloader volume and the contents of its `INFO_UF2.TXT`.
#[derive(Clone, Debug)]
pub struct BootloaderVolume {
    pub path: PathBuf,
    pub info: String,
}

impl BootloaderVolume {
    fn field(&self, prefix: &str) -> Option<&str> {
        self.info.lines().find_map(|l| l.strip_prefix(prefix)).map(str::trim)
    }

    /// The `Board-ID:` line, if present.
    pub fn board_id(&self) -> Option<&str> {
        self.field("Board-ID:")
    }

    /// The `Model:` line, if present.
    pub fn model(&self) -> Option<&str> {
        self.field("Model:")
    }

    /// Whether `INFO_UF2.TXT` mentions RP2350 anywhere (the volume label,
    /// board ID and model are all technically overridable via OTP
    /// white-labelling, so this is treated as a hint, not a guarantee).
    pub fn is_rp2350(&self) -> bool {
        self.info.contains("RP2350")
    }
}

/// Decodes octal escapes (`\NNN`, e.g. `\040` for a space) as used by
/// `/proc/mounts` for mount points containing special characters.
fn decode_mount_escapes(s: &str) -> PathBuf {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(u8::is_ascii_digit) {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 4]).unwrap(), 8) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(OsString::from_vec(out))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(&out).into_owned())
    }
}

/// Filesystem types (`/proc/mounts` field 3) the RP2350/RP2040 bootrom's
/// mass-storage volume can mount as (`research:uf2-flash` §5: "formatted as
/// a single partition with FAT16").
const FAT_FSTYPES: &[&str] = &["vfat", "msdos", "fat"];

/// Parses `/proc/mounts`-format text (`device mountpoint fstype options dump
/// pass`, one per line) into the mount points whose filesystem type is one of
/// [`FAT_FSTYPES`].
///
/// Filtering by fstype, instead of taking every mount point unconditionally,
/// both skips the pseudo-filesystems (`proc`, `sysfs`, `devtmpfs`, cgroup2,
/// ...) that `/proc/mounts` is otherwise full of and -- more importantly --
/// avoids ever calling `read_to_string(dir.join("INFO_UF2.TXT"))` on a
/// network mount (NFS/CIFS/sshfs) or an autofs trigger directory. Without
/// this filter, a hung network mount (or autofs actually mounting something)
/// sitting anywhere on the system would make [`find_volumes`]'s 250 ms poll
/// loop (used by [`wait_for_volume`], for up to 60 s) block indefinitely.
fn parse_proc_mounts(contents: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for line in contents.lines() {
        let mut fields = line.split_whitespace();
        let _device = fields.next();
        let Some(mountpoint) = fields.next() else { continue };
        let Some(fstype) = fields.next() else { continue };
        if FAT_FSTYPES.contains(&fstype) {
            dirs.push(decode_mount_escapes(mountpoint));
        }
    }
    dirs
}

/// Filesystem types (as macOS's `mount`(8) prints them) the RP2350/RP2040
/// bootrom's mass-storage volume can mount as on macOS. `cfg`-gated (rather
/// than plain `pub`/`#[allow(dead_code)]`) alongside [`parse_macos_mount_output`]
/// below: on a non-macOS target outside `cfg(test)`, nothing calls either one
/// (see the non-macOS [`macos_volume_dirs`]), and this is a binary crate, so
/// they'd otherwise be flagged as genuinely dead code rather than kept for
/// documentation/API-surface reasons.
#[cfg(any(target_os = "macos", test))]
const MACOS_FAT_FSTYPES: &[&str] = &["msdos", "exfat"];

/// Parses macOS `mount`(8) output (`device on /path (fstype, opt, opt, ...)`,
/// one per line) into the mount points whose filesystem type is one of
/// [`MACOS_FAT_FSTYPES`].
///
/// Mirrors [`parse_proc_mounts`]'s Linux filter, and for the same reason:
/// without it, [`candidate_dirs`] took *every* entry of `/Volumes`
/// unconditionally, and macOS mounts network shares (SMB/AFP/NFS) there
/// exactly like any other volume -- so one unreachable share sitting under
/// `/Volumes` would make the first `read_to_string(dir.join("INFO_UF2.TXT"))`
/// in [`find_volumes`]'s 250ms poll loop (run for up to 60s by
/// [`wait_for_volume`]) hang, blocking `rcctl flash`/`rcctl devices`
/// indefinitely -- the same class of bug `parse_proc_mounts` already guards
/// against on Linux.
///
/// This is pure text parsing, kept buildable under `cfg(test)` on any host
/// (see [`MACOS_FAT_FSTYPES`]'s doc comment) so it's exercised directly by
/// tests without needing to actually run on macOS; only [`macos_volume_dirs`],
/// which shells out to the real `mount` binary, is macOS-only.
#[cfg(any(target_os = "macos", test))]
fn parse_macos_mount_output(contents: &str) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for line in contents.lines() {
        let Some((_device, after_on)) = line.split_once(" on ") else { continue };
        let Some(open_paren) = after_on.rfind('(') else { continue };
        let path = after_on[..open_paren].trim_end();
        let opts = after_on[open_paren + 1..].trim_end_matches(')');
        let Some(fstype) = opts.split(',').next() else { continue };
        if MACOS_FAT_FSTYPES.contains(&fstype.trim()) {
            dirs.push(PathBuf::from(path));
        }
    }
    dirs
}

/// Directories under `/Volumes` worth checking for `INFO_UF2.TXT`, filtered
/// to FAT/exFAT mounts by shelling out to `mount`(8) (see
/// [`parse_macos_mount_output`]). If `mount` can't be run or its output can't
/// be parsed, falls back to an unfiltered directory listing -- no worse than
/// before this fix, rather than finding nothing at all.
#[cfg(target_os = "macos")]
fn macos_volume_dirs() -> Vec<PathBuf> {
    if let Ok(output) = std::process::Command::new("mount").output() {
        if output.status.success() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                return parse_macos_mount_output(&text);
            }
        }
    }
    std::fs::read_dir("/Volumes")
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// On non-macOS platforms `/Volumes` normally doesn't exist at all; keep the
/// previous unfiltered behaviour (there's no `mount`-based filter to apply).
#[cfg(not(target_os = "macos"))]
fn macos_volume_dirs() -> Vec<PathBuf> {
    std::fs::read_dir("/Volumes")
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Every directory worth checking for `INFO_UF2.TXT`: `/Volumes/*` (macOS,
/// filtered to FAT/exFAT mounts -- see [`macos_volume_dirs`]), `/media/$USER/*`
/// and `/run/media/$USER/*` (common Linux desktop conventions), plus every
/// FAT-formatted mount point in `/proc/mounts` (covers headless Linux /
/// distros using neither convention) -- see [`parse_proc_mounts`] for why
/// that last one is filtered by filesystem type.
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = macos_volume_dirs();

    if let Ok(user) = std::env::var("USER") {
        for base in ["/media", "/run/media"] {
            if let Ok(entries) = std::fs::read_dir(Path::new(base).join(&user)) {
                dirs.extend(entries.flatten().map(|e| e.path()));
            }
        }
    }

    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        dirs.extend(parse_proc_mounts(&mounts));
    }

    dirs.sort();
    dirs.dedup();
    dirs
}

/// Scans all candidate directories and returns every one that looks like an
/// RP2350 (or RP2040) UF2 bootloader volume, RP2350 volumes first (see
/// [`find_volume`]).
pub fn find_volumes() -> Vec<BootloaderVolume> {
    let mut volumes: Vec<_> = candidate_dirs()
        .into_iter()
        .filter_map(|dir| {
            let info = std::fs::read_to_string(dir.join("INFO_UF2.TXT")).ok()?;
            Some(BootloaderVolume { path: dir, info })
        })
        .collect();
    // Stable sort: RP2350 volumes first, ties broken by the existing
    // (alphabetical-by-path) order from `candidate_dirs`.
    volumes.sort_by_key(|v| !v.is_rp2350());
    volumes
}

/// Convenience: the best bootloader volume found, if any -- an RP2350 one is
/// preferred over e.g. a stray RP2040 (`RPI-RP2`) volume mounted alongside it
/// (SPEC-controller.md §7: the `RP2350` label/board id is "a hint"; flashing
/// the wrong board's bootloader silently does nothing since the RP2350 UF2's
/// family ID isn't one the RP2040 bootrom accepts).
pub fn find_volume() -> Option<BootloaderVolume> {
    find_volumes().into_iter().next()
}

/// Polls every [`POLL_INTERVAL`] until a bootloader volume appears or
/// `timeout` elapses.
pub fn wait_for_volume(timeout: Duration) -> Option<BootloaderVolume> {
    let start = Instant::now();
    loop {
        if let Some(v) = find_volume() {
            return Some(v);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Polls until `path` no longer looks like a bootloader volume (the device
/// has rebooted) or `timeout` elapses. Returns `true` if it disappeared.
pub fn wait_for_volume_gone(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if !path.join("INFO_UF2.TXT").is_file() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Before this fix, `candidate_dirs` took every `/proc/mounts` mount
    /// point unconditionally, including `/`, `/proc`, `/sys`, cgroups, and
    /// any NFS/CIFS/sshfs/autofs mount, and `find_volumes` would then
    /// `read_to_string` `INFO_UF2.TXT` under each of them -- inside a 250 ms
    /// poll loop run for up to 60 s by `wait_for_volume`. A hung network
    /// mount or an autofs trigger directory would block `rcctl flash`/
    /// `rcctl devices` indefinitely.
    #[test]
    fn parse_proc_mounts_keeps_only_fat_filesystems() {
        let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
cgroup2 /sys/fs/cgroup cgroup2 rw,nosuid,nodev,noexec,relatime 0 0
fileserver:/export /mnt/nfs nfs4 rw,relatime 0 0
//server/share /mnt/cifs cifs rw,relatime 0 0
sshfs#user@host: /mnt/sshfs fuse.sshfs rw,nosuid,nodev,relatime 0 0
automount(pid1234) /mnt/auto autofs rw,relatime 0 0
/dev/sdb1 /media/jeff/RP2350 vfat rw,relatime 0 0
/dev/sdc1 /media/jeff/OLD-BOOTLOADER msdos rw,relatime 0 0
";
        let dirs = parse_proc_mounts(mounts);
        assert_eq!(
            dirs,
            vec![PathBuf::from("/media/jeff/RP2350"), PathBuf::from("/media/jeff/OLD-BOOTLOADER")]
        );
    }

    #[test]
    fn parse_proc_mounts_ignores_malformed_lines() {
        assert_eq!(parse_proc_mounts("\n   \nshort line\n"), Vec::<PathBuf>::new());
    }

    #[test]
    fn parse_proc_mounts_decodes_escapes_in_the_mountpoint_field() {
        let mounts = r"/dev/sdb1 /media/jeff/My\040Drive vfat rw,relatime 0 0".to_string() + "\n";
        assert_eq!(parse_proc_mounts(&mounts), vec![PathBuf::from("/media/jeff/My Drive")]);
    }

    /// Before this fix, `candidate_dirs` took every entry of `/Volumes`
    /// unconditionally, so a hung network share (SMB/AFP/NFS) mounted
    /// alongside a real bootloader volume could block `find_volumes`'s
    /// 250ms poll loop the same way an unfiltered `/proc/mounts` entry could
    /// on Linux (see `parse_proc_mounts_keeps_only_fat_filesystems`).
    #[test]
    fn parse_macos_mount_output_keeps_only_fat_and_exfat_filesystems() {
        let mount_output = "\
/dev/disk1s1 on / (apfs, local, journaled)
devfs on /dev (devfs, local, nobrowse)
/dev/disk4s1 on /Volumes/RP2350 (msdos, local, nodev, nosuid, removable)
/dev/disk5s1 on /Volumes/BigDrive (exfat, local, nodev, nosuid, removable)
//jeff@server/share on /Volumes/NetworkShare (smbfs, nodev, nosuid, mounted by jeff)
server:/export on /Volumes/nfs-share (nfs)
";
        let dirs = parse_macos_mount_output(mount_output);
        assert_eq!(dirs, vec![PathBuf::from("/Volumes/RP2350"), PathBuf::from("/Volumes/BigDrive")]);
    }

    #[test]
    fn parse_macos_mount_output_ignores_malformed_lines() {
        assert_eq!(parse_macos_mount_output("\nnot a mount line\n"), Vec::<PathBuf>::new());
    }

    #[test]
    fn decodes_octal_escapes() {
        assert_eq!(decode_mount_escapes(r"/media/jeff/RP2350"), PathBuf::from("/media/jeff/RP2350"));
        assert_eq!(decode_mount_escapes(r"/media/jeff/My\040Drive"), PathBuf::from("/media/jeff/My Drive"));
        // /proc/mounts escapes a literal backslash as its own octal code (\134).
        assert_eq!(decode_mount_escapes(r"/media/back\134slash"), PathBuf::from(r"/media/back\slash"));
    }

    #[test]
    fn board_info_parses_fields() {
        let v = BootloaderVolume {
            path: PathBuf::from("/media/x/RP2350"),
            info: "UF2 Bootloader v1.1\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n".to_string(),
        };
        assert_eq!(v.board_id(), Some("RP2350"));
        assert_eq!(v.model(), Some("Raspberry Pi RP2350"));
        assert!(v.is_rp2350());
    }

    #[test]
    fn non_rp2350_info_is_reported_as_such() {
        let v = BootloaderVolume {
            path: PathBuf::from("/media/x/RPI-RP2"),
            info: "UF2 Bootloader v1.2\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n".to_string(),
        };
        assert!(!v.is_rp2350());
    }

    /// `find_volumes`' RP2350-first ordering, tested directly on the sort
    /// step (the discovery half needs real mount points). Before this fix,
    /// `find_volume()` was just "the first volume found in path order",
    /// so an alphabetically-earlier RP2040 (`RPI-RP2`) mounted alongside an
    /// RP2350 would be preferred and silently ignore the RP2350 UF2 rcctl
    /// writes.
    #[test]
    fn rp2350_volumes_sort_before_others() {
        let rp2040 = BootloaderVolume {
            path: PathBuf::from("/media/x/AAAA-RP2040-sorts-first-alphabetically"),
            info: "UF2 Bootloader v1.2\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n".to_string(),
        };
        let rp2350 = BootloaderVolume {
            path: PathBuf::from("/media/x/ZZZZ-RP2350-sorts-last-alphabetically"),
            info: "UF2 Bootloader v1.1\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n".to_string(),
        };
        let mut volumes = [rp2040.clone(), rp2350.clone()];
        volumes.sort_by_key(|v| !v.is_rp2350());
        assert_eq!(volumes[0].path, rp2350.path, "the RP2350 volume must be preferred");
        assert_eq!(volumes[1].path, rp2040.path);
    }
}
