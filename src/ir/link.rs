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
