// ---------------------------------------------------------------------------
// Shared fixtures for the writer/reader/round-trip tests below.
// ---------------------------------------------------------------------------

use super::*;
use crate::ir::{IrBinding, IrField, IrParam, IrSourceLoc, IrType, IrVariant};

pub(super) fn loc() -> IrSourceLoc {
    IrSourceLoc::default()
}

pub(super) fn const_int(value: &str) -> IrValue {
    IrValue::Const {
        type_: crate::types::ParameterType::parse("Integer"),
        value: value.to_string(),
    }
}

pub(super) fn fn_named(name: &str, visibility: &str, kind: &str, returns: &str) -> IrFunction {
    IrFunction {
        name: name.to_string(),
        visibility: visibility.to_string(),
        kind: kind.to_string(),
        isolated: false,
        params: vec![],
        returns: crate::types::ParameterType::parse(returns),
        body: vec![],
        file: "src/main.mfb".to_string(),
        resource_owners: std::collections::HashMap::new(),
        loc: loc(),
    }
}

pub(super) fn empty_project(name: &str) -> IrProject {
    IrProject {
        name: name.to_string(),
        entry: None,
        bindings: vec![],
        types: vec![],
        functions: vec![],
        native_resources: vec![],
        link_functions: vec![],
        link_cstructs: Vec::new(),
        link_aliases: vec![],
        docs: crate::ir::ProjectDocs::default(),
        native_libraries: Default::default(),
        max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
    }
}

/// A project that exercises most of the writer: an exported func with a
/// defaulted param, an exported sub, a private isolated func, a record type,
/// a union type, an enum type, globals of every visibility, and an entry
/// point returning Integer with args.
pub(super) fn rich_project() -> IrProject {
    let mut project = empty_project("richpkg");
    project.entry = Some(crate::ir::EntryPoint {
        name: "main".to_string(),
        returns: crate::types::ParameterType::parse("Integer"),
        accepts_args: true,
    });
    project.bindings = vec![
        IrBinding {
            name: "gPriv".to_string(),
            visibility: "private".to_string(),
            mutable: true,
            type_: crate::types::ParameterType::parse("Integer"),
            value: Some(const_int("1")),
            loc: loc(),
            file: String::new(),
            explicit_type: false,
        },
        IrBinding {
            name: "gPkg".to_string(),
            visibility: "public".to_string(),
            mutable: false,
            type_: crate::types::ParameterType::parse("String"),
            value: None,
            loc: loc(),
            file: String::new(),
            explicit_type: false,
        },
        IrBinding {
            name: "gExp".to_string(),
            visibility: "export".to_string(),
            mutable: false,
            type_: crate::types::ParameterType::parse("List OF Integer"),
            value: None,
            loc: loc(),
            file: String::new(),
            explicit_type: false,
        },
    ];
    project.types = vec![
        IrType {
            kind: "type".to_string(),
            visibility: "export".to_string(),
            name: "Point".to_string(),
            fields: vec![
                IrField {
                    visibility: Some("export".to_string()),
                    name: "x".to_string(),
                    type_: crate::types::ParameterType::parse("Integer"),
                    loc: loc(),
                },
                IrField {
                    visibility: Some("private".to_string()),
                    name: "y".to_string(),
                    type_: crate::types::ParameterType::parse("Integer"),
                    loc: loc(),
                },
            ],
            includes: vec![],
            variants: vec![],
            members: vec![],
            loc: loc(),
            file: "src/main.mfb".to_string(),
        },
        IrType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "Shape".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![IrVariant {
                name: "Dot".to_string(),
                fields: vec![IrField {
                    visibility: None,
                    name: "p".to_string(),
                    type_: crate::types::ParameterType::parse("Point"),
                    loc: loc(),
                }],
                loc: loc(),
            }],
            members: vec![],
            loc: loc(),
            file: "src/main.mfb".to_string(),
        },
        IrType {
            kind: "enum".to_string(),
            visibility: "export".to_string(),
            name: "Color".to_string(),
            fields: vec![],
            includes: vec![],
            variants: vec![],
            members: vec![
                crate::ir::IrEnumMember {
                    name: "Red".to_string(),
                },
                crate::ir::IrEnumMember {
                    name: "Green".to_string(),
                },
            ],
            loc: loc(),
            file: "src/main.mfb".to_string(),
        },
    ];
    let mut exported = fn_named("main", "export", "function", "Integer");
    exported.params = vec![
        IrParam {
            name: "n".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
            default: None,
            loc: loc(),
        },
        IrParam {
            name: "m".to_string(),
            type_: crate::types::ParameterType::parse("Integer"),
            default: Some(const_int("0")),
            loc: loc(),
        },
    ];
    let mut isolated = fn_named("worker", "private", "function", "Nothing");
    isolated.isolated = true;
    project.functions = vec![
        exported,
        fn_named("doThing", "export", "sub", "Nothing"),
        isolated,
    ];
    project
}

/// Encode a project to inner MFPC bytes with the given metadata.
pub(super) fn encode_project(project: &IrProject, metadata: &BinaryReprMetadata) -> Vec<u8> {
    build_binary_repr_bytes(project, metadata).expect("encode")
}

/// Wrap inner MFPC bytes in a minimal but valid v1.0 `.mfp` container whose
/// header identity matches an all-empty-key manifest, so
/// `read_package_binary_repr` accepts it.
pub(super) fn wrap_mfp(binary_repr: &[u8], name: &str, ident: &str, version: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00]);
    put_u16(&mut bytes, 1); // container major
    put_u16(&mut bytes, 0); // container minor
    put_u32(&mut bytes, 0); // reserved (offset 12..16)
    put_u32(&mut bytes, 0); // reserved (offset 16..20)
    let put_len = |bytes: &mut Vec<u8>, s: &str| {
        put_u32(bytes, s.len() as u32);
        bytes.extend_from_slice(s.as_bytes());
    };
    put_len(&mut bytes, name);
    put_len(&mut bytes, ident);
    put_len(&mut bytes, version);
    put_len(&mut bytes, ""); // author
    put_len(&mut bytes, ""); // url
    put_len(&mut bytes, ""); // identKey
    put_len(&mut bytes, ""); // signingKey
    put_len(&mut bytes, ""); // proof
    put_len(&mut bytes, ""); // proofSig
    put_len(&mut bytes, ""); // attestation
    put_len(&mut bytes, ""); // attestationSig
    bytes.extend_from_slice(&[0u8; 32]); // packageBinaryHash
    put_u64(&mut bytes, binary_repr.len() as u64);
    put_u16(&mut bytes, 0); // signature type (unsigned)
    put_u32(&mut bytes, 0); // signature length
    bytes.extend_from_slice(binary_repr);
    bytes
}

/// Write a `.mfp` byte blob to a temp file and return its path.
pub(super) fn temp_mfp(bytes: &[u8]) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("mfb-binrepr-test-{}-{}.mfp", std::process::id(), n));
    std::fs::write(&path, bytes).expect("write temp mfp");
    path
}
