use crate::arch::aarch64::encode::{EncodedImage, EncodedSection};
use crate::os::note::{mfb_note_descriptor, MFB_NOTE_DESCRIPTOR_SIZE, MFB_NOTE_OWNER};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const VM_BASE: u64 = 0x1_0000_0000;
/// Byte size of one imported-symbol stub (adrp + ldr + br).
const IMPORT_STUB_SIZE: usize = 12;
const PAGE_SIZE: usize = 0x4000;
/// `sizeof(struct note_command)`: cmd + cmdsize + `data_owner[16]` + offset +
/// size. Every image carries exactly one (plan-43), so this is unconditional in
/// `load_commands_size`/`load_command_count`.
const NOTE_COMMAND_SIZE: usize = 40;

mod commands;
mod macho;
#[cfg(test)]
mod tests;

use macho::*;

pub(crate) fn write_executable(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
) -> Result<PathBuf, String> {
    let bytes = encode_executable_bytes(project_name, image)?;
    let path = project_dir.join(format!("{project_name}.out"));
    write_executable_file(&path, &bytes)?;
    Ok(path)
}

/// Write an app-mode `.app` bundle (plan-04-macos-app.md §5.2):
///
/// ```text
/// <name>.app/
///   Contents/
///     Info.plist
///     MacOS/
///       <name>
/// ```
///
/// The inner Mach-O is byte-identical to the `<name>.out` the console path
/// produces from the same image; only the on-disk layout and the accompanying
/// `Info.plist` differ. Returns the path to the `<name>.app` bundle directory.
pub(crate) fn write_app_bundle(
    project_dir: &Path,
    project_name: &str,
    image: &EncodedImage,
    app_icon: Option<&Path>,
    app_version: &str,
) -> Result<PathBuf, String> {
    let bytes = encode_executable_bytes(project_name, image)?;
    let bundle_path = project_dir.join(format!("{project_name}.app"));
    let contents_dir = bundle_path.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    fs::create_dir_all(&macos_dir)
        .map_err(|err| format!("failed to create '{}': {err}", macos_dir.display()))?;

    let executable_path = macos_dir.join(project_name);
    write_executable_file(&executable_path, &bytes)?;

    // Render the app icon (plan-22-B §4.4): the resolved project `icon` source or
    // the compiler's embedded default, packaged as a multi-resolution `.icns`.
    let resources_dir = contents_dir.join("Resources");
    fs::create_dir_all(&resources_dir)
        .map_err(|err| format!("failed to create '{}': {err}", resources_dir.display()))?;
    let icns = crate::os::macos::icon::build_icns(app_icon)?;
    let icns_path = resources_dir.join("AppIcon.icns");
    fs::write(&icns_path, icns)
        .map_err(|err| format!("failed to write '{}': {err}", icns_path.display()))?;

    let plist_path = contents_dir.join("Info.plist");
    fs::write(&plist_path, app_info_plist(project_name, app_version))
        .map_err(|err| format!("failed to write '{}': {err}", plist_path.display()))?;

    Ok(bundle_path)
}

/// Encode the final Mach-O executable image to bytes, shared by the console
/// `<name>.out` and app-mode bundle writers so both emit identical binaries.
fn encode_executable_bytes(project_name: &str, image: &EncodedImage) -> Result<Vec<u8>, String> {
    // Load-time initializers (plan-linker.md §5.3/§7.5) lower to a
    // `__mod_init_func` section dyld runs after binding and before `LC_MAIN`.
    // Each must name an internal text symbol; refuse rather than silently drop a
    // dangling entry, mirroring the Linux backend's DT_INIT_ARRAY handling.
    for name in &image.initializers {
        if !is_text_symbol(image, name) {
            return Err(format!(
                "initializer '{name}' does not resolve to a text symbol"
            ));
        }
    }
    let libraries = import_libraries(image)?;
    let has_imports = !libraries.is_empty();
    let has_signing_metadata = image.signing_metadata.is_some();
    let rodata_size = rodata_len(image);
    // `__DATA` (writable) exists only for the data past the read-only constant
    // prefix — the arena global and other runtime globals (bug-187).
    let has_writable = image.data.len() > rodata_size;
    let has_rodata = rodata_size > 0;
    let code_offset = code_offset(
        &libraries,
        has_signing_metadata,
        !image.initializers.is_empty(),
        has_writable,
        has_rodata,
    );
    let mut text = image.text.clone();
    let import_locations = if has_imports {
        append_import_stubs(
            &mut text,
            image,
            VM_BASE + code_offset as u64,
            code_offset,
            image.data.len(),
        )?
    } else {
        ImportLocations::default()
    };
    let layout = macho_layout(
        code_offset,
        text.len(),
        image.data.len(),
        rodata_size,
        data_const_size(image),
        0,
    );
    // Read-only constants sit in `__DATA_CONST,__const` (past the GOT/init
    // pointers); the writable arena global and runtime globals stay in `__DATA`
    // (bug-187). Data-symbol addresses resolve into one region or the other by
    // offset.
    let rodata_vmaddr =
        VM_BASE + layout.data_const_file_offset as u64 + rodata_offset_in_data_const(image) as u64;
    let data_vmaddr = VM_BASE + layout.data_seg_file_offset as u64;
    patch_relocations(
        &mut text,
        image,
        VM_BASE + code_offset as u64,
        rodata_vmaddr,
        data_vmaddr,
        rodata_size,
        &import_locations,
    )?;
    let entry_offset = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == image.entry)
        .filter(|symbol| symbol.section == EncodedSection::Text)
        .map(|symbol| symbol.offset)
        .ok_or_else(|| format!("entry symbol '{}' does not resolve to text", image.entry))?;
    Ok(encode_mach_o(
        project_name,
        code_offset,
        entry_offset,
        &text,
        &image.data,
        &libraries,
        image,
    ))
}

/// Write executable bytes to `path` and mark the file executable (0o755).
fn write_executable_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|err| format!("failed to mark '{}' executable: {err}", path.display()))?;
    Ok(())
}

/// Minimal `Info.plist` for an app-mode bundle (plan-04-macos-app.md §6.8).
/// `CFBundleExecutable`/`CFBundleName` use the project name; the bundle id is
/// namespaced under `dev.mfbasic.<name>`. The principal class is `NSApplication`
/// so Launch Services treats the bundle as a regular AppKit application.
///
/// `app_version` is the manifest `version` and publishes as both
/// `CFBundleShortVersionString` (marketing version) and `CFBundleVersion` (build
/// version). Both keys are mandatory for App Store submission: `altool` rejects a
/// bundle missing either one (bug-248).
fn app_info_plist(project_name: &str, app_version: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\"\n",
            " \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
            "<plist version=\"1.0\">\n",
            "<dict>\n",
            "  <key>CFBundleName</key>\n",
            "  <string>{name}</string>\n",
            "  <key>CFBundleExecutable</key>\n",
            "  <string>{name}</string>\n",
            "  <key>CFBundleIdentifier</key>\n",
            "  <string>dev.mfbasic.{name}</string>\n",
            "  <key>CFBundlePackageType</key>\n",
            "  <string>APPL</string>\n",
            "  <key>CFBundleShortVersionString</key>\n",
            "  <string>{version}</string>\n",
            "  <key>CFBundleVersion</key>\n",
            "  <string>{version}</string>\n",
            "  <key>CFBundleIconFile</key>\n",
            "  <string>AppIcon</string>\n",
            "  <key>NSPrincipalClass</key>\n",
            "  <string>NSApplication</string>\n",
            "</dict>\n",
            "</plist>\n"
        ),
        name = plist_escape(project_name),
        version = plist_escape(app_version)
    )
}

/// Escape the five XML predefined entities for safe inclusion in a plist string.
fn plist_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn patch_relocations(
    text: &mut [u8],
    image: &EncodedImage,
    text_vmaddr: u64,
    rodata_vmaddr: u64,
    data_vmaddr: u64,
    rodata_size: usize,
    import_locations: &ImportLocations,
) -> Result<(), String> {
    for relocation in &image.relocations {
        match relocation.binding.as_str() {
            "internal" if relocation.kind == "branch26" => {
                let target = symbol_vmaddr(
                    image,
                    &relocation.target,
                    text_vmaddr,
                    rodata_vmaddr,
                    data_vmaddr,
                    rodata_size,
                )?;
                let word = 0x9400_0000
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize)?;
                write_u32(text, relocation.offset, word);
            }
            "data" if relocation.kind == "page21" => {
                let target = symbol_vmaddr(
                    image,
                    &relocation.target,
                    text_vmaddr,
                    rodata_vmaddr,
                    data_vmaddr,
                    rodata_size,
                )?;
                let pc = text_vmaddr + relocation.offset as u64;
                let (immlo, immhi) = adrp_page21(pc, target)?;
                let rd = read_u32(text, relocation.offset) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                );
            }
            "data" if relocation.kind == "pageoff12" => {
                let target = symbol_vmaddr(
                    image,
                    &relocation.target,
                    text_vmaddr,
                    rodata_vmaddr,
                    data_vmaddr,
                    rodata_size,
                )?;
                let imm12 = (target & 0xfff) as u32;
                let word = read_u32(text, relocation.offset);
                let rd = word & 0x1f;
                let rn = (word >> 5) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
                );
            }
            "external" if relocation.kind == "branch26" => {
                let Some(&target) = import_locations.stubs.get(&relocation.target) else {
                    return Err(format!(
                        "macOS linker cannot bind external symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let word = 0x9400_0000
                    | branch_imm26(text_vmaddr as usize + relocation.offset, target as usize)?;
                write_u32(text, relocation.offset, word);
            }
            "external" if relocation.kind == "page21" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "macOS linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let pc = text_vmaddr + relocation.offset as u64;
                let (immlo, immhi) = adrp_page21(pc, target)?;
                let rd = read_u32(text, relocation.offset) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9000_0000 | (immlo << 29) | (immhi << 5) | rd,
                );
            }
            "external" if relocation.kind == "pageoff12" => {
                let Some(&target) = import_locations.got_entries.get(&relocation.target) else {
                    return Err(format!(
                        "macOS linker cannot bind external data symbol '{}' from {}",
                        relocation.target,
                        relocation.library.as_deref().unwrap_or("<unknown library>")
                    ));
                };
                let imm12 = (target & 0xfff) as u32;
                let word = read_u32(text, relocation.offset);
                let rd = word & 0x1f;
                let rn = (word >> 5) & 0x1f;
                write_u32(
                    text,
                    relocation.offset,
                    0x9100_0000 | (imm12 << 10) | (rn << 5) | rd,
                );
            }
            _ => {
                return Err(format!(
                    "macOS linker does not support relocation {} {}",
                    relocation.binding, relocation.kind
                ));
            }
        }
    }
    Ok(())
}

/// Map a logical library name to its Mach-O dylib install path (plan-linker.md
/// §7.3). Frameworks resolve to their framework binary, plain dylibs to
/// `/usr/lib`. The `tls` driver's concrete entry is `Network.framework`.
fn dylib_path(library: &str) -> Result<String, String> {
    Ok(match library {
        "libSystem" => "/usr/lib/libSystem.B.dylib".to_string(),
        "Network" => "/System/Library/Frameworks/Network.framework/Network".to_string(),
        // Audio (plan-33-B §5): AudioQueue backend static framework imports.
        "AudioToolbox" => {
            "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox".to_string()
        }
        "CoreAudio" => "/System/Library/Frameworks/CoreAudio.framework/CoreAudio".to_string(),
        "CoreFoundation" => {
            "/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation".to_string()
        }
        "AppKit" => "/System/Library/Frameworks/AppKit.framework/AppKit".to_string(),
        "Foundation" => "/System/Library/Frameworks/Foundation.framework/Foundation".to_string(),
        "libobjc" => "/usr/lib/libobjc.A.dylib".to_string(),
        "libz" => "/usr/lib/libz.1.dylib".to_string(),
        other => {
            return Err(format!(
                "macOS linker has no dylib path for library '{other}'"
            ));
        }
    })
}

/// The distinct dynamic libraries the image imports from, in first-seen order,
/// each paired with its install path and an implicit 1-based dylib ordinal
/// (its position + 1). Empty when the image imports nothing (plan-linker.md §7.1).
fn import_libraries(image: &EncodedImage) -> Result<Vec<(String, String)>, String> {
    let mut libraries: Vec<(String, String)> = Vec::new();
    for import in &image.imports {
        if !libraries.iter().any(|(name, _)| name == &import.library) {
            libraries.push((import.library.clone(), dylib_path(&import.library)?));
        }
    }
    Ok(libraries)
}

/// The 1-based dylib ordinal for a symbol's library within `libraries`.
fn library_ordinal(libraries: &[(String, String)], library: &str) -> Result<u64, String> {
    libraries
        .iter()
        .position(|(name, _)| name == library)
        .map(|index| index as u64 + 1)
        .ok_or_else(|| format!("macOS linker has no dylib ordinal for library '{library}'"))
}

/// Whether `name` resolves to a defined text symbol in the image.
fn is_text_symbol(image: &EncodedImage, name: &str) -> bool {
    image
        .symbols
        .iter()
        .any(|symbol| symbol.name == name && symbol.section == EncodedSection::Text)
}

/// Runtime VM address of an initializer's text symbol (plan-linker.md §7.5). The
/// pointer is stored unslid; dyld rebases it by the load slide (see `rebase_info`).
/// Callers validate the symbol exists via `is_text_symbol` first.
fn initializer_addr(image: &EncodedImage, name: &str, code_offset: usize) -> u64 {
    let offset = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == name && symbol.section == EncodedSection::Text)
        .map(|symbol| symbol.offset)
        .expect("initializer text symbol validated before encoding");
    VM_BASE + code_offset as u64 + offset as u64
}

/// Bytes of `image.data` that are read-only constants — the string literals and
/// error messages the shared codegen layout places first (bug-187). Clamped to
/// the data length so a stale `rodata_size` can never over-read.
fn rodata_len(image: &EncodedImage) -> usize {
    image.rodata_size.min(image.data.len())
}

/// Byte offset of the read-only constant block *within* `__DATA_CONST`: past the
/// GOT (one slot per import) and the `__mod_init_func` pointer array (one slot per
/// initializer), aligned to the 16-byte maximum data-object alignment so every
/// constant keeps its alignment. Placing the constants after the GOT/init keeps
/// `rebase_info`'s `__mod_init_func` offset (`imports * 8`) unchanged (bug-187).
fn rodata_offset_in_data_const(image: &EncodedImage) -> usize {
    align((image.imports.len() + image.initializers.len()) * 8, 16)
}

/// File/VM size of the `__DATA_CONST` segment: the GOT (one slot per import), the
/// `__mod_init_func` pointer array (one slot per initializer), and the read-only
/// constant block (`__const`, bug-187), rounded to a page. Zero when the image has
/// no GOT, no initializers and no constants.
fn data_const_size(image: &EncodedImage) -> usize {
    let slots = image.imports.len() + image.initializers.len();
    let rodata = rodata_len(image);
    let total = if rodata > 0 {
        rodata_offset_in_data_const(image) + rodata
    } else {
        slots * 8
    };
    if total == 0 {
        0
    } else {
        align(total, PAGE_SIZE)
    }
}

/// Number of sections in `__DATA_CONST`: `__got` when there are imports,
/// `__mod_init_func` when there are initializers, `__const` when there are
/// read-only constants.
fn data_const_section_count(import_count: usize, init_count: usize, has_rodata: bool) -> u32 {
    (import_count > 0) as u32 + (init_count > 0) as u32 + has_rodata as u32
}

/// Rebase opcode stream for `LC_DYLD_INFO_ONLY`. The `__mod_init_func` pointers
/// hold absolute (unslid) text addresses, so each needs a `REBASE_TYPE_POINTER`
/// rebase against `__DATA_CONST` (segment index 2) so dyld adds the load slide
/// before running them. The GOT is bound, not rebased, so it contributes nothing.
fn rebase_info(image: &EncodedImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    if !image.initializers.is_empty() {
        // REBASE_OPCODE_SET_TYPE_IMM (0x10) | REBASE_TYPE_POINTER (1).
        bytes.push(0x11);
        // REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB (0x20) | __DATA_CONST (seg 2),
        // offset = past the GOT slots.
        bytes.push(0x22);
        put_uleb128(&mut bytes, (image.imports.len() * 8) as u64);
        // REBASE_OPCODE_DO_REBASE_ULEB_TIMES (0x60) for each initializer pointer.
        bytes.push(0x60);
        put_uleb128(&mut bytes, image.initializers.len() as u64);
        // REBASE_OPCODE_DONE.
        bytes.push(0x00);
    }
    bytes
}

#[derive(Default)]
struct ImportLocations {
    stubs: HashMap<String, u64>,
    got_entries: HashMap<String, u64>,
}

fn append_import_stubs(
    text: &mut Vec<u8>,
    image: &EncodedImage,
    text_vmaddr: u64,
    code_offset: usize,
    data_len: usize,
) -> Result<ImportLocations, String> {
    let mut locations = ImportLocations::default();
    // Each import appends a 3-instruction (12-byte) stub to the text section.
    // The GOT lives at `data_const_file_offset`, which is the page-aligned end of
    // the final code (stubs included) plus the constant data. Compute the layout
    // from that final code length, not the pre-stub length, so the GOT address
    // baked into every stub matches where the GOT is actually placed. Using the
    // pre-stub length makes the two diverge by a page whenever the stub bytes push
    // the total across a `PAGE_SIZE` boundary, which sends each stub's `br` to a
    // garbage address.
    let final_code_len = text.len() + image.imports.len() * IMPORT_STUB_SIZE;
    let layout = macho_layout(
        code_offset,
        final_code_len,
        data_len,
        rodata_len(image),
        data_const_size(image),
        0,
    );
    for (index, import) in image.imports.iter().enumerate() {
        let stub_offset = text.len();
        let stub_vmaddr = text_vmaddr + stub_offset as u64;
        let got_vmaddr = VM_BASE + layout.data_const_file_offset as u64 + (index * 8) as u64;
        emit_import_stub(text, stub_vmaddr, got_vmaddr)?;
        locations.stubs.insert(import.symbol.clone(), stub_vmaddr);
        locations
            .got_entries
            .insert(import.symbol.clone(), got_vmaddr);
    }
    Ok(locations)
}

fn emit_import_stub(text: &mut Vec<u8>, stub_vmaddr: u64, got_vmaddr: u64) -> Result<(), String> {
    let (immlo, immhi) = adrp_page21(stub_vmaddr, got_vmaddr)?;
    put_u32(text, 0x9000_0010 | (immlo << 29) | (immhi << 5));
    put_u32(
        text,
        0xf940_0210 | ((((got_vmaddr & 0xfff) / 8) as u32) << 10),
    );
    put_u32(text, 0xd61f_0200);
    Ok(())
}

/// Runtime VM address of a symbol. Text symbols sit in `__TEXT`. Data symbols are
/// split (bug-187): a constant (`offset < rodata_size`) lives in the read-only
/// `__DATA_CONST,__const` block at `rodata_vmaddr`; a writable datum (the arena
/// global and other runtime globals) lives in `__DATA` at `data_vmaddr`, indexed
/// past the read-only prefix.
fn symbol_vmaddr(
    image: &EncodedImage,
    symbol_name: &str,
    text_vmaddr: u64,
    rodata_vmaddr: u64,
    data_vmaddr: u64,
    rodata_size: usize,
) -> Result<u64, String> {
    let symbol = image
        .symbols
        .iter()
        .find(|symbol| symbol.name == symbol_name)
        .ok_or_else(|| format!("symbol '{symbol_name}' does not resolve"))?;
    Ok(match symbol.section {
        EncodedSection::Text => text_vmaddr + symbol.offset as u64,
        EncodedSection::Data if symbol.offset < rodata_size => rodata_vmaddr + symbol.offset as u64,
        EncodedSection::Data => data_vmaddr + (symbol.offset - rodata_size) as u64,
    })
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

fn branch_imm26(source: usize, target: usize) -> Result<u32, String> {
    let delta = target as isize - source as isize;
    // A `BL`/`B` imm26 is a signed 26-bit word offset: ±2^25 words = ±128 MiB.
    // Masking without a reach check silently wraps an over-range branch into a
    // wrong instruction (bug-168); error instead, matching the riscv path.
    if delta % 4 != 0 || !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(format!(
            "macOS linker: branch displacement {delta} exceeds the ±128 MiB reach of BL/B"
        ));
    }
    Ok(((delta / 4) as i32 as u32) & 0x03ff_ffff)
}

/// Encode an `ADRP` page displacement, reach-checked (bug-168). The immediate is
/// a signed 21-bit count of 4 KiB pages (±2^20 pages = ±4 GiB); an over-range
/// delta must error rather than truncate to a wrong page. Returns `(immlo,
/// immhi)`.
fn adrp_page21(pc: u64, target: u64) -> Result<(u32, u32), String> {
    let page_delta = ((target & !0xfff) as i64 - (pc & !0xfff) as i64) >> 12;
    if !(-(1 << 20)..(1 << 20)).contains(&page_delta) {
        return Err(format!(
            "macOS linker: ADRP page displacement {page_delta} exceeds the ±4 GiB reach of ADRP"
        ));
    }
    let encoded = page_delta as u32;
    Ok((encoded & 0b11, (encoded >> 2) & 0x7ffff))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice length"))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_uleb128(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn put_be_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn put_be_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
