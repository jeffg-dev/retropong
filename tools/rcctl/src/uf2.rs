//! Pure ELF -> UF2 converter for the RP2350 boot ROM.
//!
//! See `docs/SPEC-controller.md` §7 for the user-facing summary and the
//! research notes (`research:uf2-flash`) for the verified block format,
//! family IDs and bootrom acceptance rules this module implements.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use elf::abi::{EM_ARM, ET_EXEC, PT_LOAD};
use elf::endian::AnyEndian;
use elf::file::Class;
use elf::ElfBytes;

/// UF2 block magic numbers (Microsoft UF2 spec / pico-sdk `boot/uf2.h`).
pub const MAGIC_START0: u32 = 0x0A32_4655;
pub const MAGIC_START1: u32 = 0x9E5D_5157;
pub const MAGIC_END: u32 = 0x0AB1_6F30;

/// UF2 block flags (`boot/uf2.h`). Only [`FLAG_FAMILY_ID_PRESENT`] and
/// [`FLAG_EXTENSION_FLAGS_PRESENT`] are ever set by this module; the rest
/// are documented here for reference (e.g. when reading a foreign UF2 with
/// [`parse_uf2`]).
#[allow(dead_code)]
pub const FLAG_NOT_MAIN_FLASH: u32 = 0x0000_0001;
#[allow(dead_code)]
pub const FLAG_FILE_CONTAINER: u32 = 0x0000_1000;
pub const FLAG_FAMILY_ID_PRESENT: u32 = 0x0000_2000;
#[allow(dead_code)]
pub const FLAG_MD5_PRESENT: u32 = 0x0000_4000;
pub const FLAG_EXTENSION_FLAGS_PRESENT: u32 = 0x0000_8000;

/// Extension tag that marks a block as ignorable by the bootrom (used by the
/// RP2350-E10 absolute-block workaround).
pub const UF2_EXTENSION_RP2_IGNORE_BLOCK: u32 = 0x9957_E304;

/// Total size of one UF2 block on disk.
pub const BLOCK_SIZE: usize = 512;
/// Size of the `data` area within a block (payload + trailing padding).
pub const DATA_AREA_SIZE: usize = 476;
/// RP2350 always uses a 256-byte payload per block.
pub const PAGE_SIZE: usize = 256;

/// Flash address window accepted for a flashable image (RP2350 datasheet
/// §5.5.2 / SPEC-controller.md §7). Content whose load address (LMA) falls
/// outside this range cannot be produced as a flashing UF2 by this tool.
pub const FLASH_START: u32 = 0x1000_0000;
pub const FLASH_END: u32 = 0x1200_0000;

/// Location and shape of the picotool-compatible RP2350-E10 "absolute
/// block" workaround (see `research:uf2-flash` §3).
pub const ABS_BLOCK_TARGET: u32 = 0x10FF_FF00;
pub const ABS_BLOCK_FLAGS: u32 = FLAG_FAMILY_ID_PRESENT | FLAG_EXTENSION_FLAGS_PRESENT;
pub const ABS_BLOCK_NUM_BLOCKS: u32 = 2;
pub const ABS_BLOCK_FILL: u8 = 0xEF;

/// UF2 family IDs relevant to RP2350 (verified against `boot/uf2.h`, the
/// RP2350 datasheet Table 455 and the bootrom's static asserts).
///
/// `rp2040` and `rp2350-riscv` are deliberately not offered here (see
/// `docs/SPEC-controller.md` §7 and the `research:uf2-flash` audit):
/// - An `rp2040`-family image would need picotool/elf2uf2's dummy zero-page
///   sector padding to be safe, because the RP2040 bootrom erases flash by
///   `block_no`, not by target address (unlike RP2350, whose bootrom this
///   converter's page-splitting logic assumes throughout); this toolkit only
///   ever targets the RP2350-based XIAO board, so there is no RP2040 flash
///   window/boot2 support to pad for in the first place.
/// - `rp2350-riscv` would need a RISC-V (`EM_RISCV`) ELF to convert, but
///   [`elf_to_uf2`] only accepts `EM_ARM` images (the firmware is always
///   built for `thumbv8m.main-none-eabihf`), so the value could never
///   actually be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Family {
    #[value(name = "absolute")]
    Absolute,
    #[value(name = "data")]
    Data,
    #[value(name = "rp2350-arm-s")]
    #[default]
    Rp2350ArmS,
    #[value(name = "rp2350-arm-ns")]
    Rp2350ArmNs,
}

impl Family {
    pub const fn id(self) -> u32 {
        match self {
            Family::Absolute => 0xe48b_ff57,
            Family::Data => 0xe48b_ff58,
            Family::Rp2350ArmS => 0xe48b_ff59,
            Family::Rp2350ArmNs => 0xe48b_ff5b,
        }
    }

    pub const fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0xe48b_ff57 => Family::Absolute,
            0xe48b_ff58 => Family::Data,
            0xe48b_ff59 => Family::Rp2350ArmS,
            0xe48b_ff5b => Family::Rp2350ArmNs,
            _ => return None,
        })
    }

    pub const fn name(self) -> &'static str {
        match self {
            Family::Absolute => "absolute",
            Family::Data => "data",
            Family::Rp2350ArmS => "rp2350-arm-s",
            Family::Rp2350ArmNs => "rp2350-arm-ns",
        }
    }
}

impl std::fmt::Display for Family {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Names an RP2350-family UF2 id this tool recognises but doesn't offer on
/// `--family` -- `rp2040` and `rp2350-riscv` (see [`Family`]'s doc comment
/// for why they're not CLI-selectable). Used only for diagnostic display
/// (`rcctl uf2-info`, via [`Uf2Block::family_kind`]/[`FamilyKind`]), so that
/// e.g. the RP2040-family output of the crates.io `elf2uf2-rs` 2.2.0 mistake
/// SPEC-controller.md §7 warns about is named, not just printed as a bare
/// hex id -- or, before this fix, folded into an undifferentiated
/// "<none/unknown>" alongside a UF2 with no family id at all.
pub const fn known_family_name(id: u32) -> Option<&'static str> {
    match id {
        0xe48b_ff56 => Some("rp2040"),
        0xe48b_ff5a => Some("rp2350-riscv"),
        _ => None,
    }
}

/// A block's family for display purposes ([`Uf2Summary::families`] /
/// [`Uf2Block::family_kind`]): distinguishes a CLI-selectable [`Family`],
/// an id [`known_family_name`] recognises but `--family` doesn't offer, an
/// id nobody here recognises at all, and a block with no family id present
/// -- so `uf2-info` on a foreign UF2 always shows the concrete id it found
/// instead of collapsing every unrecognized case into one bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FamilyKind {
    /// One of the four families this tool can produce via `--family`.
    Known(Family),
    /// `FLAG_FAMILY_ID_PRESENT` was set and the id is a recognised one this
    /// tool doesn't offer on `--family` (see [`known_family_name`]).
    Named(u32, &'static str),
    /// `FLAG_FAMILY_ID_PRESENT` was set but the id isn't recognised at all.
    Unrecognized(u32),
    /// `FLAG_FAMILY_ID_PRESENT` was not set on this block.
    Absent,
}

impl std::fmt::Display for FamilyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FamilyKind::Known(fam) => write!(f, "{fam} (0x{:08x})", fam.id()),
            FamilyKind::Named(id, name) => write!(f, "{name} (0x{id:08x}, not selectable via --family)"),
            FamilyKind::Unrecognized(id) => write!(f, "<unrecognized> (0x{id:08x})"),
            FamilyKind::Absent => write!(f, "<none: no family id present>"),
        }
    }
}

/// Options controlling [`elf_to_uf2`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Uf2Options {
    /// Family ID stamped into every block of the main transfer.
    pub family: Family,
    /// Prepend the picotool-style RP2350-E10 absolute block workaround.
    pub abs_block: bool,
}

/// One parsed or generated 512-byte UF2 block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Uf2Block {
    pub flags: u32,
    pub target_addr: u32,
    pub payload_size: u32,
    pub block_no: u32,
    pub num_blocks: u32,
    /// `file_size` field; doubles as the family ID when
    /// [`FLAG_FAMILY_ID_PRESENT`] is set (the only case we ever produce).
    pub file_size_or_family: u32,
    pub data: [u8; DATA_AREA_SIZE],
}

impl Uf2Block {
    fn data_page(
        target_addr: u32,
        block_no: u32,
        num_blocks: u32,
        family: Family,
        payload: &[u8; PAGE_SIZE],
    ) -> Self {
        let mut data = [0u8; DATA_AREA_SIZE];
        data[..PAGE_SIZE].copy_from_slice(payload);
        Self {
            flags: FLAG_FAMILY_ID_PRESENT,
            target_addr,
            payload_size: PAGE_SIZE as u32,
            block_no,
            num_blocks,
            file_size_or_family: family.id(),
            data,
        }
    }

    fn abs_block() -> Self {
        let mut data = [0u8; DATA_AREA_SIZE];
        data[..PAGE_SIZE].fill(ABS_BLOCK_FILL);
        data[PAGE_SIZE..PAGE_SIZE + 4].copy_from_slice(&UF2_EXTENSION_RP2_IGNORE_BLOCK.to_le_bytes());
        Self {
            flags: ABS_BLOCK_FLAGS,
            target_addr: ABS_BLOCK_TARGET,
            payload_size: PAGE_SIZE as u32,
            block_no: 0,
            num_blocks: ABS_BLOCK_NUM_BLOCKS,
            file_size_or_family: Family::Absolute.id(),
            data,
        }
    }

    /// Family ID this block carries, as a [`FamilyKind`] -- distinguishing a
    /// CLI-selectable [`Family`] from a recognised-but-not-offered id
    /// (`rp2040`/`rp2350-riscv`, via [`known_family_name`]), an id nobody
    /// here recognises, and no family id at all (a foreign UF2 read via
    /// [`parse_uf2`] might not set the flag; this tool always does). Used by
    /// `rcctl uf2-info`.
    pub fn family_kind(&self) -> FamilyKind {
        if self.flags & FLAG_FAMILY_ID_PRESENT == 0 {
            return FamilyKind::Absent;
        }
        let id = self.file_size_or_family;
        match Family::from_id(id) {
            Some(f) => FamilyKind::Known(f),
            None => match known_family_name(id) {
                Some(name) => FamilyKind::Named(id, name),
                None => FamilyKind::Unrecognized(id),
            },
        }
    }

    /// True if this is an RP2350-E10-style block the bootrom is told to
    /// ignore for transfer-completion purposes.
    pub fn is_rp2_ignore_block(&self) -> bool {
        self.flags & FLAG_EXTENSION_FLAGS_PRESENT != 0
            && self
                .data
                .get(PAGE_SIZE..PAGE_SIZE + 4)
                .map(|s| u32::from_le_bytes(s.try_into().unwrap()) == UF2_EXTENSION_RP2_IGNORE_BLOCK)
                .unwrap_or(false)
    }

    pub fn to_bytes(self) -> [u8; BLOCK_SIZE] {
        let mut b = [0u8; BLOCK_SIZE];
        b[0..4].copy_from_slice(&MAGIC_START0.to_le_bytes());
        b[4..8].copy_from_slice(&MAGIC_START1.to_le_bytes());
        b[8..12].copy_from_slice(&self.flags.to_le_bytes());
        b[12..16].copy_from_slice(&self.target_addr.to_le_bytes());
        b[16..20].copy_from_slice(&self.payload_size.to_le_bytes());
        b[20..24].copy_from_slice(&self.block_no.to_le_bytes());
        b[24..28].copy_from_slice(&self.num_blocks.to_le_bytes());
        b[28..32].copy_from_slice(&self.file_size_or_family.to_le_bytes());
        b[32..32 + DATA_AREA_SIZE].copy_from_slice(&self.data);
        b[BLOCK_SIZE - 4..BLOCK_SIZE].copy_from_slice(&MAGIC_END.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        if b.len() != BLOCK_SIZE {
            bail!("a UF2 block must be exactly {BLOCK_SIZE} bytes, got {}", b.len());
        }
        let u32_at = |off: usize| u32::from_le_bytes(b[off..off + 4].try_into().unwrap());
        if u32_at(0) != MAGIC_START0 || u32_at(4) != MAGIC_START1 {
            bail!("bad UF2 block: start magic mismatch");
        }
        if u32_at(BLOCK_SIZE - 4) != MAGIC_END {
            bail!("bad UF2 block: end magic mismatch");
        }
        let mut data = [0u8; DATA_AREA_SIZE];
        data.copy_from_slice(&b[32..32 + DATA_AREA_SIZE]);
        Ok(Self {
            flags: u32_at(8),
            target_addr: u32_at(12),
            payload_size: u32_at(16),
            block_no: u32_at(20),
            num_blocks: u32_at(24),
            file_size_or_family: u32_at(28),
            data,
        })
    }
}

/// Parses a `.uf2` file into its blocks (used by `rcctl uf2-info` and by
/// tests to round-trip [`elf_to_uf2`]'s output).
pub fn parse_uf2(bytes: &[u8]) -> Result<Vec<Uf2Block>> {
    if bytes.is_empty() {
        bail!("empty UF2 file");
    }
    if !bytes.len().is_multiple_of(BLOCK_SIZE) {
        bail!("UF2 file length {} is not a multiple of {BLOCK_SIZE}", bytes.len());
    }
    bytes.as_chunks::<BLOCK_SIZE>().0.iter().map(|b| Uf2Block::from_bytes(b)).collect()
}

/// One 256-byte flash page being assembled from possibly-several ELF
/// segment fragments.
#[derive(Clone)]
struct PageBuilder {
    data: [u8; PAGE_SIZE],
    touched: [bool; PAGE_SIZE],
}

impl Default for PageBuilder {
    fn default() -> Self {
        Self { data: [0u8; PAGE_SIZE], touched: [false; PAGE_SIZE] }
    }
}

/// Computes the `[off, off+n)` byte range for a `PT_LOAD` segment's file
/// contents. Does the addition in `u64` and checks it against `file_len`
/// before ever casting to `usize`, so a crafted `p_offset`/size pair can
/// only ever produce a clean error -- never a `usize` overflow panic,
/// regardless of the host pointer width.
fn segment_file_range(off: u64, n: u64, file_len: usize) -> Result<std::ops::Range<usize>> {
    let end = off
        .checked_add(n)
        .ok_or_else(|| anyhow!("segment file range overflow (p_offset=0x{off:x}, size={n})"))?;
    if end > file_len as u64 {
        bail!("segment file range 0x{off:x}..0x{end:x} is out of bounds of the ELF file (file is {file_len} bytes)");
    }
    Ok(off as usize..end as usize)
}

/// Converts a linked ELF32 image into a flashing UF2, per the rules in
/// `research:uf2-flash` (`Concrete recipe for the Rust host tool`) and
/// SPEC-controller.md §7.
///
/// Only little-endian ELF32 `EM_ARM` `ET_EXEC` images are accepted -- anything
/// else (an ELF64 or big-endian image, or a host binary such as an x86-64
/// executable) is rejected up front with a message naming the actual
/// class/endianness/machine, rather than being parsed as if it were Cortex-M
/// firmware and failing later with a misleading flash-window or linker-script
/// error.
///
/// Only `PT_LOAD` segments with `min(p_filesz, p_memsz) > 0` contribute
/// bytes, taken from the ELF file at `p_offset` and placed by their load
/// address (`p_paddr`), *not* their virtual address — so `.data`'s
/// initializer is captured at its flash LMA. Every contributing byte's LMA
/// must fall inside the RP2350 flash window; content whose LMA is in SRAM
/// (e.g. a link that never relocated `.data`'s physical address into flash)
/// is rejected, since this tool only ever produces flash images.
pub fn elf_to_uf2(elf_bytes: &[u8], opts: &Uf2Options) -> Result<Vec<u8>> {
    let file = ElfBytes::<AnyEndian>::minimal_parse(elf_bytes).context("parsing ELF header")?;

    if file.ehdr.class != Class::ELF32
        || file.ehdr.endianness != AnyEndian::Little
        || file.ehdr.e_machine != EM_ARM
        || file.ehdr.e_type != ET_EXEC
    {
        bail!(
            "not a linked Cortex-M ELF32 firmware image (got class={:?}, endianness={:?}, \
             e_machine={}, e_type={}); rcctl only converts little-endian ELF32 EM_ARM ET_EXEC \
             images built for the firmware target",
            file.ehdr.class,
            file.ehdr.endianness,
            file.ehdr.e_machine,
            file.ehdr.e_type,
        );
    }

    let segments = file
        .segments()
        .ok_or_else(|| anyhow!("ELF has no program header table (not a linked, loadable image)"))?;

    let mut pages: BTreeMap<u32, PageBuilder> = BTreeMap::new();
    let mut any_content = false;

    for ph in segments.iter() {
        if ph.p_type != PT_LOAD {
            continue;
        }
        let n = ph.p_filesz.min(ph.p_memsz);
        if n == 0 {
            continue;
        }
        any_content = true;

        let paddr = ph.p_paddr;
        let end = paddr
            .checked_add(n)
            .ok_or_else(|| anyhow!("PT_LOAD segment address overflow (p_paddr=0x{paddr:x}, size={n})"))?;
        if paddr < FLASH_START as u64 || end > FLASH_END as u64 {
            bail!(
                "PT_LOAD segment's load address (LMA) range 0x{paddr:08x}..0x{end:08x} falls outside \
                 the RP2350 flash window 0x{FLASH_START:08x}..0x{FLASH_END:08x}; SRAM-only content \
                 cannot be flashed (rcctl only produces flash images) -- if this is a RAM-resident \
                 segment, check that the linker script placed its LMA in flash"
            );
        }

        let range = segment_file_range(ph.p_offset, n, elf_bytes.len())?;
        let bytes = &elf_bytes[range];

        let base = paddr as u32;
        for (i, &byte) in bytes.iter().enumerate() {
            let addr = base + i as u32;
            let page_key = addr & !(PAGE_SIZE as u32 - 1);
            let page_off = (addr & (PAGE_SIZE as u32 - 1)) as usize;
            let page = pages.entry(page_key).or_default();
            if page.touched[page_off] {
                bail!("overlapping ELF segment content at flash address 0x{addr:08x}");
            }
            page.touched[page_off] = true;
            page.data[page_off] = byte;
        }
    }

    if !any_content {
        bail!("ELF has no PT_LOAD segments with content -- nothing to flash");
    }

    // Like picotool's `pages2uf2` (`research:uf2-flash` §3 / critic #7): the
    // abs block is a workaround layered on top of a *different* main family,
    // so it is never emitted when the main transfer's own family is already
    // `Absolute` -- on an A2 bootrom (which doesn't honour the IGNORE_BLOCK
    // extension tag) a same-family abs block and a same-sized main transfer
    // would collide, since the bootrom treats any repeat of a family/
    // num_blocks/block_no it already has as a duplicate of the first block
    // seen (see `Uf2Block::abs_block`'s `block_no: 0`).
    let emit_abs_block = opts.abs_block && opts.family != Family::Absolute;
    let num_blocks = pages.len() as u32;
    let extra_blocks = if emit_abs_block { 1 } else { 0 };
    let mut out = Vec::with_capacity(BLOCK_SIZE * (num_blocks as usize + extra_blocks));
    if emit_abs_block {
        out.extend_from_slice(&Uf2Block::abs_block().to_bytes());
    }
    for (i, (&addr, page)) in pages.iter().enumerate() {
        let block = Uf2Block::data_page(addr, i as u32, num_blocks, opts.family, &page.data);
        out.extend_from_slice(&block.to_bytes());
    }
    Ok(out)
}

/// Human-readable summary used by `rcctl uf2-info`.
pub struct Uf2Summary {
    pub total_blocks: usize,
    pub abs_blocks: usize,
    pub data_blocks: usize,
    pub addr_min: u32,
    pub addr_max_end: u64,
    /// `(family, count)` among the data blocks, in first-seen order.
    pub families: Vec<(FamilyKind, usize)>,
}

pub fn summarize(blocks: &[Uf2Block]) -> Uf2Summary {
    let mut data_min = u32::MAX;
    let mut data_max_end = 0u64;
    let mut data_blocks = 0usize;
    let mut abs_blocks = 0usize;
    let mut families: Vec<(FamilyKind, usize)> = Vec::new();

    for b in blocks {
        if b.is_rp2_ignore_block() {
            abs_blocks += 1;
            continue;
        }
        data_blocks += 1;
        data_min = data_min.min(b.target_addr);
        data_max_end = data_max_end.max(b.target_addr as u64 + b.payload_size as u64);
        let fam = b.family_kind();
        if let Some(entry) = families.iter_mut().find(|(f, _)| *f == fam) {
            entry.1 += 1;
        } else {
            families.push((fam, 1));
        }
    }

    Uf2Summary {
        total_blocks: blocks.len(),
        abs_blocks,
        data_blocks,
        addr_min: if data_blocks == 0 { 0 } else { data_min },
        addr_max_end: data_max_end,
        families,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-builds a minimal ELF32 LE image with the given `PT_LOAD`
    /// segments, `(p_paddr, p_vaddr, file_bytes, p_memsz)`.
    fn build_elf32(segments: &[(u32, u32, &[u8], u32)]) -> Vec<u8> {
        const EHSIZE: usize = 52;
        const PHENTSIZE: usize = 32;
        let phnum = segments.len();

        let mut offsets = Vec::new();
        let mut data_blob = Vec::new();
        for (_, _, data, _) in segments {
            offsets.push(EHSIZE + PHENTSIZE * phnum + data_blob.len());
            data_blob.extend_from_slice(data);
        }

        let mut ehdr = vec![0u8; EHSIZE];
        ehdr[0] = 0x7f;
        ehdr[1] = b'E';
        ehdr[2] = b'L';
        ehdr[3] = b'F';
        ehdr[4] = 1; // ELFCLASS32
        ehdr[5] = 1; // ELFDATA2LSB
        ehdr[6] = 1; // EV_CURRENT
        ehdr[16..18].copy_from_slice(&2u16.to_le_bytes()); // e_type = ET_EXEC
        ehdr[18..20].copy_from_slice(&40u16.to_le_bytes()); // e_machine = EM_ARM
        ehdr[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
        let entry = segments.first().map(|s| s.1).unwrap_or(0);
        ehdr[24..28].copy_from_slice(&entry.to_le_bytes()); // e_entry
        ehdr[28..32].copy_from_slice(&(EHSIZE as u32).to_le_bytes()); // e_phoff
        ehdr[40..42].copy_from_slice(&(EHSIZE as u16).to_le_bytes()); // e_ehsize
        ehdr[42..44].copy_from_slice(&(PHENTSIZE as u16).to_le_bytes()); // e_phentsize
        ehdr[44..46].copy_from_slice(&(phnum as u16).to_le_bytes()); // e_phnum

        let mut phdrs = Vec::new();
        for (i, (paddr, vaddr, data, memsz)) in segments.iter().enumerate() {
            let mut ph = vec![0u8; PHENTSIZE];
            ph[0..4].copy_from_slice(&1u32.to_le_bytes()); // p_type = PT_LOAD
            ph[4..8].copy_from_slice(&(offsets[i] as u32).to_le_bytes()); // p_offset
            ph[8..12].copy_from_slice(&vaddr.to_le_bytes()); // p_vaddr
            ph[12..16].copy_from_slice(&paddr.to_le_bytes()); // p_paddr
            ph[16..20].copy_from_slice(&(data.len() as u32).to_le_bytes()); // p_filesz
            ph[20..24].copy_from_slice(&memsz.to_le_bytes()); // p_memsz
            ph[24..28].copy_from_slice(&5u32.to_le_bytes()); // p_flags = R+X
            ph[28..32].copy_from_slice(&4u32.to_le_bytes()); // p_align
            phdrs.extend_from_slice(&ph);
        }

        let mut file = ehdr;
        file.extend_from_slice(&phdrs);
        file.extend_from_slice(&data_blob);
        file
    }

    #[test]
    fn converts_text_and_data_with_page_split_and_lma() {
        // .text-like segment: 300 bytes at flash 0x10000000 -> crosses a
        // 256-byte page boundary.
        let text = vec![0xAAu8; 300];
        // .data-like segment: VMA in RAM, LMA (p_paddr) in flash, with a
        // bss tail (memsz > filesz) that must NOT be emitted.
        let data_init = vec![0xBBu8; 64];

        let elf = build_elf32(&[
            (0x1000_0000, 0x1000_0000, &text, 300),
            (0x1000_1000, 0x2000_0000, &data_init, 128),
        ]);

        let uf2 = elf_to_uf2(&elf, &Uf2Options::default()).unwrap();
        let blocks = parse_uf2(&uf2).unwrap();

        assert_eq!(blocks.len(), 3, "300B text (2 pages) + 64B data (1 page)");
        assert_eq!(blocks[0].target_addr, 0x1000_0000);
        assert_eq!(blocks[1].target_addr, 0x1000_0100);
        assert_eq!(blocks[2].target_addr, 0x1000_1000);
        for (i, b) in blocks.iter().enumerate() {
            assert_eq!(b.block_no, i as u32);
            assert_eq!(b.num_blocks, 3);
            assert_eq!(b.payload_size, 256);
            assert_eq!(b.family_kind(), FamilyKind::Known(Family::Rp2350ArmS));
        }

        // Page split across the 256-byte boundary: block 1 holds the last
        // 44 bytes of `text`, zero-padded for the rest of the page.
        assert!(blocks[1].data[..44].iter().all(|&b| b == 0xAA));
        assert!(blocks[1].data[44..256].iter().all(|&b| b == 0));

        // `.data` is captured at its LMA (flash), not its VMA (RAM); the
        // bss tail (bytes 64..128) is simply absent (zero-filled), never an
        // error.
        assert!(blocks[2].data[..64].iter().all(|&b| b == 0xBB));
        assert!(blocks[2].data[64..256].iter().all(|&b| b == 0));
    }

    #[test]
    fn rejects_sram_lma_content() {
        let data = vec![0x11u8; 16];
        // p_paddr in SRAM: this would only be valid for a RAM-resident
        // image, which this tool refuses since it only produces flash UF2s.
        let elf = build_elf32(&[(0x2000_0000, 0x2000_0000, &data, 16)]);
        let err = elf_to_uf2(&elf, &Uf2Options::default()).unwrap_err();
        assert!(err.to_string().contains("flash window"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_overlapping_fragments() {
        let a = vec![0x01u8; 16];
        let b = vec![0x02u8; 16];
        // Two segments whose LMA ranges overlap.
        let elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &a, 16), (0x1000_0008, 0x1000_0008, &b, 16)]);
        let err = elf_to_uf2(&elf, &Uf2Options::default()).unwrap_err();
        assert!(err.to_string().contains("overlapping"), "unexpected error: {err}");
    }

    #[test]
    fn abs_block_layout() {
        let text = vec![0x42u8; 16];
        let elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &text, 16)]);
        let opts = Uf2Options { abs_block: true, ..Uf2Options::default() };
        let uf2 = elf_to_uf2(&elf, &opts).unwrap();
        let blocks = parse_uf2(&uf2).unwrap();

        assert_eq!(blocks.len(), 2, "abs block + 1 data block");
        let abs = &blocks[0];
        assert_eq!(abs.target_addr, ABS_BLOCK_TARGET);
        assert_eq!(abs.flags, FLAG_FAMILY_ID_PRESENT | FLAG_EXTENSION_FLAGS_PRESENT);
        assert_eq!(abs.payload_size, 256);
        assert_eq!(abs.block_no, 0);
        assert_eq!(abs.num_blocks, 2);
        assert_eq!(abs.family_kind(), FamilyKind::Known(Family::Absolute));
        assert!(abs.data[..256].iter().all(|&b| b == 0xEF));
        assert_eq!(
            u32::from_le_bytes(abs.data[256..260].try_into().unwrap()),
            UF2_EXTENSION_RP2_IGNORE_BLOCK
        );
        assert!(abs.is_rp2_ignore_block());

        // The data block's own num_blocks is unaffected by the abs block.
        let data_block = &blocks[1];
        assert_eq!(data_block.num_blocks, 1);
        assert_eq!(data_block.block_no, 0);
        assert!(!data_block.is_rp2_ignore_block());
    }

    /// Builds a minimal (no program/section headers) ELF header with the
    /// given class byte (`1`=ELF32, `2`=ELF64), data-encoding byte
    /// (`1`=little, `2`=big), `e_type` and `e_machine` -- enough for
    /// [`ElfBytes::minimal_parse`] to succeed so the class/endianness/machine
    /// guard in `elf_to_uf2` can be exercised without needing valid segments.
    fn build_minimal_elf_header(class: u8, data_enc: u8, e_type: u16, machine: u16) -> Vec<u8> {
        let be = data_enc == 2;
        let put16 = |buf: &mut [u8], off: usize, v: u16| {
            buf[off..off + 2].copy_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
        };
        let put32 = |buf: &mut [u8], off: usize, v: u32| {
            buf[off..off + 4].copy_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
        };
        let put64 = |buf: &mut [u8], off: usize, v: u64| {
            buf[off..off + 8].copy_from_slice(&if be { v.to_be_bytes() } else { v.to_le_bytes() });
        };

        let ehsize: usize = if class == 1 { 52 } else { 64 };
        let mut h = vec![0u8; ehsize];
        h[0] = 0x7f;
        h[1] = b'E';
        h[2] = b'L';
        h[3] = b'F';
        h[4] = class;
        h[5] = data_enc;
        h[6] = 1; // EV_CURRENT
        put16(&mut h, 16, e_type);
        put16(&mut h, 18, machine);
        put32(&mut h, 20, 1); // e_version

        if class == 1 {
            // e_entry/e_phoff/e_shoff already zero (no program/section headers).
            put16(&mut h, 40, ehsize as u16); // e_ehsize
            put16(&mut h, 42, 32); // e_phentsize
            put16(&mut h, 46, 40); // e_shentsize
        } else {
            let _ = put64; // silence unused-closure warning on the ELF32 branch
            put16(&mut h, 52, ehsize as u16); // e_ehsize
            put16(&mut h, 54, 56); // e_phentsize
            put16(&mut h, 58, 64); // e_shentsize
        }
        h
    }

    #[test]
    fn rejects_elf64_images() {
        // ELF64 EM_X86_64 ET_EXEC: silently converting this would produce a
        // misleading "flash window" error instead of naming the real problem.
        use elf::abi::EM_X86_64;
        let elf = build_minimal_elf_header(2, 1, ET_EXEC, EM_X86_64);
        let err = elf_to_uf2(&elf, &Uf2Options::default()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("ELF32"), "unexpected error: {msg}");
        assert!(msg.contains("ELF64"), "unexpected error: {msg}");
    }

    #[test]
    fn rejects_big_endian_images() {
        // Big-endian ELF32 EM_ARM: same class/machine as a real firmware
        // image, but the wrong endianness.
        let elf = build_minimal_elf_header(1, 2, ET_EXEC, EM_ARM);
        let err = elf_to_uf2(&elf, &Uf2Options::default()).unwrap_err();
        assert!(err.to_string().contains("Big"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_non_arm_host_binary() {
        // Feed rcctl's own test binary (an ELF64 EM_X86_64 host executable
        // on this platform) through the converter: it must be rejected with
        // a message naming the ELF as the problem, not the RP2350 flash
        // window / linker script.
        let path = std::env::current_exe().expect("current_exe");
        let bytes = std::fs::read(&path).expect("reading own test binary");
        let err = elf_to_uf2(&bytes, &Uf2Options::default()).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("flash window"), "should not blame the flash window: {msg}");
        assert!(!msg.contains("linker script"), "should not blame the linker script: {msg}");
    }

    #[test]
    fn segment_file_range_rejects_overflow_without_panicking() {
        // p_offset = u64::MAX-2, size 8: `off + n` overflows u64. Before the
        // checked add this was computed as a plain `usize` addition and
        // panicked with "attempt to add with overflow" in debug builds (and
        // silently wrapped in release). It must now be a clean error.
        let result = std::panic::catch_unwind(|| segment_file_range(u64::MAX - 2, 8, 1024));
        let err = result.expect("must not panic").expect_err("overflowing range must be rejected");
        assert!(err.to_string().contains("overflow"), "unexpected error: {err}");
    }

    #[test]
    fn segment_file_range_rejects_out_of_bounds() {
        assert!(segment_file_range(1000, 100, 1024).is_err());
        assert_eq!(segment_file_range(10, 20, 1024).unwrap(), 10..30);
    }

    #[test]
    fn crafted_p_offset_does_not_panic_end_to_end() {
        // A crafted (but in-range for ELF32's 32-bit p_offset field) offset
        // that lands past the end of the actual file bytes must be rejected
        // cleanly by `elf_to_uf2`, not panic.
        let mut elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &[0xAAu8; 8], 8)]);
        // Program header table starts right after the 52-byte ELF32 header;
        // p_offset is at offset 4 within a 32-byte entry.
        let ph_offset_field = 52 + 4;
        elf[ph_offset_field..ph_offset_field + 4].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            elf_to_uf2(&elf, &Uf2Options::default())
        }));
        let err = result.expect("must not panic").expect_err("crafted p_offset must be rejected");
        assert!(err.to_string().contains("out of bounds"), "unexpected error: {err}");
    }

    #[test]
    fn summarize_does_not_overflow_on_foreign_block() {
        // A block near the top of the 32-bit address space whose
        // target_addr + payload_size overflows u32 (foreign/corrupt UF2, not
        // one this tool produced -- `from_bytes` performs no range check).
        let mut data = [0u8; DATA_AREA_SIZE];
        data.fill(0);
        let block = Uf2Block {
            flags: FLAG_FAMILY_ID_PRESENT,
            target_addr: 0xFFFF_FF00,
            payload_size: 256,
            block_no: 0,
            num_blocks: 1,
            file_size_or_family: Family::Rp2350ArmS.id(),
            data,
        };
        let result = std::panic::catch_unwind(|| summarize(&[block]));
        let summary = result.expect("summarize must not panic on out-of-range fields");
        assert_eq!(summary.addr_max_end, 0xFFFF_FF00u64 + 256);
    }

    #[test]
    fn family_selection_round_trips() {
        let text = vec![0x07u8; 4];
        let elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &text, 4)]);
        for family in [Family::Absolute, Family::Data, Family::Rp2350ArmS, Family::Rp2350ArmNs] {
            let opts = Uf2Options { family, abs_block: false };
            let uf2 = elf_to_uf2(&elf, &opts).unwrap();
            let blocks = parse_uf2(&uf2).unwrap();
            assert_eq!(blocks[0].family_kind(), FamilyKind::Known(family));
        }
    }

    /// Before this fix, `--abs-block` prepended the E10 workaround block
    /// unconditionally, even when the main transfer's own family was already
    /// `Absolute` -- exactly the picotool-diverging case flagged in the audit
    /// (critic #7 / `research:uf2-flash` §3): `pages2uf2` only emits it when
    /// `family_id != ABSOLUTE_FAMILY_ID`.
    #[test]
    fn abs_block_suppressed_when_main_family_is_absolute() {
        let text = vec![0x42u8; 16];
        let elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &text, 16)]);
        let opts = Uf2Options { family: Family::Absolute, abs_block: true };
        let uf2 = elf_to_uf2(&elf, &opts).unwrap();
        let blocks = parse_uf2(&uf2).unwrap();

        assert_eq!(blocks.len(), 1, "no separate abs block when the main family is already Absolute");
        assert!(!blocks[0].is_rp2_ignore_block());
        assert_eq!(blocks[0].family_kind(), FamilyKind::Known(Family::Absolute));
    }

    /// For every other family, `--abs-block` still emits it (unchanged
    /// behaviour, `abs_block_layout` above covers the block's own shape).
    #[test]
    fn abs_block_still_emitted_for_non_absolute_families() {
        let text = vec![0x42u8; 16];
        let elf = build_elf32(&[(0x1000_0000, 0x1000_0000, &text, 16)]);
        for family in [Family::Data, Family::Rp2350ArmS, Family::Rp2350ArmNs] {
            let opts = Uf2Options { family, abs_block: true };
            let uf2 = elf_to_uf2(&elf, &opts).unwrap();
            let blocks = parse_uf2(&uf2).unwrap();
            assert_eq!(blocks.len(), 2, "family {family} should still get the abs block");
            assert!(blocks[0].is_rp2_ignore_block());
        }
    }

    #[test]
    fn empty_elf_is_an_error() {
        let elf = build_elf32(&[]);
        let err = elf_to_uf2(&elf, &Uf2Options::default()).unwrap_err();
        assert!(err.to_string().contains("nothing to flash"), "unexpected error: {err}");
    }

    /// Before this fix, `Uf2Block::family()`/`Family::from_id` had no way to
    /// name an RP2040-family block (0xe48bff56) -- exactly what the crates.io
    /// `elf2uf2-rs` 2.2.0 mistake SPEC-controller.md §7 warns about produces
    /// -- so `rcctl uf2-info` printed `family: <none/unknown>` with no id at
    /// all, indistinguishable from a block with no family id present.
    #[test]
    fn family_kind_names_rp2040_and_rp2350_riscv_without_widening_the_cli_enum() {
        assert_eq!(known_family_name(0xe48b_ff56), Some("rp2040"));
        assert_eq!(known_family_name(0xe48b_ff5a), Some("rp2350-riscv"));
        assert_eq!(
            known_family_name(0xe48b_ff59),
            None,
            "a CLI-selectable id is not `known_family_name`'s job"
        );

        let rp2040_block = Uf2Block {
            flags: FLAG_FAMILY_ID_PRESENT,
            target_addr: FLASH_START,
            payload_size: PAGE_SIZE as u32,
            block_no: 0,
            num_blocks: 1,
            file_size_or_family: 0xe48b_ff56,
            data: [0u8; DATA_AREA_SIZE],
        };
        assert_eq!(rp2040_block.family_kind(), FamilyKind::Named(0xe48b_ff56, "rp2040"));
    }

    #[test]
    fn family_kind_distinguishes_absent_from_unrecognized() {
        let no_family_flag = Uf2Block {
            flags: 0,
            target_addr: FLASH_START,
            payload_size: PAGE_SIZE as u32,
            block_no: 0,
            num_blocks: 1,
            file_size_or_family: 0,
            data: [0u8; DATA_AREA_SIZE],
        };
        assert_eq!(no_family_flag.family_kind(), FamilyKind::Absent);

        let truly_unknown = Uf2Block {
            flags: FLAG_FAMILY_ID_PRESENT,
            target_addr: FLASH_START,
            payload_size: PAGE_SIZE as u32,
            block_no: 0,
            num_blocks: 1,
            file_size_or_family: 0xdead_beef,
            data: [0u8; DATA_AREA_SIZE],
        };
        assert_eq!(truly_unknown.family_kind(), FamilyKind::Unrecognized(0xdead_beef));
    }

    #[test]
    fn family_kind_display_includes_the_raw_id() {
        assert_eq!(FamilyKind::Known(Family::Rp2350ArmS).to_string(), "rp2350-arm-s (0xe48bff59)");
        assert_eq!(
            FamilyKind::Named(0xe48b_ff56, "rp2040").to_string(),
            "rp2040 (0xe48bff56, not selectable via --family)"
        );
        assert_eq!(FamilyKind::Unrecognized(0xdead_beef).to_string(), "<unrecognized> (0xdeadbeef)");
        assert_eq!(FamilyKind::Absent.to_string(), "<none: no family id present>");
    }

    #[test]
    fn parse_uf2_rejects_bad_length() {
        let err = parse_uf2(&[0u8; 100]).unwrap_err();
        assert!(err.to_string().contains("multiple of"));
    }

    /// Exercises an unaligned segment start and two `PT_LOAD` segments
    /// sharing a single 256-byte page, mirroring the real embassy/rp235x
    /// firmware ELF's layout observed in `research:embassy-rp235x`
    /// (`.vector_table`+`.start_block` ending at the unaligned address
    /// `0x1000013c`, `.text` immediately following it, next section at
    /// `0x10004474`) as a checked-in fixture built with [`build_elf32`].
    ///
    /// This replaces a previous version of this test that read a real ELF
    /// from a session-specific scratchpad path and silently passed (asserted
    /// nothing) whenever that file was absent -- which was always true on
    /// any machine other than the one session that produced it, including
    /// CI. It was also the *only* test exercising a non-256-aligned segment
    /// start or a page shared between two segments;
    /// `converts_text_and_data_with_page_split_and_lma` only uses
    /// page-aligned segment starts.
    #[test]
    fn converts_unaligned_segment_start_with_shared_page_merge() {
        const SEG1_LEN: usize = 0x13c; // e.g. .vector_table + .start_block
        const SEG2_LEN: usize = 0x4338; // e.g. .text, starts mid-page

        let seg1 = vec![0xAAu8; SEG1_LEN];
        let seg2 = vec![0xBBu8; SEG2_LEN];

        let elf = build_elf32(&[
            (0x1000_0000, 0x1000_0000, &seg1, SEG1_LEN as u32),
            (0x1000_013c, 0x1000_013c, &seg2, SEG2_LEN as u32),
        ]);

        let uf2 = elf_to_uf2(&elf, &Uf2Options::default()).expect("fixture ELF should convert");
        let blocks = parse_uf2(&uf2).unwrap();

        let total_len = SEG1_LEN + SEG2_LEN;
        let expected_pages = total_len.div_ceil(PAGE_SIZE);
        assert_eq!(blocks.len(), expected_pages);
        assert_eq!(blocks[0].target_addr, FLASH_START);

        // Page 0 (0x10000000..0x10000100): entirely segment 1.
        assert!(blocks[0].data[..PAGE_SIZE].iter().all(|&b| b == 0xAA));

        // Page 1 (0x10000100..0x10000200): the merge point -- segment 1's
        // tail and segment 2's head land in the *same* page.
        let merge_at = SEG1_LEN % PAGE_SIZE;
        assert_ne!(merge_at, 0, "fixture must actually straddle a page boundary");
        assert_eq!(blocks[1].target_addr, FLASH_START + PAGE_SIZE as u32);
        assert!(blocks[1].data[..merge_at].iter().all(|&b| b == 0xAA), "segment 1's tail");
        assert!(blocks[1].data[merge_at..PAGE_SIZE].iter().all(|&b| b == 0xBB), "segment 2's head");

        // Last page: segment 2's tail, zero-padded for the rest of the page
        // (both within the 256-byte payload and the rest of the 476-byte
        // `data` area beyond it).
        let last = blocks.last().unwrap();
        let end_off = total_len % PAGE_SIZE;
        assert_ne!(end_off, 0, "fixture must end mid-page to exercise zero padding");
        assert!(last.data[..end_off].iter().all(|&b| b == 0xBB));
        assert!(last.data[end_off..].iter().all(|&b| b == 0));
    }
}
