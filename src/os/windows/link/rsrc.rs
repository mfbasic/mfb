//! plan-66-K: the PE `.rsrc` resource section.
//!
//! An app-mode `.exe` carries three resource kinds, packaged into the standard
//! three-level PE resource directory tree (Type → Id → Language → data):
//!
//! - **RT_GROUP_ICON (14) + RT_ICON (3)** — the taskbar/Explorer icon, rendered
//!   from the project `icon` at several sizes (shared `os::icon::render_png`, PNG
//!   images, which Windows Vista+ accepts directly inside an icon).
//! - **RT_MANIFEST (24)** — a fusion manifest declaring per-monitor DPI awareness
//!   and Common Controls v6, so the window is crisp on HiDPI displays.
//! - **RT_VERSION (16)** — a `VS_VERSIONINFO` built from the manifest `version`,
//!   surfaced on Explorer's Details tab.
//!
//! The whole section is emitted only for GUI (app-mode) builds, so console `.exe`
//! images are byte-identical to the pre-K writer.

use std::path::Path;

// Resource type ids (Win32 `RT_*`).
const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const RT_MANIFEST: u16 = 24;
/// Single language used for every resource: `MAKELANGID(LANG_ENGLISH, SUBLANG_US)`.
const LANG_EN_US: u16 = 0x0409;
/// The manifest id Windows looks up for an application's fusion manifest.
const CREATEPROCESS_MANIFEST_RESOURCE_ID: u16 = 1;
/// Icon sizes rendered into the group (px). 256 rides as a PNG; the rest too.
const ICON_SIZES: [u32; 4] = [16, 32, 48, 256];

/// A single leaf resource: its type, numeric id, and raw bytes (already in the
/// on-disk resource format).
struct Resource {
    type_id: u16,
    res_id: u16,
    data: Vec<u8>,
}

/// Build the `.rsrc` section bytes for a GUI build. `rsrc_rva` is the section's
/// virtual address (needed because `IMAGE_RESOURCE_DATA_ENTRY.OffsetToData` is an
/// image RVA, not a section-relative offset). Returns the section bytes; the
/// caller sets data directory `[2]` to `(rsrc_rva, bytes.len())`.
pub(super) fn build_rsrc(
    rsrc_rva: u32,
    app_icon: Option<&Path>,
    app_version: Option<&str>,
) -> Result<Vec<u8>, String> {
    let mut resources: Vec<Resource> = Vec::new();

    // RT_MANIFEST (id 1): always present in app mode → DPI-aware + Common Controls.
    resources.push(Resource {
        type_id: RT_MANIFEST,
        res_id: CREATEPROCESS_MANIFEST_RESOURCE_ID,
        data: MANIFEST_XML.as_bytes().to_vec(),
    });

    // RT_ICON (ids 1..=n) + RT_GROUP_ICON (id 1): the app icon, when the project
    // supplies one (or a default is rendered from `None`).
    if app_icon.is_some() {
        let mut dir_entries: Vec<u8> = Vec::new();
        for (i, &size) in ICON_SIZES.iter().enumerate() {
            let png = crate::os::icon::render_png(app_icon, size)?;
            let icon_id = (i + 1) as u16;
            // GRPICONDIRENTRY (14 bytes): width, height (0 == 256), colorCount,
            // reserved, planes(1), bitCount(32), bytesInRes, id.
            let dim = if size >= 256 { 0u8 } else { size as u8 };
            dir_entries.push(dim); // bWidth
            dir_entries.push(dim); // bHeight
            dir_entries.push(0); // bColorCount (0 for >=256 colors)
            dir_entries.push(0); // bReserved
            dir_entries.extend_from_slice(&1u16.to_le_bytes()); // wPlanes
            dir_entries.extend_from_slice(&32u16.to_le_bytes()); // wBitCount
            dir_entries.extend_from_slice(&(png.len() as u32).to_le_bytes()); // dwBytesInRes
            dir_entries.extend_from_slice(&icon_id.to_le_bytes()); // nID (RT_ICON id)
            resources.push(Resource {
                type_id: RT_ICON,
                res_id: icon_id,
                data: png,
            });
        }
        // GRPICONDIR header: reserved(0), type(1 = icon), count.
        let mut group = Vec::new();
        group.extend_from_slice(&0u16.to_le_bytes());
        group.extend_from_slice(&1u16.to_le_bytes());
        group.extend_from_slice(&(ICON_SIZES.len() as u16).to_le_bytes());
        group.extend_from_slice(&dir_entries);
        resources.push(Resource {
            type_id: RT_GROUP_ICON,
            res_id: 1,
            data: group,
        });
    }

    // RT_VERSION (id 1): VS_VERSIONINFO from the manifest version.
    if let Some(version) = app_version {
        resources.push(Resource {
            type_id: RT_VERSION,
            res_id: 1,
            data: build_version_info(version),
        });
    }

    Ok(serialize_tree(rsrc_rva, &resources))
}

/// Serialize the three-level resource directory tree. Every leaf uses a single
/// language (`LANG_EN_US`). Directory-entry offsets are section-relative with the
/// high bit set for subdirectories; the data entry's `OffsetToData` is an image
/// RVA (`rsrc_rva + section offset`).
fn serialize_tree(rsrc_rva: u32, resources: &[Resource]) -> Vec<u8> {
    // Group by type id (ascending), then by resource id (ascending) within a type.
    let mut types: Vec<u16> = resources.iter().map(|r| r.type_id).collect();
    types.sort_unstable();
    types.dedup();

    let dir_hdr = 16usize; // IMAGE_RESOURCE_DIRECTORY
    let dir_ent = 8usize; // IMAGE_RESOURCE_DIRECTORY_ENTRY
    let data_ent = 16usize; // IMAGE_RESOURCE_DATA_ENTRY

    // Pass 1 — lay out every structure's section offset.
    // Level 1: one type directory.
    let l1_off = 0usize;
    let mut off = l1_off + dir_hdr + types.len() * dir_ent;
    // Level 2: one id directory per type.
    let mut l2_off = Vec::new();
    for &t in &types {
        let ids = ids_of_type(resources, t);
        l2_off.push(off);
        off += dir_hdr + ids.len() * dir_ent;
    }
    // Level 3: one language directory per (type, id).
    let mut l3_off = Vec::new(); // flat, in (type, id) order
    for &t in &types {
        for _id in ids_of_type(resources, t) {
            l3_off.push(off);
            off += dir_hdr + dir_ent; // one language entry
        }
    }
    // Data entries (one per leaf), then the blobs (4-aligned).
    let mut de_off = Vec::new();
    for _ in 0..leaf_count(resources) {
        de_off.push(off);
        off += data_ent;
    }
    let mut blob_off = Vec::new();
    for leaf in leaves(resources) {
        off = align4(off);
        blob_off.push(off);
        off += leaf.data.len();
    }
    let total = align4(off);

    // Pass 2 — emit.
    let mut buf = vec![0u8; total];
    // Level 1 directory.
    write_dir_header(&mut buf, l1_off, types.len());
    let mut e = l1_off + dir_hdr;
    for (ti, &t) in types.iter().enumerate() {
        write_dir_entry(&mut buf, e, t as u32, l2_off[ti] as u32, true);
        e += dir_ent;
    }
    // Level 2 directories (per type).
    let mut leaf_index = 0usize;
    for (ti, &t) in types.iter().enumerate() {
        let ids = ids_of_type(resources, t);
        write_dir_header(&mut buf, l2_off[ti], ids.len());
        let mut e2 = l2_off[ti] + dir_hdr;
        for &id in &ids {
            write_dir_entry(&mut buf, e2, id as u32, l3_off[leaf_index] as u32, true);
            e2 += dir_ent;
            leaf_index += 1;
        }
    }
    // Level 3 language directories + data entries + blobs.
    for (li, leaf) in leaves(resources).into_iter().enumerate() {
        write_dir_header(&mut buf, l3_off[li], 1);
        write_dir_entry(
            &mut buf,
            l3_off[li] + dir_hdr,
            LANG_EN_US as u32,
            de_off[li] as u32,
            false,
        );
        // IMAGE_RESOURCE_DATA_ENTRY.
        put_u32(&mut buf, de_off[li], rsrc_rva + blob_off[li] as u32);
        put_u32(&mut buf, de_off[li] + 4, leaf.data.len() as u32);
        put_u32(&mut buf, de_off[li] + 8, 0); // CodePage
        put_u32(&mut buf, de_off[li] + 12, 0); // Reserved
        buf[blob_off[li]..blob_off[li] + leaf.data.len()].copy_from_slice(&leaf.data);
    }
    buf
}

fn ids_of_type(resources: &[Resource], type_id: u16) -> Vec<u16> {
    let mut ids: Vec<u16> = resources
        .iter()
        .filter(|r| r.type_id == type_id)
        .map(|r| r.res_id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Leaves in (type asc, id asc) order — the order the directory tree walks them.
fn leaves(resources: &[Resource]) -> Vec<&Resource> {
    let mut types: Vec<u16> = resources.iter().map(|r| r.type_id).collect();
    types.sort_unstable();
    types.dedup();
    let mut out = Vec::new();
    for t in types {
        for id in ids_of_type(resources, t) {
            out.push(
                resources
                    .iter()
                    .find(|r| r.type_id == t && r.res_id == id)
                    .unwrap(),
            );
        }
    }
    out
}

fn leaf_count(resources: &[Resource]) -> usize {
    resources.len()
}

fn write_dir_header(buf: &mut [u8], off: usize, id_entries: usize) {
    // Characteristics, TimeDateStamp, MajorVersion, MinorVersion all 0.
    put_u16(buf, off + 12, 0); // NumberOfNamedEntries
    put_u16(buf, off + 14, id_entries as u16); // NumberOfIdEntries
}

fn write_dir_entry(buf: &mut [u8], off: usize, id: u32, target: u32, is_dir: bool) {
    put_u32(buf, off, id);
    put_u32(
        buf,
        off + 4,
        if is_dir { target | 0x8000_0000 } else { target },
    );
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn put_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// A fusion manifest declaring per-monitor-v2 DPI awareness and Common Controls
/// v6. Kept deterministic (no timestamps) for byte-stable output.
const MANIFEST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="amd64" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true</dpiAware>
    </windowsSettings>
  </application>
</assembly>
"#;

/// Build a `VS_VERSIONINFO` resource from a dotted version string (e.g. "1.2.3").
/// Missing components default to 0; extra components are ignored.
fn build_version_info(version: &str) -> Vec<u8> {
    let mut parts = [0u16; 4];
    for (i, comp) in version.split('.').take(4).enumerate() {
        parts[i] = comp.trim().parse::<u16>().unwrap_or(0);
    }
    let ms = ((parts[0] as u32) << 16) | parts[1] as u32;
    let ls = ((parts[2] as u32) << 16) | parts[3] as u32;

    // VS_FIXEDFILEINFO (52 bytes) — the root node's binary value.
    let mut fixed = Vec::new();
    for v in [
        0xFEEF04BDu32, // dwSignature
        0x0001_0000,   // dwStrucVersion
        ms,            // dwFileVersionMS
        ls,            // dwFileVersionLS
        ms,            // dwProductVersionMS
        ls,            // dwProductVersionLS
        0,             // dwFileFlagsMask
        0,             // dwFileFlags
        0x0004_0004,   // dwFileOS = VOS_NT_WINDOWS32
        1,             // dwFileType = VFT_APP
        0,             // dwFileSubtype
        0,             // dwFileDateMS
        0,             // dwFileDateLS
    ] {
        fixed.extend_from_slice(&v.to_le_bytes());
    }

    let vstr = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]);
    let file_version = vnode_text("FileVersion", &vstr);
    let product_version = vnode_text("ProductVersion", &vstr);
    let string_table = vnode("040904B0", 1, &[], 0, &[file_version, product_version]);
    let string_file_info = vnode("StringFileInfo", 1, &[], 0, &[string_table]);

    // VarFileInfo → Translation = [langid, codepage 1200].
    let mut translation = Vec::new();
    translation.extend_from_slice(&LANG_EN_US.to_le_bytes());
    translation.extend_from_slice(&0x04B0u16.to_le_bytes());
    let var = vnode(
        "Translation",
        0,
        &translation,
        translation.len() as u16,
        &[],
    );
    let var_file_info = vnode("VarFileInfo", 1, &[], 0, &[var]);

    vnode(
        "VS_VERSION_INFO",
        0,
        &fixed,
        fixed.len() as u16,
        &[string_file_info, var_file_info],
    )
}

/// A `String` version node: a UTF-16 (NUL-terminated) text value whose
/// `wValueLength` counts characters (including the NUL).
fn vnode_text(key: &str, value: &str) -> Vec<u8> {
    let mut w: Vec<u8> = value.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    w.extend_from_slice(&[0, 0]); // NUL
    let chars = (w.len() / 2) as u16;
    vnode(key, 1, &w, chars, &[])
}

/// Build one `VS_VERSIONINFO` tree node:
/// `{ WORD wLength; WORD wValueLength; WORD wType; WCHAR szKey[]; pad32; Value; pad32; Child* }`.
/// `wLength` (the whole node length) is back-patched. Value and each child are
/// 32-bit aligned. `value_len` is the raw `wValueLength` (chars for text, bytes
/// for binary), per the spec's per-node convention.
fn vnode(key: &str, wtype: u16, value: &[u8], value_len: u16, children: &[Vec<u8>]) -> Vec<u8> {
    let mut key_w: Vec<u8> = key.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    key_w.extend_from_slice(&[0, 0]); // NUL
    let mut b = Vec::new();
    b.extend_from_slice(&[0, 0]); // wLength (patched below)
    b.extend_from_slice(&value_len.to_le_bytes());
    b.extend_from_slice(&wtype.to_le_bytes());
    b.extend_from_slice(&key_w);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.extend_from_slice(value);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    for c in children {
        b.extend_from_slice(c);
        while b.len() % 4 != 0 {
            b.push(0);
        }
    }
    let len = b.len() as u16;
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_version_only_when_no_icon() {
        // A GUI build with no icon still gets a manifest (DPI) and a version.
        let bytes = build_rsrc(0x5000, None, Some("1.2.3")).expect("rsrc");
        // Level-1 directory NumberOfIdEntries = 2 (RT_MANIFEST + RT_VERSION).
        assert_eq!(u16::from_le_bytes([bytes[14], bytes[15]]), 2);
    }

    #[test]
    fn version_node_length_is_selfconsistent() {
        // The root node's wLength must equal the emitted byte length.
        let v = build_version_info("3.1.4.1");
        let w_length = u16::from_le_bytes([v[0], v[1]]) as usize;
        assert_eq!(
            w_length,
            v.len(),
            "VS_VERSION_INFO wLength must match its bytes"
        );
        // wValueLength == 52 (VS_FIXEDFILEINFO byte count).
        assert_eq!(u16::from_le_bytes([v[2], v[3]]), 52);
    }

    #[test]
    fn data_entry_rva_is_image_relative() {
        // A single manifest: its data entry OffsetToData must be rsrc_rva + offset.
        let rva = 0x9000u32;
        let bytes = build_rsrc(rva, None, None).expect("rsrc");
        // Walk to the one data entry: L1(16+8) → L2(16+8) → L3(16+8) → data entry.
        let de = 24 + 24 + 24;
        let off = u32::from_le_bytes([bytes[de], bytes[de + 1], bytes[de + 2], bytes[de + 3]]);
        assert!(
            off >= rva && off < rva + bytes.len() as u32,
            "data RVA in-section"
        );
    }

    #[test]
    fn icon_source_adds_rt_icon_and_group_icon_types() {
        // A provided 1024×1024 icon exercises the `app_icon.is_some()` branch:
        // render_png over each ICON_SIZES entry, the GRPICONDIRENTRY table, and the
        // GRPICONDIR header. The level-1 directory then carries four type entries
        // (RT_MANIFEST + RT_ICON + RT_GROUP_ICON + RT_VERSION) vs. two without.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("app.png");
        image::RgbaImage::from_pixel(1024, 1024, image::Rgba([9, 40, 128, 255]))
            .save(&path)
            .expect("write 1024 source");
        let bytes = build_rsrc(0x6000, Some(&path), Some("2.0.1")).expect("rsrc with icon");
        assert_eq!(
            u16::from_le_bytes([bytes[14], bytes[15]]),
            4,
            "manifest + icon + group-icon + version = 4 level-1 type entries"
        );
    }
}
