//! Per-record-type construction functions (plan-118-D).
//!
//! A `Point(x, y, label)` used to inline, at every site: the arena allocation,
//! the ~194-instruction allocation-failure block, the per-field stores, and a
//! byte-copy loop for each `String` field (records inline their String fields —
//! `.ai/codegen-invariants.md`). `val:Constructor` was 2,173,050 exclusive
//! builder-emitted instructions over 7,876 sites, the second-largest expansion
//! category in the module.
//!
//! The per-type census that chose what to synthesize (`-vv`
//! `costliest constructor type`, `tests/acceptance`, 59 distinct types,
//! 4,147,094 instructions inclusive):
//!
//! ```text
//!    3104179      3609x  record Error
//!     833679      3609x  record ErrorLoc
//!      33646        35x  record vector.Fixed4
//!      30095        43x  record vector.Fixed3
//!         ...
//! ```
//!
//! Two types are **95 %** of it, at 860 and 231 instructions per site, and both
//! are the compiler's own error plumbing: every `FAIL` builds an `Error` whose
//! third field is a fresh `ErrorLoc`, which is why their site counts are equal
//! and equal to `op:Fail`'s.
//!
//! # Mechanism
//!
//! The same one plan-118-C's `toString` renderers use: a **synthesized**
//! function that calls the very emitter the site used to inline
//! (`emit_build_inlined_record`), so the field layout, the String inlining and
//! the allocation are identical instruction for instruction and the one-layout
//! law is not forked. Arguments arrive by the ordinary user-function convention
//! — `x0`–`x7` then the stack tail (bug-08) — so a wide record needs no second
//! convention.
//!
//! The allocation error stays at the CALL SITE: an `ErrOutOfMemory` carries the
//! `ErrorLoc` of the construction that failed. `emit_build_inlined_record`
//! raises nothing else, which is what makes the single-code helper contract
//! sound here (see `to_string_helpers` for what happens when it is not).

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::target::shared::abi;
use crate::target::shared::nir::{self, NirFunction, NirValue};
use crate::types::ParameterType;
use std::collections::{HashMap, HashSet};

/// How many construction sites a record type needs before it gets its own
/// function. Below this the body is roughly the inline code it replaces, so the
/// call is pure overhead.
///
/// Three is the plan's recommendation and the census supports it: the
/// distribution is extremely top-heavy (two types are 95 %), so the threshold's
/// exact value moves almost nothing — 34 of the 59 types have ≥ 3 sites and
/// together they are 99 % of the cost.
pub(crate) const CONSTRUCT_SITE_THRESHOLD: usize = 3;

/// The emitted symbol for `type_`'s construction function.
pub(crate) fn construct_symbol(type_: &ParameterType) -> String {
    // An emitted symbol name, so the type renders here — the type -> symbol
    // boundary, as in `thread_copy_symbol`.
    let mut sanitized = String::new();
    for ch in type_.name().chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    format!("_mfb_ctor_{sanitized}")
}

/// The record types this module constructs at least [`CONSTRUCT_SITE_THRESHOLD`]
/// times, each with its field count, **sorted by type name**.
///
/// Counted over the NIR through the one authoritative traversal (bug-328), so a
/// new `NirOp`/`NirValue` variant cannot hide a construction site from the
/// count and leave a type synthesized-but-unamortized (or worse, a site calling
/// a function that was never synthesized).
///
/// The sort is load-bearing, not tidiness: the caller emits one function per
/// entry, and the ORDER functions appear in the module is observable in the
/// `.ncode`. Returning the `HashMap` directly made three builds of one fixture
/// produce three different `sha256`s — a compiler whose output depends on hash
/// seeding, which every byte-identity golden would flap against.
pub(crate) fn synthesized_constructor_types(
    module: &nir::NirModule,
    type_model: &TypeModel,
) -> Vec<(ParameterType, usize)> {
    struct Counter<'a> {
        counts: HashMap<ParameterType, usize>,
        arity: HashMap<ParameterType, usize>,
        type_model: &'a TypeModel,
    }
    impl nir::visit::NirVisitor for Counter<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            if let NirValue::Constructor { type_, args } = value {
                // Records only: the union-variant arm is a different shape (a tag
                // plus a fixed-size payload) and is 15,352 instructions in total.
                // `vector_field_count` types never reach the general arm at all —
                // they lower register-native — so they must not be counted here.
                if self.type_model.record_fields.contains_key(type_)
                    && crate::codegen::builtins::vector::vector_field_count(type_).is_none()
                {
                    *self.counts.entry(type_.clone()).or_insert(0) += 1;
                    self.arity.insert(type_.clone(), args.len());
                }
            }
            nir::visit::walk_value(self, value);
        }
    }
    let mut counter = Counter {
        counts: HashMap::new(),
        arity: HashMap::new(),
        type_model,
    };
    for function in &module.functions {
        nir::visit::NirVisitor::visit_ops(&mut counter, &function.body);
    }
    let mut qualified: Vec<(ParameterType, usize)> = counter
        .counts
        .iter()
        .filter(|(_, count)| **count >= CONSTRUCT_SITE_THRESHOLD)
        .filter_map(|(type_, _)| {
            counter
                .arity
                .get(type_)
                .map(|arity| (type_.clone(), *arity))
        })
        .collect();
    qualified.sort_by(|left, right| left.0.name().cmp(&right.0.name()));
    qualified
}

/// Synthesize `construct.T` — allocate the block, inline the `String` fields,
/// store the rest, and return it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_construct_helper<'a>(
    type_: &ParameterType,
    arity: usize,
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
    let symbol = construct_symbol(type_);
    let mut builder = CodeBuilder::for_synthetic_function(
        &symbol,
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
    // Park every argument in a slot exactly as the inline arm does, so
    // `emit_build_inlined_record` sees the shape it always has. The register
    // arguments must be read before anything allocates.
    let mut arg_slots = Vec::with_capacity(arity);
    for index in 0..arity {
        let slot = builder.allocate_stack_object("constructor_arg", 8);
        let scratch = builder.allocate_register();
        if index < abi::REGISTER_ARGUMENT_COUNT {
            builder.emit(abi::move_register(&scratch, abi::argument_register(index)?));
        } else {
            builder.emit(abi::incoming_stack_arg_load(
                &scratch,
                index - abi::REGISTER_ARGUMENT_COUNT,
            ));
        }
        builder.emit(abi::store_u64(&scratch, abi::stack_pointer(), slot));
        arg_slots.push(slot);
    }
    let register = builder.emit_build_inlined_record(type_, &arg_slots)?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &register));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

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
        name: format!("construct.{}", type_.name()),
        symbol,
        params: Vec::new(),
        returns: type_.name().into_owned(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

impl CodeBuilder<'_> {
    /// Marshal the already-lowered argument slots, `bl construct.T`, check.
    pub(crate) fn emit_construct_helper_call(
        &mut self,
        type_: &ParameterType,
        arg_slots: &[usize],
    ) -> Result<VirtualRegister, String> {
        let symbol = construct_symbol(type_);
        let ok = self.label("construct_ok");
        let scratch = self.allocate_register();
        // Stack tail first, so `x0`–`x7` are set last and nothing clobbers them
        // (bug-08; same order `emit_prepared_call_args` uses).
        for (index, slot) in arg_slots.iter().enumerate() {
            if index < abi::REGISTER_ARGUMENT_COUNT {
                continue;
            }
            self.emit(abi::load_u64(&scratch, abi::stack_pointer(), *slot));
            self.emit(abi::outgoing_stack_arg_store(
                &scratch,
                index - abi::REGISTER_ARGUMENT_COUNT,
            ));
        }
        for (index, slot) in arg_slots.iter().enumerate() {
            if index >= abi::REGISTER_ARGUMENT_COUNT {
                continue;
            }
            self.emit(abi::load_u64(&scratch, abi::stack_pointer(), *slot));
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                &scratch,
            ));
        }
        self.emit(abi::branch_link(&symbol));
        self.push_internal_call_relocation(&symbol);
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&ok));
        self.raise_error_bare("ErrOutOfMemory")?;
        self.emit(abi::label(&ok));
        let result = self.allocate_register();
        self.emit(abi::move_register(&result, abi::mfb_return(1)));
        Ok(result)
    }
}

/// The set form of [`synthesized_constructor_types`], for the builder field.
pub(crate) fn synthesized_constructor_set(
    types: &[(ParameterType, usize)],
) -> HashSet<ParameterType> {
    types.iter().map(|(type_, _)| type_.clone()).collect()
}
