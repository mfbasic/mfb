/// Whether `ctype` is a C ABI type the marshaling backend implements for an
/// `ABI (...)` slot or the ABI return (plan-50-A).
///
/// This is the **slot** namespace. It is deliberately distinct from
/// `is_c_abi_type` (`syntaxcheck::helpers` / `ir::verify`), which answers the
/// opposite question — which names are *banned* from a wrapper's MFBASIC-facing
/// signature (`NATIVE_CPTR_ESCAPE`) — and which excludes `CBool`/`CByte`/`CVoid`
/// on purpose. Do not merge the two lists.
///
/// The set is enumerated from what `lower_link_thunk` actually implements, not
/// from the spec prose; `link_thunk::tests::every_known_ctype_lowers` walks every
/// name here through the thunk to keep the two in step.
///
/// Two names are position-restricted; see [`abi_ctype_valid_as_argument`] and
/// [`abi_ctype_valid_as_return`].
pub(crate) fn abi_slot_ctype_is_known(ctype: &str) -> bool {
    matches!(
        ctype,
        "CPtr"
            | "CString"
            | "CInt8"
            | "CInt16"
            | "CInt32"
            | "CInt64"
            | "CUInt8"
            | "CUInt16"
            | "CUInt32"
            | "CUInt64"
            | "CBool"
            | "CByte"
            | "CFloat"
            | "CDouble"
            | "CVoid"
    )
}

/// Whether `ctype` may appear on an `ABI (...)` argument slot.
///
/// Everything but `CVoid`: a C function takes no `void` argument, so a `CVoid`
/// slot is meaningless. `CVoid` is valid only as the ABI return.
pub(crate) fn abi_ctype_valid_as_argument(ctype: &str) -> bool {
    abi_slot_ctype_is_known(ctype) && ctype != "CVoid"
}

/// Whether `ctype` may appear as the ABI return (`AS <name> <ctype>`).
///
/// Everything but `CString`: `CString` is the *argument* direction — "build a
/// NUL-terminated copy of this MFBASIC `String` for the duration of the call".
/// A C function that returns `char *` is declared `CPtr` and paired with a
/// wrapper `AS String`, which drives the copy-out (`emit_copy_cstring_to_string`).
/// There is no return-side meaning for `CString` and no arm implementing one.
pub(crate) fn abi_ctype_valid_as_return(ctype: &str) -> bool {
    abi_slot_ctype_is_known(ctype) && ctype != "CString"
}

/// The largest total size a `CSTRUCT` may lay out to, in bytes (plan-50-B).
///
/// The struct buffer lives in the marshaling thunk's stack frame, so an unbounded
/// size decoded from a crafted `.mfp` would be a frame-overflow primitive. 1024 is
/// ~32x the largest real binding struct (`SF_INFO` is 32 bytes) and far below
/// anything that threatens the frame.
pub(crate) const MAX_CSTRUCT_SIZE: usize = 1024;

/// A `CSTRUCT <CName> AS <MfbType>` declaration: the byte layout of a C struct and
/// the MFBASIC record it presents as (plan-50-B).
#[derive(Clone)]
pub(crate) struct IrCStruct {
    /// The owning `LINK` alias, e.g. `sndLink`.
    pub(crate) alias: String,
    /// The C-side name, e.g. `SfFormatInfo`. Never escapes the LINK block.
    pub(crate) name: String,
    /// The MFBASIC record type this maps to, e.g. `AudioFormat`.
    pub(crate) maps_to: String,
    /// Fields in C declaration order — the order drives the offsets.
    pub(crate) fields: Vec<IrCStructField>,
}

/// One `CSTRUCT` field: `<name> <ctype>`.
#[derive(Clone)]
pub(crate) struct IrCStructField {
    pub(crate) name: String,
    pub(crate) ctype: String,
}

/// A computed C struct layout: total size, alignment, and each field's byte
/// offset (parallel to the declaration order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CLayout {
    pub(crate) size: usize,
    pub(crate) align: usize,
    pub(crate) offsets: Vec<usize>,
}

/// The `(size, align)` of an ABI ctype as a C struct member, for `target`.
///
/// Every supported target is LP64 — x86-64, aarch64, and riscv64 are LP64, and
/// Windows x64 is LLP64 but the ABI vocabulary is entirely fixed-width, so the
/// table is identical. `target` is threaded anyway because this is an ABI
/// contract: a future ILP32 target (arm64_32, riscv32) would need 4-byte
/// pointers, and finding every call site under time pressure is worse than
/// carrying the parameter now.
///
/// Returns `None` for a name that is not a valid struct-field ctype.
pub(crate) fn ctype_size_align(ctype: &str, target: &str) -> Option<(usize, usize)> {
    debug_assert!(
        !target.contains("arm64_32") && !target.contains("riscv32"),
        "ctype_size_align assumes LP64; an ILP32 target needs its own pointer width"
    );
    let _ = target;
    match ctype {
        // C `_Bool` and `unsigned char`.
        "CInt8" | "CUInt8" | "CBool" | "CByte" => Some((1, 1)),
        "CInt16" | "CUInt16" => Some((2, 2)),
        "CInt32" | "CUInt32" | "CFloat" => Some((4, 4)),
        // `CString` is a `const char *` FIELD here: the pointer, not its bytes.
        "CInt64" | "CUInt64" | "CDouble" | "CPtr" | "CString" => Some((8, 8)),
        // `CVoid` has no storage; it is an ABI return type only.
        _ => None,
    }
}

/// Compute a C struct's layout from its field ctypes, using standard natural
/// alignment (plan-50-B §4.2).
///
/// This is the single place C layout knowledge lives. Offsets are **derived**,
/// never declared or transported — which is what makes the `.mfp` package path
/// safe: a crafted package can choose ctypes, but every ctype has a known
/// size/align, so there is no attacker-supplied offset to forge.
///
/// Verified against `gcc` on real hardware (`sndfile.h`, aarch64):
///   `SF_INFO`        size 32 align 8, offsets 0/8/12/16/20/24
///   `SF_FORMAT_INFO` size 24 align 8, offsets 0/8/16  (4 bytes of pad after `format`)
///
/// Returns `Err` naming the offending field for an unlayoutable ctype.
pub(crate) fn compute_c_layout(
    fields: &[(String, String)],
    target: &str,
) -> Result<CLayout, String> {
    let mut offset = 0usize;
    let mut struct_align = 1usize;
    let mut offsets = Vec::with_capacity(fields.len());
    for (name, ctype) in fields {
        let (fsize, falign) = ctype_size_align(ctype, target)
            .ok_or_else(|| format!("field `{name}` has no C layout for type `{ctype}`"))?;
        offset = offset.div_ceil(falign) * falign;
        offsets.push(offset);
        offset += fsize;
        struct_align = struct_align.max(falign);
    }
    let size = offset.div_ceil(struct_align) * struct_align;
    Ok(CLayout {
        size,
        align: struct_align,
        offsets,
    })
}

/// Collect every slot name a link expression reads (plan-50-I).
///
/// Shared by both checkers so a crafted `.mfp` gets the same rejection source
/// does. `IrLinkExpr::Var` carries the identifier verbatim from lowering, which
/// cannot emit diagnostics, so this is where an unknown name is caught.
pub(crate) fn link_expr_var_names<'a>(expr: &'a IrLinkExpr, out: &mut Vec<&'a str>) {
    match expr {
        IrLinkExpr::Var(name) => out.push(name.as_str()),
        IrLinkExpr::Int(_) => {}
        IrLinkExpr::Compare { lhs, rhs, .. } | IrLinkExpr::And(lhs, rhs) | IrLinkExpr::Or(lhs, rhs) => {
            link_expr_var_names(lhs, out);
            link_expr_var_names(rhs, out);
        }
        IrLinkExpr::Not(inner) => link_expr_var_names(inner, out),
    }
}

/// One reason a `CSTRUCT` declaration is not usable, paired with the rule that
/// reports it (plan-50-B §4.4).
pub(crate) struct CStructFault {
    pub(crate) rule: &'static str,
    pub(crate) message: String,
}

/// Validate one `CSTRUCT` declaration and return every fault found.
///
/// Shared verbatim by the source path (`syntaxcheck`) and the package path
/// (`ir::verify`) so a crafted `.mfp` cannot get a weaker check than source —
/// deliberately unlike `IrFree`, whose ctypes are dropped at lowering, leaving
/// the package path able to check strictly less than the frontend.
///
/// `cstruct_names` is every `CSTRUCT` name declared in the same LINK alias, used
/// to reject nesting.
pub(crate) fn check_cstruct(
    name: &str,
    fields: &[(String, String)],
    cstruct_names: &[String],
    target: &str,
) -> Vec<CStructFault> {
    let mut faults = Vec::new();
    let fault = |rule: &'static str, message: String| CStructFault { rule, message };

    if fields.is_empty() {
        faults.push(fault(
            "NATIVE_CSTRUCT_INVALID",
            format!("CSTRUCT `{name}` declares no fields; a C struct needs at least one."),
        ));
    }

    let mut seen: Vec<&str> = Vec::new();
    for (field, ctype) in fields {
        if seen.contains(&field.as_str()) {
            faults.push(fault(
                "NATIVE_CSTRUCT_INVALID",
                format!("CSTRUCT `{name}` declares field `{field}` more than once."),
            ));
        }
        seen.push(field.as_str());

        // Nesting is out of scope: a struct-valued field would need its own
        // layout recursion and has no consumer. Reject it explicitly rather than
        // letting it fall into "unknown ctype", which would misdescribe the cause.
        if cstruct_names.iter().any(|n| n == ctype) {
            faults.push(fault(
                "NATIVE_CSTRUCT_INVALID",
                format!(
                    "CSTRUCT `{name}` field `{field}` has struct type `{ctype}`; nested structs are not supported."
                ),
            ));
            continue;
        }
        if !abi_slot_ctype_is_known(ctype) {
            faults.push(fault(
                "NATIVE_ABI_UNKNOWN_CTYPE",
                format!("CSTRUCT `{name}` field `{field}` uses unknown C type `{ctype}`."),
            ));
            continue;
        }
        if ctype_size_align(ctype, target).is_none() {
            faults.push(fault(
                "NATIVE_CSTRUCT_INVALID",
                format!(
                    "CSTRUCT `{name}` field `{field}` uses `{ctype}`, which has no storage and cannot be a struct field."
                ),
            ));
        }
    }

    // Only meaningful once every field is layoutable.
    if faults.is_empty() {
        match compute_c_layout(fields, target) {
            Ok(layout) => {
                if layout.size > MAX_CSTRUCT_SIZE {
                    faults.push(fault(
                        "NATIVE_CSTRUCT_TOO_LARGE",
                        format!(
                            "CSTRUCT `{name}` lays out to {} bytes, over the {MAX_CSTRUCT_SIZE}-byte maximum.",
                            layout.size
                        ),
                    ));
                }
            }
            Err(message) => faults.push(fault(
                "NATIVE_CSTRUCT_INVALID",
                format!("CSTRUCT `{name}`: {message}"),
            )),
        }
    }
    faults
}

/// A native `LINK` resource declaration carried through to package metadata.
#[derive(Clone)]
pub(crate) struct IrNativeResource {
    pub(crate) name: String,
    pub(crate) visibility: String,
    /// The registered close op, dotted `alias.func` (plan-link-update.md §5).
    pub(crate) close_function: String,
    pub(crate) sendable: bool,
    pub(crate) close_may_fail: bool,
}

/// A native `LINK` function carried end to end so the backend can emit its MFB↔C
/// marshaling thunk and the per-library dlopen/dlsym initializer (plan-linker.md
/// §12). Unlike [`IrNativeResource`], this carries the full ABI surface.
#[derive(Clone)]
pub(crate) struct IrLinkFunction {
    /// The block binding, e.g. `sqliteLink`.
    pub(crate) alias: String,
    /// The MFBASIC-facing function name, e.g. `open`.
    pub(crate) name: String,
    /// The native library logical name, e.g. `sqlite3`.
    pub(crate) library: String,
    /// The native C symbol, e.g. `sqlite3_open`.
    pub(crate) symbol: String,
    /// Wrapper parameters in declared order: `(name, mfb_type)`.
    pub(crate) params: Vec<(String, String)>,
    /// The wrapper return type (`Db`, `Integer`, `String`, `Boolean`, `Nothing`).
    pub(crate) return_type: String,
    /// Whether the return was declared `RES` (a produced resource handle).
    pub(crate) return_resource: bool,
    /// ABI slots in native C argument order.
    pub(crate) abi_slots: Vec<IrAbiSlot>,
    /// The native return-slot name (`return` ⇒ the C return is the wrapper result;
    /// any other name ⇒ a status used only by `SUCCESS_ON`/`RESULT`).
    pub(crate) abi_return_name: String,
    /// The native return C type, e.g. `CInt32` or `CPtr`.
    pub(crate) abi_return_ctype: String,
    /// `CONST slot = value` pins resolved to an integer immediate.
    pub(crate) consts: Vec<(String, i64)>,
    /// `SUCCESS_ON <expr>` over the native return variable.
    pub(crate) success_on: Option<IrLinkExpr>,
    /// `RESULT <expr>` value mapping over the native return variable.
    pub(crate) result: Option<IrLinkExpr>,
    /// `FREE <slot>` deallocation of a caller-owned native return (mfbasic.md §17).
    pub(crate) free: Option<IrFree>,
}

/// A `FREE` block: after the wrapper copies the produced pointer into its owned
/// MFBASIC result, the original native pointer is passed to `symbol`.
#[derive(Clone)]
pub(crate) struct IrFree {
    /// The produced slot freed (currently always `return`).
    pub(crate) slot: String,
    /// The deallocator native symbol, e.g. `sqlite3_free`.
    pub(crate) symbol: String,
}

/// An `ABI (...)` slot's direction (plan-50-C).
///
/// Replaces the old `is_out: bool`. plan-50-E adds `InOut`, which a bool cannot
/// express, and two bools would admit an illegal `(true, true)` state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AbiDirection {
    /// A C argument marshaled from a wrapper parameter or a `CONST` pin.
    In,
    /// Native storage the callee writes; its value is copied back after the call.
    Out,
    /// Both: fields are written in before the call and read back after. Only
    /// meaningful for a struct slot (plan-50-E).
    InOut,
}

impl AbiDirection {
    /// The wire byte. Decode rejects anything outside `0..=2` — an unknown
    /// direction must be an error, never a silent default.
    pub(crate) fn code(self) -> u8 {
        match self {
            AbiDirection::In => 0,
            AbiDirection::Out => 1,
            AbiDirection::InOut => 2,
        }
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(AbiDirection::In),
            1 => Some(AbiDirection::Out),
            2 => Some(AbiDirection::InOut),
            _ => None,
        }
    }

    /// Whether the callee writes this slot — i.e. it needs an output buffer.
    pub(crate) fn writes_back(self) -> bool {
        matches!(self, AbiDirection::Out | AbiDirection::InOut)
    }
}

/// One `ABI (...)` slot: `name ctype`, `name OUT ctype`, or `name INOUT ctype`.
#[derive(Clone)]
pub(crate) struct IrAbiSlot {
    pub(crate) name: String,
    pub(crate) ctype: String,
    pub(crate) direction: AbiDirection,
}

/// A boolean/integer expression over the function's ABI slot names, used for
/// `SUCCESS_ON`/`RESULT`/`RETURN`. Kept deliberately small: comparisons and
/// boolean connectives over named slots and integer literals cover the surface.
#[derive(Clone)]
pub(crate) enum IrLinkExpr {
    /// The value of a named ABI slot, or of the ABI return (`AS <name> <ctype>`).
    ///
    /// plan-50-I: this used to be a nameless unit variant meaning "the native
    /// return", and `lower_link_expr` mapped *every* identifier onto it — so
    /// `SUCCESS_ON typo = 0` silently meant `status = 0`, and an expression could
    /// not name any other slot even though the spec said it could.
    Var(String),
    Int(i64),
    /// A comparison `lhs <op> rhs` producing `0`/`1`. `op` is one of
    /// `= <> < > <= >=`.
    Compare {
        op: String,
        lhs: Box<IrLinkExpr>,
        rhs: Box<IrLinkExpr>,
    },
    And(Box<IrLinkExpr>, Box<IrLinkExpr>),
    Or(Box<IrLinkExpr>, Box<IrLinkExpr>),
    Not(Box<IrLinkExpr>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(spec: &[(&str, &str)]) -> Vec<(String, String)> {
        spec.iter()
            .map(|(n, t)| ((*n).to_string(), (*t).to_string()))
            .collect()
    }

    /// The ctype list `link_thunk::tests::every_known_ctype_lowers` walks must be
    /// the whole accepted set, or that guard silently stops covering a name. The
    /// two live in different modules, so this is where they are held together.
    #[test]
    fn ctype_list_is_exhaustive() {
        const CTYPES: &[&str] = &[
            "CPtr", "CString", "CInt8", "CInt16", "CInt32", "CInt64", "CUInt8", "CUInt16",
            "CUInt32", "CUInt64", "CBool", "CByte", "CFloat", "CDouble", "CVoid",
        ];
        for ctype in CTYPES {
            assert!(
                abi_slot_ctype_is_known(ctype),
                "{ctype} is in the backend's list but rejected by abi_slot_ctype_is_known"
            );
        }
        // The converse: every name the authority accepts must be in that list.
        // Enumerating a `matches!` needs a candidate set, so probe the spellings
        // any binding could plausibly write plus the accepted ones.
        for candidate in CTYPES
            .iter()
            .chain(["CFloat32", "CIntPtr", "CSize", "CLong", "Cint32"].iter())
        {
            if abi_slot_ctype_is_known(candidate) {
                assert!(
                    CTYPES.contains(candidate),
                    "{candidate} is accepted but link_thunk's CTYPES list omits it"
                );
            }
        }
    }

    /// Ground truth probed with gcc against the real `sndfile.h` on the aarch64
    /// box during planning:
    ///   SF_INFO size=32 align=8 frames=0 samplerate=8 channels=12 format=16
    ///           sections=20 seekable=24
    /// Note `channels` at 12, not 16: MFBASIC's `8*i` record layout gets this
    /// wrong, which is why a real layout computer exists.
    #[test]
    fn sf_info_matches_gcc() {
        let layout = compute_c_layout(
            &fields(&[
                ("frames", "CInt64"),
                ("samplerate", "CInt32"),
                ("channels", "CInt32"),
                ("format", "CInt32"),
                ("sections", "CInt32"),
                ("seekable", "CInt32"),
            ]),
            "macos-aarch64",
        )
        .expect("SF_INFO lays out");
        assert_eq!(layout.size, 32);
        assert_eq!(layout.align, 8);
        assert_eq!(layout.offsets, vec![0, 8, 12, 16, 20, 24]);
    }

    /// gcc: SF_FORMAT_INFO size=24 align=8 format=0 name=8 extension=16.
    /// The 4 bytes of padding after `format` are the case that proves natural
    /// alignment is being applied rather than fields being packed.
    #[test]
    fn sf_format_info_matches_gcc() {
        let layout = compute_c_layout(
            &fields(&[
                ("format", "CInt32"),
                ("name", "CString"),
                ("extension", "CString"),
            ]),
            "macos-aarch64",
        )
        .expect("SF_FORMAT_INFO lays out");
        assert_eq!(layout.size, 24);
        assert_eq!(layout.align, 8);
        assert_eq!(layout.offsets, vec![0, 8, 16]);
    }

    #[test]
    fn single_byte_field() {
        let layout = compute_c_layout(&fields(&[("flag", "CInt8")]), "macos-aarch64").unwrap();
        assert_eq!((layout.size, layout.align, layout.offsets), (1, 1, vec![0]));
    }

    /// Trailing padding: a 1-byte field followed by an 8-byte one pads to 16, not 9.
    #[test]
    fn pads_to_struct_alignment() {
        let layout = compute_c_layout(
            &fields(&[("flag", "CInt8"), ("big", "CInt64")]),
            "macos-aarch64",
        )
        .unwrap();
        assert_eq!((layout.size, layout.align, layout.offsets), (16, 8, vec![0, 8]));
    }

    /// A trailing small field still pads the struct out to its alignment.
    #[test]
    fn pads_after_last_field() {
        let layout = compute_c_layout(
            &fields(&[("big", "CInt64"), ("flag", "CInt8")]),
            "macos-aarch64",
        )
        .unwrap();
        assert_eq!((layout.size, layout.align, layout.offsets), (16, 8, vec![0, 8]));
    }

    #[test]
    fn all_byte_fields_need_no_padding() {
        let layout = compute_c_layout(
            &fields(&[("a", "CInt8"), ("b", "CByte"), ("c", "CBool")]),
            "macos-aarch64",
        )
        .unwrap();
        assert_eq!((layout.size, layout.align, layout.offsets), (3, 1, vec![0, 1, 2]));
    }

    #[test]
    fn sixteen_bit_alignment() {
        let layout = compute_c_layout(
            &fields(&[("a", "CInt8"), ("b", "CInt16"), ("c", "CInt32")]),
            "macos-aarch64",
        )
        .unwrap();
        assert_eq!((layout.size, layout.align, layout.offsets), (8, 4, vec![0, 2, 4]));
    }

    #[test]
    fn rejects_cvoid_field() {
        let err = compute_c_layout(&fields(&[("nothing", "CVoid")]), "macos-aarch64").unwrap_err();
        assert!(err.contains("CVoid"), "{err}");
    }

    #[test]
    fn rejects_unknown_field_ctype() {
        let err = compute_c_layout(&fields(&[("x", "CSize")]), "macos-aarch64").unwrap_err();
        assert!(err.contains("CSize"), "{err}");
    }

    /// The layout table must be identical on every supported target — all four are
    /// LP64 for this vocabulary.
    #[test]
    fn layout_is_target_invariant() {
        let f = fields(&[
            ("format", "CInt32"),
            ("name", "CString"),
            ("extension", "CString"),
        ]);
        let a = compute_c_layout(&f, "macos-aarch64").unwrap();
        for target in ["linux-x86_64", "linux-aarch64", "linux-riscv64", "windows-x86_64"] {
            assert_eq!(compute_c_layout(&f, target).unwrap(), a, "{target} differs");
        }
    }
}
