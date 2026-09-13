//! The `DOC` table: MFPC section 17, the documentation a compiled package
//! carries (plan-126-D Phase 1; moved from `src/binary_repr` and
//! `src/ast/types.rs`).
//!
//! # Why this can live in the shared crate at all
//!
//! Section 17 is **self-contained**. Every field is inline and length-prefixed —
//! `signature` is a pre-rendered string, `args`/`props`/`errors` are
//! `(String, String)` pairs, `desc` is `(u8 kind, String)` pairs — so decoding it
//! touches no string pool, no type table and no other section. That is what lets
//! the registry read a published package's documentation without the compiler.
//!
//! # Two different things are frozen here, for two different consumers
//!
//! * The **numeric** [`DocProseKind::code`] values and the [`DOC_KIND_FUNC`]..
//!   [`DOC_KIND_RESOURCE`] ids are **`.mfp` wire format**. They are written into
//!   every documented package ever published; a renumbering would silently
//!   mis-decode all of them, and nothing in the type system would notice.
//! * The **string** [`DocProseKind::label`] values (`"desc"`, `"warn"`, …) are
//!   what the compiler's **`-ast` dump** prints. They are frozen by that golden
//!   output, not by the wire.
//!
//! Both are pinned by literal in `the_doc_codes_and_labels_are_frozen`. (The
//! plan that moved this module believed `code()` fed the `-ast` output; it does
//! not — `-ast` serializes `label()`. See plan-126-D's Corrections.)
//!
//! # What stays in the compiler
//!
//! `DocHeaderKind` (the `DOC` block header: `FUNC`/`TYPE`/…/`PACKAGE`) never
//! reaches the wire — it is consumed only by the AST and `doc::from_source` — so
//! it did not follow its sibling here. Likewise `docs_from_ir`, which converts
//! compiler IR into [`PackageDocs`], stays compiler-side: only the wire half of
//! that boundary moved.

use crate::bytes::{
    bounded_capacity, cursor_optional_str, cursor_pair_list, cursor_prose_list, cursor_string,
    cursor_u16, cursor_u32, put_bytes, put_optional_str, put_pair_list, put_prose_list, put_u16,
    put_u32,
};
use crate::mfpc::{read_section_table, SECTION_DOC_TABLE};

/// The kind of a prose block in a `DOC` body: an ordinary description paragraph
/// (`DESC`) or one of the callouts (`WARN`/`INFO`/`SEC`). They interleave in
/// source order so a callout can sit between two paragraphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocProseKind {
    Desc,
    Warn,
    Info,
    Sec,
}

impl DocProseKind {
    pub fn from_keyword(keyword: &str) -> Option<DocProseKind> {
        match keyword.to_ascii_uppercase().as_str() {
            "DESC" => Some(DocProseKind::Desc),
            "WARN" => Some(DocProseKind::Warn),
            "INFO" => Some(DocProseKind::Info),
            "SEC" => Some(DocProseKind::Sec),
            _ => None,
        }
    }

    /// Stable on-wire code for the `.mfp` doc section. **Wire format** — see the
    /// module doc. (This is *not* what the `-ast` dump prints; that is
    /// [`Self::label`].)
    pub fn code(self) -> u8 {
        match self {
            DocProseKind::Desc => 0,
            DocProseKind::Warn => 1,
            DocProseKind::Info => 2,
            DocProseKind::Sec => 3,
        }
    }

    /// Decode a wire code. An unknown code reads as [`DocProseKind::Desc`]
    /// rather than failing, so a later callout kind degrades to a plain
    /// paragraph in an older reader instead of hiding the text.
    pub fn from_code(code: u8) -> DocProseKind {
        match code {
            1 => DocProseKind::Warn,
            2 => DocProseKind::Info,
            3 => DocProseKind::Sec,
            _ => DocProseKind::Desc,
        }
    }

    /// The name the compiler's `-ast` dump prints for this kind. Frozen by that
    /// golden output rather than by the wire.
    pub fn label(self) -> &'static str {
        match self {
            DocProseKind::Desc => "desc",
            DocProseKind::Warn => "warn",
            DocProseKind::Info => "info",
            DocProseKind::Sec => "sec",
        }
    }
}

/// The decoded `doc` section of a compiled package (plan-09-doc.md §5). Empty
/// when the package was built without any exported `DOC` blocks.
#[derive(Clone, Default)]
pub struct PackageDocs {
    pub package: Option<PackageDocEntry>,
    pub decls: Vec<DeclDocEntry>,
}

impl PackageDocs {
    pub fn is_empty(&self) -> bool {
        self.package.is_none() && self.decls.is_empty()
    }
}

#[derive(Clone)]
pub struct PackageDocEntry {
    pub name: String,
    /// Prose blocks as `(kind code, text)` — see [`DocProseKind::code`].
    pub desc: Vec<(u8, String)>,
    pub deprecated: Option<String>,
}

#[derive(Clone)]
pub struct DeclDocEntry {
    /// One of `func`, `sub`, `type`, `union`, `enum`, `resource` — the string
    /// form of a `DOC_KIND_*` id, via [`doc_kind_name`].
    pub kind: String,
    pub name: String,
    pub signature: String,
    /// `GROUP` name (FUNC/SUB), or empty.
    pub group: String,
    /// Prose blocks as `(kind code, text)` — see [`DocProseKind::code`].
    pub desc: Vec<(u8, String)>,
    pub args: Vec<(String, String)>,
    pub props: Vec<(String, String)>,
    pub ret: String,
    pub errors: Vec<(String, String)>,
    pub example: String,
    pub internal: bool,
    pub deprecated: Option<String>,
}

// Declaration-kind ids within section 17. **Wire format**, frozen.
pub const DOC_KIND_FUNC: u16 = 0;
pub const DOC_KIND_SUB: u16 = 1;
pub const DOC_KIND_TYPE: u16 = 2;
pub const DOC_KIND_UNION: u16 = 3;
pub const DOC_KIND_ENUM: u16 = 4;
pub const DOC_KIND_RESOURCE: u16 = 5;

/// The string form of a `DOC_KIND_*` id. An unknown id reads as `"func"`.
pub fn doc_kind_name(kind: u16) -> &'static str {
    match kind {
        DOC_KIND_SUB => "sub",
        DOC_KIND_TYPE => "type",
        DOC_KIND_UNION => "union",
        DOC_KIND_ENUM => "enum",
        DOC_KIND_RESOURCE => "resource",
        _ => "func",
    }
}

/// Decode a section-17 body.
pub fn read_doc_table(bytes: &[u8]) -> Result<PackageDocs, String> {
    let mut offset = 0;
    let has_package = *bytes
        .get(offset)
        .ok_or_else(|| "truncated doc table".to_string())?;
    offset += 1;
    let package = if has_package == 0 {
        None
    } else {
        let name = cursor_string(bytes, &mut offset)?;
        let desc = cursor_prose_list(bytes, &mut offset)?;
        let deprecated = cursor_optional_str(bytes, &mut offset)?;
        Some(PackageDocEntry {
            name,
            desc,
            deprecated,
        })
    };
    let count = cursor_u32(bytes, &mut offset)? as usize;
    // A doc declaration occupies ~40+ wire bytes; use that as the min-element size
    // for the pre-allocation bound (was an understated 2).
    let mut decls = Vec::with_capacity(bounded_capacity(count, bytes.len() - offset, 40));
    for _ in 0..count {
        let kind = doc_kind_name(cursor_u16(bytes, &mut offset)?).to_string();
        let name = cursor_string(bytes, &mut offset)?;
        let signature = cursor_string(bytes, &mut offset)?;
        let group = cursor_string(bytes, &mut offset)?;
        let desc = cursor_prose_list(bytes, &mut offset)?;
        let args = cursor_pair_list(bytes, &mut offset)?;
        let props = cursor_pair_list(bytes, &mut offset)?;
        let ret = cursor_string(bytes, &mut offset)?;
        let errors = cursor_pair_list(bytes, &mut offset)?;
        let example = cursor_string(bytes, &mut offset)?;
        let internal = *bytes
            .get(offset)
            .ok_or_else(|| "truncated doc entry".to_string())?
            != 0;
        offset += 1;
        let deprecated = cursor_optional_str(bytes, &mut offset)?;
        decls.push(DeclDocEntry {
            kind,
            name,
            signature,
            group,
            desc,
            args,
            props,
            ret,
            errors,
            example,
            internal,
            deprecated,
        });
    }
    // bug-282 B3: restore the trailing-bytes invariant every other section
    // enforces (audit-1 PKG-05); the doc table was added afterwards and skipped it.
    if offset != bytes.len() {
        return Err("invalid trailing bytes in doc table".to_string());
    }
    Ok(PackageDocs { package, decls })
}

/// Encode a section-17 body. The inverse of [`read_doc_table`] for every value
/// the compiler emits; `a_real_packages_doc_section_round_trips_byte_for_byte`
/// holds the two to each other on a real published package.
pub fn encode_doc_table(docs: &PackageDocs) -> Vec<u8> {
    let mut bytes = Vec::new();
    match &docs.package {
        Some(package) => {
            bytes.push(1);
            put_bytes(&mut bytes, package.name.as_bytes());
            put_prose_list(&mut bytes, &package.desc);
            put_optional_str(&mut bytes, &package.deprecated);
        }
        None => bytes.push(0),
    }
    put_u32(&mut bytes, docs.decls.len() as u32);
    for decl in &docs.decls {
        let kind = match decl.kind.as_str() {
            "sub" => DOC_KIND_SUB,
            "type" => DOC_KIND_TYPE,
            "union" => DOC_KIND_UNION,
            "enum" => DOC_KIND_ENUM,
            "resource" => DOC_KIND_RESOURCE,
            _ => DOC_KIND_FUNC,
        };
        put_u16(&mut bytes, kind);
        put_bytes(&mut bytes, decl.name.as_bytes());
        put_bytes(&mut bytes, decl.signature.as_bytes());
        put_bytes(&mut bytes, decl.group.as_bytes());
        put_prose_list(&mut bytes, &decl.desc);
        put_pair_list(&mut bytes, &decl.args);
        put_pair_list(&mut bytes, &decl.props);
        put_bytes(&mut bytes, decl.ret.as_bytes());
        put_pair_list(&mut bytes, &decl.errors);
        put_bytes(&mut bytes, decl.example.as_bytes());
        bytes.push(u8::from(decl.internal));
        put_optional_str(&mut bytes, &decl.deprecated);
    }
    bytes
}

/// Decode the documentation carried by an MFPC payload (the `packageBinaryRepr`
/// inside a `.mfp`, not the whole `.mfp` file) — the one call the registry needs
/// (plan-126-E).
///
/// Distinguishes the two cases a caller must treat differently:
///
/// * **`Ok` with an empty [`PackageDocs`]** — the container is valid and simply
///   carries no section 17. That is the *normal* state for an undocumented
///   package.
/// * **`Err`** — the payload is not a valid container, or section 17 is present
///   but malformed.
///
/// Collapsing these into one outcome would be convenient and wrong: plan-126-E's
/// backfill sweep must count a malformed doc section in a stored, signed blob as
/// a *finding*, separately from a package that was never documented.
///
/// Deliberately lighter than the compiler's `binary_repr::read_package_docs`,
/// which runs the full package decode and identity validation first. This one
/// reads only the section table and section 17; it is not a substitute for that
/// check on the compiler side.
pub fn read_package_doc_section(payload: &[u8]) -> Result<PackageDocs, String> {
    let sections = read_section_table(payload)?;
    match sections.get(&SECTION_DOC_TABLE) {
        Some(section) => read_doc_table(section),
        None => Ok(PackageDocs::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mfpc::{encode_sections, Section, SECTION_MANIFEST};

    /// The error of a result expected to fail. `PackageDocs` deliberately does
    /// not derive `Debug` -- the types moved verbatim from the compiler, which
    /// never needed it -- so `unwrap_err` is unavailable. Same idiom as the
    /// compiler's `.mfp` header tests use for the non-`Debug` `MfpHeader`.
    fn err_of(result: Result<PackageDocs, String>) -> String {
        match result {
            Ok(_) => panic!("expected an error, got a decoded doc table"),
            Err(err) => err,
        }
    }

    /// The numeric wire codes and the `-ast` labels are frozen for different
    /// consumers — see the module doc. Pinned by literal, because a renumbering
    /// is invisible to the type checker and would mis-decode every published
    /// documented package.
    #[test]
    fn the_doc_codes_and_labels_are_frozen() {
        assert_eq!(DOC_KIND_FUNC, 0);
        assert_eq!(DOC_KIND_SUB, 1);
        assert_eq!(DOC_KIND_TYPE, 2);
        assert_eq!(DOC_KIND_UNION, 3);
        assert_eq!(DOC_KIND_ENUM, 4);
        assert_eq!(DOC_KIND_RESOURCE, 5);

        for (kind, code, label, keyword) in [
            (DocProseKind::Desc, 0u8, "desc", "DESC"),
            (DocProseKind::Warn, 1, "warn", "WARN"),
            (DocProseKind::Info, 2, "info", "INFO"),
            (DocProseKind::Sec, 3, "sec", "SEC"),
        ] {
            assert_eq!(kind.code(), code, "{kind:?} wire code");
            assert_eq!(DocProseKind::from_code(code), kind, "{kind:?} decode");
            assert_eq!(kind.label(), label, "{kind:?} -ast label");
            assert_eq!(DocProseKind::from_keyword(keyword), Some(kind));
            // Keywords are case-insensitive in source.
            assert_eq!(
                DocProseKind::from_keyword(&keyword.to_ascii_lowercase()),
                Some(kind)
            );
        }
        // Unknown wire code degrades to a paragraph rather than hiding text.
        assert_eq!(DocProseKind::from_code(200), DocProseKind::Desc);
        assert_eq!(DocProseKind::from_keyword("NOTE"), None);

        for (id, name) in [
            (DOC_KIND_FUNC, "func"),
            (DOC_KIND_SUB, "sub"),
            (DOC_KIND_TYPE, "type"),
            (DOC_KIND_UNION, "union"),
            (DOC_KIND_ENUM, "enum"),
            (DOC_KIND_RESOURCE, "resource"),
        ] {
            assert_eq!(doc_kind_name(id), name);
        }
        assert_eq!(doc_kind_name(999), "func");
    }

    /// bug-282 B3: section 17 enforces the trailing-bytes invariant every other
    /// section does. An extra byte after a well-formed table is tampering, not
    /// padding.
    #[test]
    fn trailing_bytes_after_a_doc_table_are_rejected() {
        let mut bytes = encode_doc_table(&PackageDocs::default());
        read_doc_table(&bytes).expect("an empty table is valid");
        bytes.push(0);
        assert_eq!(
            err_of(read_doc_table(&bytes)),
            "invalid trailing bytes in doc table"
        );
    }

    #[test]
    fn a_truncated_doc_table_is_rejected() {
        assert_eq!(err_of(read_doc_table(&[])), "truncated doc table");
        // Declares one package entry, then stops.
        assert!(read_doc_table(&[1]).is_err());
    }

    /// A valid container with no section 17 is the *normal* undocumented case:
    /// `Ok(empty)`. A payload that is not a container is `Err`. The two must
    /// stay distinguishable — plan-126-E counts the second as a finding.
    #[test]
    fn read_package_doc_section_separates_absent_from_malformed() {
        let undocumented = encode_sections(&[Section::new(SECTION_MANIFEST, vec![0; 4])]);
        let docs = read_package_doc_section(&undocumented).expect("absent is not an error");
        assert!(docs.is_empty());

        assert!(read_package_doc_section(b"not an MFPC container").is_err());

        // Present but malformed: section 17 with trailing garbage.
        let mut body = encode_doc_table(&PackageDocs::default());
        body.push(0xFF);
        let malformed = encode_sections(&[Section::new(SECTION_DOC_TABLE, body)]);
        assert_eq!(
            err_of(read_package_doc_section(&malformed)),
            "invalid trailing bytes in doc table"
        );
    }

    /// Extract the MFPC payload from a whole `.mfp` using the shared field
    /// readers. Test-only: the registry gets its payload from `MfpPackage`.
    fn mfp_payload(bytes: &[u8]) -> &[u8] {
        use crate::mfp::{read_mfp_bytes, read_u32, read_u64, FIXED_PREFIX_LEN, MFP_MAGIC};
        assert_eq!(bytes[..8], MFP_MAGIC, "fixture is an .mfp");
        let mut offset = FIXED_PREFIX_LEN;
        for field in [
            "name",
            "ident",
            "version",
            "author",
            "url",
            "identKey",
            "signingKey",
            "proof",
            "proofSig",
            "attestation",
            "attestationSig",
        ] {
            read_mfp_bytes(bytes, &mut offset, field, usize::MAX).unwrap();
        }
        offset += 32; // packageBinaryHash
        let payload_len = read_u64(bytes, offset).unwrap() as usize;
        offset += 8;
        let signature_len = read_u32(bytes, offset + 2).unwrap() as usize;
        offset += 6 + signature_len;
        &bytes[offset..offset + payload_len]
    }

    // Moved verbatim from `src/binary_repr/tests/doc_table_tests.rs` with the
    // section-17 codec it covers (plan-126-D).
    #[test]
    fn doc_table_round_trips() {
        let docs = PackageDocs {
            package: Some(PackageDocEntry {
                name: "mathx".to_string(),
                desc: vec![
                    (0, "First paragraph.".to_string()),
                    (0, "Second.".to_string()),
                ],
                deprecated: Some(String::new()),
            }),
            decls: vec![
                DeclDocEntry {
                    kind: "func".to_string(),
                    name: "addUp".to_string(),
                    signature: "EXPORT FUNC addUp(a AS Integer, b AS Integer) AS Integer"
                        .to_string(),
                    group: "Math".to_string(),
                    desc: vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())],
                    args: vec![
                        ("a".to_string(), "first".to_string()),
                        ("b".to_string(), "second".to_string()),
                    ],
                    props: vec![],
                    ret: "the sum".to_string(),
                    errors: vec![("5001".to_string(), "overflow".to_string())],
                    example: "LET x AS Integer = addUp(1, 2)".to_string(),
                    internal: false,
                    deprecated: None,
                },
                DeclDocEntry {
                    kind: "type".to_string(),
                    name: "Point".to_string(),
                    signature: "EXPORT TYPE Point".to_string(),
                    group: String::new(),
                    desc: vec![],
                    args: vec![],
                    props: vec![("x".to_string(), "the x".to_string())],
                    ret: String::new(),
                    errors: vec![],
                    example: String::new(),
                    internal: true,
                    deprecated: Some("use Coord".to_string()),
                },
            ],
        };

        let bytes = encode_doc_table(&docs);
        let decoded = read_doc_table(&bytes).expect("doc table decodes");

        let package = decoded.package.expect("package entry");
        assert_eq!(package.name, "mathx");
        assert_eq!(package.desc, docs.package.as_ref().unwrap().desc);
        assert_eq!(package.deprecated, Some(String::new()));

        assert_eq!(decoded.decls.len(), 2);
        let add = &decoded.decls[0];
        assert_eq!(add.kind, "func");
        assert_eq!(add.name, "addUp");
        assert_eq!(add.group, "Math");
        assert_eq!(
            add.desc,
            vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())]
        );
        assert_eq!(add.args, docs.decls[0].args);
        assert_eq!(add.errors, docs.decls[0].errors);
        assert_eq!(add.ret, "the sum");
        assert!(!add.internal);
        assert_eq!(add.deprecated, None);

        let point = &decoded.decls[1];
        assert_eq!(point.kind, "type");
        assert_eq!(point.props, docs.decls[1].props);
        assert!(point.internal);
        assert_eq!(point.deprecated, Some("use Coord".to_string()));
    }

    /// **The real-package decode.** A committed, compiler-produced fixture —
    /// `repository/tests/fixtures/libsnd.mfp`, tracked in git, carrying a real
    /// section 17 — rather than `packages/jwt/jwt.mfp`, which is a gitignored
    /// build artifact absent from a fresh checkout (plan-126-D Corrections).
    ///
    /// The strongest property available on real data, and not one a synthetic
    /// fixture can prove: decoding and re-encoding reproduces section 17
    /// **byte for byte**, so the moved codec is exactly the one that wrote it.
    #[test]
    fn a_real_packages_doc_section_round_trips_byte_for_byte() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../repository/tests/fixtures/libsnd.mfp"
        );
        let mfp = std::fs::read(path).expect("committed libsnd.mfp fixture");
        let payload = mfp_payload(&mfp);

        let docs = read_package_doc_section(payload).expect("libsnd's docs decode");
        assert!(
            !docs.decls.is_empty(),
            "libsnd is a documented package; a zero decl count means the decode missed section 17"
        );

        let sections = read_section_table(payload).unwrap();
        let original = sections[&SECTION_DOC_TABLE];
        assert_eq!(
            encode_doc_table(&docs),
            original,
            "re-encoding the decoded docs must reproduce section 17 exactly"
        );
    }
}
