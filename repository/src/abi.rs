//! Minimal reader for the per-symbol ABI index the compiler embeds in a
//! package's `packageBinaryRepr` (plan-10-B1). The registry does not decode
//! the whole MFPC container — it only needs the string pool (section 2) and
//! the ABI index (section 15) to publish `{ "<symbol>": "<hex sigHash>" }` for
//! resolution. The section lives inside `packageBinaryRepr`, so plan-23's
//! `packageBinaryHash` and the package signature already cover it; the registry
//! parses it best-effort and never has to trust it for correctness.

use std::collections::BTreeMap;

// plan-126-C: the MFPC magic, the section ids, the field/format versions, the
// limits and the `libc`/`lib_type` wire encodings all come from
// `mfb_wire::mfpc` now. They used to be **restated** here, under a comment
// explaining that this crate does not depend on the compiler crate and citing
// `src/binary_repr/mod.rs` as the source of truth — fifteen constants kept in
// step by hand. `mfb_wire` is the crate both may depend on, so the apology and
// the copies are both gone.
//
// `read_section_table` came from there too, and that one mattered more than the
// constants: it decodes bytes a package signature covers, and the two
// hand-synchronized copies had already drifted in **both** directions. This
// crate's version lacked the MFPC major-version check and used `as usize` where
// the compiler used `checked_usize`; the compiler's lacked the
// `MAX_MFPC_SECTIONS` ceiling this one added in bug-578. The shared reader
// enforces the union, so both sides gained a guard.
use mfb_wire::bytes::ABI_HASH_LEN;
use mfb_wire::mfpc::{
    read_section_table, ABI_FORMAT_VERSION, MAX_DESCRIPTION_BYTES, NATIVE_LIBRARY_HASH_LEN,
    PACKAGE_META_FIELD_DESCRIPTION, SECTION_ABI_INDEX, SECTION_MANIFEST,
    SECTION_NATIVE_LIBRARY_TABLE, SECTION_PACKAGE_META, SECTION_STRING_POOL, WIRE_LIBC_GLIBC,
    WIRE_LIBC_MUSL, WIRE_LIBC_UNSPECIFIED, WIRE_LIB_TYPE_VENDOR,
};

/// Decode the `libc` wire byte into the token stored in
/// `package_version_targets.libc`. An unrecognized value is an error, not a
/// silent `None`: section 10 rides inside the signed payload, so a byte outside
/// the vocabulary means a broken or tampered package, and reporting it as
/// "no libc constraint" would let a tampered locator widen its own platform
/// match.
fn decode_libc(raw: u8) -> Result<Option<String>, String> {
    match raw {
        WIRE_LIBC_UNSPECIFIED => Ok(None),
        WIRE_LIBC_GLIBC => Ok(Some("glibc".to_string())),
        WIRE_LIBC_MUSL => Ok(Some("musl".to_string())),
        other => Err(format!(
            "native library locator declares unknown libc discriminant {other}"
        )),
    }
}

/// Ceilings on what section 10 may declare (bug-275).
///
/// `entry_count` and `locator_count` are raw `u32`s read straight off an
/// attacker-controlled payload, and parsing previously stopped only when the
/// offset ran off the end. At ~46 bytes per locator a single ~48 MiB payload can
/// encode on the order of a million of them, and `validate_package_request`
/// probes the blob store once per locator — an S3 `head_object` each on the
/// hosted backend. One `/validate` or `/publish` could therefore fan out to ~1M
/// backend operations, reachable by any self-registered owner.
///
/// A real `libraries` table is one entry per logical library with one locator per
/// supported platform triple, so these bounds are orders of magnitude above any
/// legitimate package while capping the fan-out.
const MAX_VENDOR_ENTRIES: usize = 1024;
const MAX_VENDOR_LOCATORS: usize = 4096;

/// Format-level ceilings on what an MFPC container may declare (bug-578).
///
/// Every count below is a raw `u32` read straight off an attacker-controlled,
/// authenticated-but-untrusted payload, and parsing previously stopped only
/// when the running offset ran off the end of the section. That makes the
/// 64 MiB request-body limit (`server.rs:MAX_BODY_BYTES`) the only bound — and
/// it is the wrong bound, because the in-memory representation is several times
/// larger than the wire bytes that declare it. The worst case is the string
/// pool: a `String` is 24 bytes on a 64-bit target while the smallest possible
/// entry is its 4-byte length prefix, so a ~48 MiB pool of empty entries
/// expands to a ~288 MiB `Vec<String>` on a 512 MiB server, reachable by any
/// self-registered owner through `/validate` or `/publish`.
///
/// The values are derived from the compiler's *actual* maxima, measured across
/// every committed compiler-produced package (`packages/*/*.mfp`):
///
/// | package     | sections | pool entries | pool bytes | ABI exports | meta fields |
/// |-------------|---------:|-------------:|-----------:|------------:|------------:|
/// | cli         |       11 |           45 |        423 |           8 |           1 |
/// | json_schema |       12 |          628 |      9,175 |          24 |           1 |
/// | jwt         |       12 |          914 |     12,763 |          48 |           1 |
/// | libsnd      |       14 |           56 |        669 |          12 |           1 |
/// | mustache    |       12 |          305 |      4,112 |          12 |           1 |
/// | sqlite3     |       14 |           64 |        767 |          28 |           1 |
/// | yaml        |       12 |          356 |      4,600 |          11 |           1 |
///
/// Each ceiling therefore sits three to four orders of magnitude above anything
/// the compiler emits today. They bound the damage; they are not a schema.
///
// `MAX_MFPC_SECTIONS` moved to `mfb_wire::mfpc` with `read_section_table`, the
// only thing that enforces it (plan-126-C). The measurement table above is the
// evidence for it and for the four ceilings below, so it stays here; the
// constant's own rationale travelled with it.

/// `MAX_STRING_POOL_ENTRIES`: 1,147x the largest real pool (jwt, 914 entries).
/// Caps the `Vec<String>` at ~24 MiB of `String` headers.
const MAX_STRING_POOL_ENTRIES: usize = 1024 * 1024;

/// `MAX_STRING_POOL_BYTES`: aggregate decoded string bytes, ~2,600x the largest
/// real pool (jwt, 12,763 bytes) and half the request-body limit. The pool's
/// bytes are 1:1 with the wire, so this is the weaker of the two pool bounds —
/// it exists so the pool's share of a 64 MiB body stays bounded on its own.
const MAX_STRING_POOL_BYTES: usize = 32 * 1024 * 1024;

/// `MAX_ABI_EXPORTS`: 5,461x the largest real index (jwt, 48 exports). An export
/// is 38 wire bytes, so the body limit alone allowed ~1.7 million iterations,
/// each a `hex::encode` allocation, on every `/validate` and `/publish`.
const MAX_ABI_EXPORTS: usize = 256 * 1024;

/// `MAX_PACKAGE_META_FIELDS`: the writer emits exactly one field (the
/// description, `src/binary_repr/writer.rs:1127`). An unknown field id is
/// *skipped*, which is what makes a later field additive — and what made the
/// loop free to spin ~10 million times on a 64 MiB payload.
const MAX_PACKAGE_META_FIELDS: usize = 256;

/// One `vendor` locator drawn from a package's section-10 `NATIVE_LIBRARY_TABLE`
/// — the logical library it belongs to, its bare source filename, the hex
/// SHA-256 that is the vendored file's blob key (plan-48-A §4.4), and the
/// platform triple it applies to (plan-61-A §3).
///
/// One `VendorBlobRef` is one *locator*, not one distinct blob. Two locators
/// may legitimately share a `hash` — two platforms shipping a byte-identical
/// build under different `source` filenames — so callers must never dedupe by
/// hash when accumulating platform support, or they under-report targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorBlobRef {
    pub logical: String,
    pub source: String,
    pub hash: String,
    pub os: String,
    /// `None` is the any-arch wildcard (an empty `arch` string on the wire),
    /// meaning the locator matches every architecture on its OS. It is not
    /// missing data.
    pub arch: Option<String>,
    /// `None` when the locator specifies no libc; otherwise `"glibc"` or
    /// `"musl"`. A token rather than the wire integer so the value needs no
    /// mapping table downstream (plan-61-A §Open Decisions).
    pub libc: Option<String>,
    /// `"vendor"` or `"system"`. Constant `"vendor"` for every ref this parser
    /// produces (§3.1) — carried so capturing `system` locators later is not a
    /// schema change.
    pub lib_type: String,
}

/// Parse every `vendor` locator's blob hash from a package's `packageBinaryRepr`
/// payload (plan-48-A §4.4). Returns an empty vector when the package carries no
/// section-10 table (nothing to vendor). Errors only when a table is present but
/// malformed — which, since section 10 rides inside the signed+welded payload,
/// means a broken or tampered package the caller should refuse rather than
/// silently treat as vendoring nothing.
pub fn parse_vendor_blobs(payload: &[u8]) -> Result<Vec<VendorBlobRef>, String> {
    // A payload that is not even an MFPC container carries no section 10, so it
    // vendors nothing. Treated best-effort (like `abi_index_json`): the payload
    // is welded to the signature, so a genuinely broken one fails
    // `verify_payload_hash` elsewhere, and the registry never has to trust the
    // table for correctness. Only a *malformed section 10 inside a valid
    // container* is a real error worth surfacing.
    let Ok(sections) = read_section_table(payload) else {
        return Ok(Vec::new());
    };
    let Some(table_bytes) = sections.get(&SECTION_NATIVE_LIBRARY_TABLE) else {
        return Ok(Vec::new());
    };
    let string_bytes = sections
        .get(&SECTION_STRING_POOL)
        .ok_or_else(|| "package is missing the string pool section".to_string())?;
    let strings = read_string_pool(string_bytes)?;
    read_native_vendor_locators(table_bytes, &strings)
}

fn read_native_vendor_locators(
    bytes: &[u8],
    strings: &[String],
) -> Result<Vec<VendorBlobRef>, String> {
    let mut offset = 0usize;
    let entry_count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Reject on the declared count before allocating or probing anything.
    if entry_count > MAX_VENDOR_ENTRIES {
        return Err(format!(
            "native library table declares {entry_count} entries (limit {MAX_VENDOR_ENTRIES})"
        ));
    }
    let mut total_locators = 0usize;
    let mut refs = Vec::new();
    for _ in 0..entry_count {
        let logical = table_string(strings, read_u32(bytes, offset)?)?;
        offset += 4;
        let locator_count = read_u32(bytes, offset)? as usize;
        offset += 4;
        // Bound the *total* across entries, not just each entry: many small
        // entries reach the same fan-out as one oversized one.
        total_locators = total_locators.saturating_add(locator_count);
        if total_locators > MAX_VENDOR_LOCATORS {
            return Err(format!(
                "native library table declares more than {MAX_VENDOR_LOCATORS} locators"
            ));
        }
        for _ in 0..locator_count {
            // os, arch: interned string ids. An empty `arch` is the any-arch
            // wildcard and becomes `None` — distinct from a concrete arch, and
            // never conflated with an absent locator (plan-61-A §3, gotcha 1).
            let os = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            let arch = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            let arch = if arch.is_empty() { None } else { Some(arch) };
            let libc = decode_libc(read_u8(bytes, offset)?)?;
            offset += 1;
            let lib_type = read_u8(bytes, offset)?;
            offset += 1;
            let source = table_string(strings, read_u32(bytes, offset)?)?;
            offset += 4;
            if lib_type == WIRE_LIB_TYPE_VENDOR {
                let end = offset
                    .checked_add(NATIVE_LIBRARY_HASH_LEN)
                    .ok_or("native library locator hash overflows")?;
                let raw = bytes
                    .get(offset..end)
                    .ok_or("truncated native library locator hash")?;
                refs.push(VendorBlobRef {
                    logical: logical.clone(),
                    source,
                    hash: hex::encode(raw),
                    os,
                    arch,
                    libc,
                    lib_type: "vendor".to_string(),
                });
                offset = end;
            }
        }
    }
    Ok(refs)
}

/// The human-facing metadata carried in MANIFEST section 1, resolved through
/// the string pool (plan-61-A §4).
///
/// This is the copy the registry renders. The `.mfp` header carries the same
/// two strings in plaintext as a fast-scan convenience, but *this* copy lives
/// inside `packageBinaryRepr`, which `packageBinaryHash` covers and the package
/// signature covers transitively. A registry whose pitch is transparency must
/// render what the publisher signed, not what a plaintext prefix claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestMetadata {
    pub author: String,
    pub url: String,
}

/// Read the `description` from section 18 (plan-61-D §3).
///
/// `Ok(None)` when the package carries no section 18 — the normal case for
/// every package built before plan-61-D and every package that declares no
/// description. Unknown field ids inside the section are **skipped**, which is
/// what makes a later field additive.
pub fn parse_package_description(payload: &[u8]) -> Result<Option<String>, String> {
    let Ok(sections) = read_section_table(payload) else {
        return Ok(None);
    };
    let Some(bytes) = sections.get(&SECTION_PACKAGE_META) else {
        return Ok(None);
    };
    let mut offset = 0usize;
    let field_count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Reject on the declared count before the loop (bug-578). Unknown field ids
    // are skipped rather than rejected — which is what makes a later field
    // additive, and what left this loop free to spin ~10 million times.
    if field_count > MAX_PACKAGE_META_FIELDS {
        return Err(format!(
            "package meta section declares {field_count} fields \
             (limit {MAX_PACKAGE_META_FIELDS})"
        ));
    }
    let mut description = None;
    for _ in 0..field_count {
        let field_id = read_u16(bytes, offset)?;
        offset += 2;
        let length = read_u32(bytes, offset)? as usize;
        offset += 4;
        let end = offset
            .checked_add(length)
            .ok_or("package meta field length overflows")?;
        let value = bytes
            .get(offset..end)
            .ok_or("truncated package meta field")?;
        offset = end;
        if field_id == PACKAGE_META_FIELD_DESCRIPTION {
            if length > MAX_DESCRIPTION_BYTES {
                return Err(format!(
                    "package description exceeds the {MAX_DESCRIPTION_BYTES} byte limit \
                     ({length} bytes)"
                ));
            }
            description = Some(
                String::from_utf8(value.to_vec())
                    .map_err(|_| "package description is not valid UTF-8".to_string())?,
            );
        }
    }
    Ok(description)
}

/// Byte offsets of `author` and `url` within MANIFEST section 1. The section is
/// a fixed positional record: six u32 identity fields precede `author`
/// (`src/binary_repr/writer.rs:993-1000`, read back at
/// `src/binary_repr/reader.rs:975-982`).
const MANIFEST_AUTHOR_OFFSET: usize = 24;
const MANIFEST_URL_OFFSET: usize = 28;

/// Read `author` and `url` from a payload's MANIFEST section.
///
/// `Ok(None)` when the payload carries no section 1 at all — the same
/// best-effort posture `parse_vendor_blobs` takes toward a missing section 10,
/// and for the same reason: the payload is welded to the signature, so a
/// genuinely broken one fails `verify_payload_hash` elsewhere. A section that
/// is *present but malformed* is an error.
pub fn parse_manifest_metadata(payload: &[u8]) -> Result<Option<ManifestMetadata>, String> {
    let Ok(sections) = read_section_table(payload) else {
        return Ok(None);
    };
    let Some(manifest_bytes) = sections.get(&SECTION_MANIFEST) else {
        return Ok(None);
    };
    let string_bytes = sections
        .get(&SECTION_STRING_POOL)
        .ok_or_else(|| "package is missing the string pool section".to_string())?;
    let strings = read_string_pool(string_bytes)?;
    let author = table_string(&strings, read_u32(manifest_bytes, MANIFEST_AUTHOR_OFFSET)?)?;
    let url = table_string(&strings, read_u32(manifest_bytes, MANIFEST_URL_OFFSET)?)?;
    Ok(Some(ManifestMetadata { author, url }))
}

fn table_string(strings: &[String], id: u32) -> Result<String, String> {
    strings
        .get(id as usize)
        .cloned()
        .ok_or_else(|| "native library table names an out-of-range string".to_string())
}

/// Parse the exported-symbol ABI map from a package's `packageBinaryRepr`
/// payload: `{ "<name>": "<hex sigHash>" }`, keys sorted. Errors if the payload
/// is not an MFPC container or the ABI/string sections are malformed; callers
/// treat any error as "no ABI index" (an empty object).
pub fn parse_abi_index(payload: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let sections = read_section_table(payload)?;
    let string_bytes = sections
        .get(&SECTION_STRING_POOL)
        .ok_or_else(|| "package is missing the string pool section".to_string())?;
    let abi_bytes = sections
        .get(&SECTION_ABI_INDEX)
        .ok_or_else(|| "package is missing the ABI index section".to_string())?;
    let strings = read_string_pool(string_bytes)?;
    read_abi_exports(abi_bytes, &strings)
}

/// The JSON `abiIndex` value for a package payload (empty object when absent).
pub fn abi_index_json(payload: &[u8]) -> serde_json::Value {
    match parse_abi_index(payload) {
        Ok(map) => serde_json::Value::Object(
            map.into_iter()
                .map(|(name, hash)| (name, serde_json::Value::String(hash)))
                .collect(),
        ),
        Err(_) => serde_json::json!({}),
    }
}

fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0usize;
    let count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Bound the pre-allocation by how many entries the section could actually
    // hold, not by its byte length (bug-276 R8). A `String` is 24 bytes while the
    // smallest possible entry is its 4-byte length prefix, so `count.min(len)`
    // over-reserved by up to 24x — a ~48 MiB section declaring a huge count forced
    // a ~1.15 GiB transient allocation, on every /validate and /publish, for a
    // pool that can hold at most `len / 4` strings.
    //
    // That cap is still not an absolute one (bug-578): `bytes.len() / 4` is
    // exactly the number of empty entries an attacker supplies, so a ~48 MiB
    // pool of 4-byte entries still built ~12 million `String`s — ~288 MiB on a
    // 512 MiB server. The entry ceiling below is what bounds it; the `min` is
    // kept because it still avoids over-reserving for a *truncated* pool.
    if count > MAX_STRING_POOL_ENTRIES {
        return Err(format!(
            "string pool declares {count} entries (limit {MAX_STRING_POOL_ENTRIES})"
        ));
    }
    let mut strings = Vec::with_capacity(count.min(bytes.len() / 4));
    let mut total_bytes = 0usize;
    for _ in 0..count {
        let length = read_u32(bytes, offset)? as usize;
        offset += 4;
        // Bound the aggregate before the entry is copied out, not after.
        total_bytes = total_bytes.saturating_add(length);
        if total_bytes > MAX_STRING_POOL_BYTES {
            return Err(format!(
                "string pool declares more than {MAX_STRING_POOL_BYTES} bytes of strings"
            ));
        }
        let end = offset.checked_add(length).ok_or("bad string length")?;
        if end > bytes.len() {
            return Err("truncated string pool entry".to_string());
        }
        strings.push(
            std::str::from_utf8(&bytes[offset..end])
                .map_err(|_| "string pool entry is not valid UTF-8".to_string())?
                .to_string(),
        );
        offset = end;
    }
    Ok(strings)
}

fn read_abi_exports(bytes: &[u8], strings: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut offset = 0usize;
    let version = read_u16(bytes, offset)?;
    offset += 2;
    if version != ABI_FORMAT_VERSION {
        return Err(format!("unsupported ABI index format version {version}"));
    }
    offset += 2; // reserved
    let export_count = read_u32(bytes, offset)? as usize;
    offset += 4;
    // Reject on the declared count before the loop (bug-578): an export is 38
    // wire bytes, so the request-body limit alone allowed ~1.7 million
    // iterations, each a `hex::encode` allocation.
    if export_count > MAX_ABI_EXPORTS {
        return Err(format!(
            "ABI index declares {export_count} exports (limit {MAX_ABI_EXPORTS})"
        ));
    }
    let mut map = BTreeMap::new();
    for _ in 0..export_count {
        let name_index = read_u32(bytes, offset)? as usize;
        offset += 4;
        let _kind = read_u16(bytes, offset)?;
        offset += 2;
        let hash_end = offset.checked_add(ABI_HASH_LEN).ok_or("bad ABI hash")?;
        if hash_end > bytes.len() {
            return Err("truncated ABI export hash".to_string());
        }
        let hash = hex::encode(&bytes[offset..hash_end]);
        offset = hash_end;
        let name = strings
            .get(name_index)
            .ok_or_else(|| "ABI export names an out-of-range string".to_string())?;
        map.insert(name.clone(), hash);
    }
    Ok(map)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let slice = bytes.get(offset..offset + 2).ok_or("truncated u16")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u8(bytes: &[u8], offset: usize) -> Result<u8, String> {
    bytes.get(offset).copied().ok_or("truncated u8".to_string())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or("truncated u32")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

// `read_u64` is gone (plan-126-C). Its only production caller was this file's
// `read_section_table`, which moved to `mfb_wire::mfpc` — where the equivalent
// is `bytes::checked_u64_at`, with its own tests. Keeping it here would have
// left a helper used by nothing but a test that tested the helper.

/// A minimal `vendor` locator for tests that care only about the blob hash —
/// the version→blob edges in `store.rs` and the reachability walk in `gc.rs`,
/// which predate the platform axis plan-61-A added. Shared here so those tests
/// keep asserting what they always asserted without each restating a full
/// platform triple they do not use.
#[cfg(test)]
pub(crate) fn vendor_ref_for_hash(hash: &str) -> VendorBlobRef {
    VendorBlobRef {
        logical: "libtest".to_string(),
        source: format!("{hash}.a"),
        hash: hash.to_string(),
        os: "linux".to_string(),
        arch: Some("x86_64".to_string()),
        libc: None,
        lib_type: "vendor".to_string(),
    }
}

#[cfg(test)]
mod tests;
