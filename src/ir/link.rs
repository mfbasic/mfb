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

/// One `ABI (...)` slot: `name ctype` or `name OUT ctype`.
#[derive(Clone)]
pub(crate) struct IrAbiSlot {
    pub(crate) name: String,
    pub(crate) ctype: String,
    pub(crate) is_out: bool,
}

/// A boolean/integer expression over the single native return variable, used for
/// `SUCCESS_ON`/`RESULT`. Kept deliberately small: comparisons and boolean
/// connectives over the return variable and integer literals cover the surface.
#[derive(Clone)]
pub(crate) enum IrLinkExpr {
    /// The native return variable (the `AS <name> <ctype>` value).
    Var,
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
