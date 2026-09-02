//! Out-of-line `toString` renderers for the arms whose inline expansion is
//! large (plan-118-C phase 2).
//!
//! # The mechanism
//!
//! These helpers are not re-implementations. Each one builds a **synthesized
//! function** with `CodeBuilder::for_synthetic_function` and then calls the
//! very same `emit_*_to_string_value` emitter the call site used to call
//! inline, so the emitted formatting sequence is identical instruction for
//! instruction — no porting, and no chance of the helper and the (deleted)
//! inline path disagreeing about rounding, which for `Money` would be a
//! user-visible correctness bug rather than a size regression.
//!
//! What the call site keeps is the *error*: the emitter's own
//! `raise_error_bare("ErrOutOfMemory")` runs inside the synthesized function and
//! makes it return an error Result, and the site checks the tag and re-raises
//! with its own `ErrorLoc` — the line and column of the `toString` the program
//! wrote, which a shared helper cannot know. Same Result ABI as
//! `_mfb_rt_float_to_string` and `_mfb_rt_int_to_string`.
//!
//! **That contract is why only SINGLE-error arms may live here.** The site
//! re-raises one fixed code, so an arm that can fail two ways would have its
//! second failure silently reported as the first. `toString(List OF Byte)` can
//! raise `ErrEncoding` as well as `ErrOutOfMemory` and therefore stays inline —
//! it was briefly routed through here and turned "Text encoding or decoding
//! failed" into "Allocation failed", which
//! `rt-error/general/toString_invalid_encoding` caught. Fixed, Money and Scalar
//! raise `ErrOutOfMemory` and nothing else (checked: their only
//! `raise_error_bare` reaches `emit_decimal_alloc_and_copy_integer` /
//! `emit_materialize_string_from_bytes`, both allocation-only).
//!
//! # Which arms
//!
//! Chosen from the corpus, not from the emitters' static size. The `-vv`
//! `toString arm: …` counters over `tests/acceptance` (5,811 sites):
//!
//! | arm | sites | inline instrs/site |
//! |---|---|---|
//! | Integer | 3,675 | ~100 → `_mfb_rt_int_to_string` |
//! | String | 678 | 0 (identity) |
//! | Float | 469 | already out-of-line |
//! | Boolean | 438 | ~0 (two rodata pointers) |
//! | Fixed | 231 | ~230 → here |
//! | Money | 157 | ~234 → here |
//! | AttributedString | 98 | a deep copy, not a render |
//! | Byte | 50 | shares the integer helper |
//! | List OF Byte | 12 | ~467 → stays inline: two error codes, see above |
//! | Scalar | 3 | ~113 → here |

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::NirFunction;
use crate::types::ParameterType;
use std::collections::HashMap;

/// `x0` = the raw scaled i64, `x1` = precision. Returns the allocation Result.
pub(crate) const FIXED_TO_STRING_SYMBOL: &str = "_mfb_rt_fixed_to_string";
/// `x0` = the raw scaled i64, `x1` = precision. Returns the allocation Result.
pub(crate) const MONEY_TO_STRING_SYMBOL: &str = "_mfb_rt_money_to_string";
/// `x0` = a scalar's code point. Returns the allocation Result.
pub(crate) const SCALAR_TO_STRING_SYMBOL: &str = "_mfb_rt_scalar_to_string";

/// Which renderer a synthesized helper wraps.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToStringHelper {
    Fixed,
    Money,
    Scalar,
}

impl ToStringHelper {
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            ToStringHelper::Fixed => FIXED_TO_STRING_SYMBOL,
            ToStringHelper::Money => MONEY_TO_STRING_SYMBOL,
            ToStringHelper::Scalar => SCALAR_TO_STRING_SYMBOL,
        }
    }

    fn name(self) -> &'static str {
        match self {
            ToStringHelper::Fixed => "runtime.fixedToString",
            ToStringHelper::Money => "runtime.moneyToString",
            ToStringHelper::Scalar => "runtime.scalarToString",
        }
    }

    /// Every helper this module can synthesize, for the demand gate.
    pub(crate) fn every() -> [ToStringHelper; 3] {
        [
            ToStringHelper::Fixed,
            ToStringHelper::Money,
            ToStringHelper::Scalar,
        ]
    }
}

/// Synthesize one `toString` renderer as a standalone function.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_to_string_helper<'a>(
    kind: ToStringHelper,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    package_return_types: &'a HashMap<String, ParameterType>,
    platform_imports: &'a HashMap<String, String>,
    platform: &'a dyn CodegenPlatform,
    build_mode: crate::target::NativeBuildMode,
    globals: &'a HashMap<String, GlobalValue>,
    string_symbols: &'a HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let symbol = kind.symbol();
    let mut builder = CodeBuilder::for_synthetic_function(
        symbol,
        function_symbols,
        functions,
        package_return_types,
        platform_imports,
        platform,
        build_mode,
        globals,
        string_symbols,
        type_model,
    );
    // Park the arguments before anything can clobber the argument bank — the
    // renderers below allocate, and an allocation call destroys it
    // (`.ai/arch-abi.md`, "stage ABI args via temporaries").
    let value = builder.allocate_register();
    builder.emit(abi::move_register(&value, abi::c_arg(0)));
    let precision = builder.allocate_register();
    builder.emit(abi::move_register(&precision, abi::c_arg(1)));

    let rendered = match kind {
        ToStringHelper::Fixed => builder.emit_fixed_to_string_value(&value, &precision)?,
        ToStringHelper::Money => builder.emit_money_to_string_value(&value, &precision)?,
        ToStringHelper::Scalar => builder.emit_scalar_to_string_value(&value)?,
    };
    // Success: the standard `(tag, value)` allocation Result. The failure path
    // is already emitted above — the renderer's own `raise_error_bare` returns
    // an error Result from here, which the call site re-raises with its loc.
    builder.emit(abi::move_register(
        RESULT_VALUE_REGISTER,
        &rendered.location,
    ));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    // Same tail as `lower_builtin_function_wrapper`, the other synthesized
    // function: allocate, run the two peepholes, then build the frame.
    builder.run_register_allocation()?;
    let mut instructions = builder.instructions;
    let is_x86 = crate::codegen::engine::mir::active_backend()
        .register_model()
        .arena_base()
        == crate::arch::x86_64::regmodel::ARENA_BASE_REGISTER;
    crate::optimizer::opt2::peephole::forward_stores_to_loads(&mut instructions, is_x86);
    crate::optimizer::opt2::peephole::remove_fp_shuttles(
        &mut instructions,
        crate::codegen::engine::mir::active_backend().register_model(),
    );
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );
    Ok(CodeFunction {
        name: kind.name().to_string(),
        symbol: symbol.to_string(),
        params: Vec::new(),
        returns: "String".to_string(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

impl CodeBuilder<'_> {
    /// Marshal + `bl` + tag check for one of the synthesized renderers above.
    ///
    /// `precision` is `None` for the renderers that take only a value; the
    /// argument register is still written (with zero) so the helper's own
    /// unconditional read of it is defined.
    pub(crate) fn emit_to_string_helper_call(
        &mut self,
        kind: ToStringHelper,
        value: impl Into<Operand>,
        precision: Option<Operand>,
        text: &str,
    ) -> Result<ValueResult, String> {
        let ok = self.label("to_string_helper_ok");
        self.emit(abi::move_register(abi::c_arg(0), value));
        match precision {
            Some(precision) => self.emit(abi::move_register(abi::c_arg(1), precision)),
            None => self.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0")),
        }
        self.emit(abi::branch_link(kind.symbol()));
        self.push_internal_call_relocation(kind.symbol());
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::mfb_return(1)));
        Ok(ValueResult {
            origin: None,
            type_: ParameterType::String,
            location: Operand::from(result.render()),
            text: text.to_string(),
        })
    }
}
