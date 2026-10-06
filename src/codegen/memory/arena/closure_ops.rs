//! bug-781: single-owner closures — the copy and drop of a function value.
//!
//! A function value is a pointer to a 16-byte `{code, env}` object (`mfb spec memory
//! closures`). Every holder owns its own: copying a function value into a second holder
//! copies the object and deep-copies its environment, and dropping a holder frees both.
//! A `FUNC(Float) AS Float` type does not say what a closure captured, so neither can be
//! generated from the type at the holder. Instead every environment describes itself:
//! word 0 is the address of the creating `LAMBDA`'s environment routine, emitted here
//! from the layout the creating function recorded (`ClosureEnvLayout`), and the routine
//! copies or drops an environment of exactly that layout.
//!
//! * `_mfb_rt_closure_drop(object)` — nothing for a null word or a static descriptor
//!   (`env = 0`: a `FunctionRef`, which allocates nothing); otherwise the environment
//!   routine's drop, then the object.
//! * `_mfb_rt_closure_copy(object) -> object` — the same pointer for a null word or a
//!   static descriptor; otherwise a fresh object with the same `code` over the routine's
//!   copy of the environment.
//! * `_mfb_rt_closure_env_<lambda>(op, env) -> env` — `CLOSURE_ENV_OP_DROP` drops every
//!   owned capture (`emit_owned_value_drop`, the drop any owner of that type runs) and
//!   frees the block; `CLOSURE_ENV_OP_COPY` allocates a block of the same size, copies
//!   the header and every plain word verbatim, and deep-copies every owned capture
//!   (`copy_value_to_current_arena`, the copy any owning store of that type makes).
//!
//! The auxiliary words a self-updating `LAMBDA` carries (frame addresses of its
//! creator) are plain: such a closure is only ever a direct `collections::forEach`
//! action, never stored, returned or copied (bug-781, *Resolved: auxiliary environment
//! words never reach an escaping closure*), and its drop frees only the block.

use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::arena::graph_copy::finish_helper;
use crate::target::shared::abi;
use crate::types::ParameterType;
use std::collections::HashMap;

/// The tables every synthetic builder is made from.
pub(crate) struct ClosureHelperTables<'a> {
    pub(crate) function_symbols: &'a HashMap<String, String>,
    pub(crate) functions: &'a HashMap<String, &'a crate::target::shared::nir::NirFunction>,
    pub(crate) package_return_types: &'a HashMap<String, ParameterType>,
    pub(crate) platform_imports: &'a HashMap<String, String>,
    pub(crate) platform: &'a dyn crate::codegen::engine::types::CodegenPlatform,
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) globals: &'a HashMap<String, GlobalValue>,
    pub(crate) string_symbols: &'a HashMap<String, String>,
}

impl<'a> ClosureHelperTables<'a> {
    fn builder(&self, symbol: &str, type_model: TypeModel) -> CodeBuilder<'a> {
        CodeBuilder::for_synthetic_function(
            symbol,
            self.function_symbols,
            self.functions,
            self.package_return_types,
            self.platform_imports,
            self.platform,
            self.build_mode,
            self.globals,
            self.string_symbols,
            type_model,
        )
    }
}

/// Call the environment routine named by word 0 of the environment in `env_slot`, with
/// `op` in argument 0 and the environment in argument 1. Its result (a copy's new
/// environment) is left in the return register.
fn emit_env_routine_call(builder: &mut CodeBuilder<'_>, env_slot: usize, op: usize) {
    let sp = abi::stack_pointer();
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &op.to_string(),
    ));
    builder.emit(abi::load_u64(abi::c_arg(1), sp, env_slot));
    let routine = builder.allocate_register();
    builder.emit(abi::load_u64(&routine, sp, env_slot));
    builder.emit(abi::load_u64(&routine, &routine, CLOSURE_ENV_OFFSET_OPS));
    builder.emit(abi::branch_link_register(&routine));
}

/// `_mfb_rt_closure_drop(object)`.
pub(crate) fn lower_closure_drop(
    tables: &ClosureHelperTables<'_>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = tables.builder(CLOSURE_DROP_SYMBOL, type_model);
    let sp = abi::stack_pointer();
    let object_slot = builder.allocate_stack_object("closure_drop_object", 8);
    let env_slot = builder.allocate_stack_object("closure_drop_env", 8);
    let done = builder.label("closure_drop_done");
    let object = builder.allocate_register();
    builder.emit(abi::move_register(&object, abi::c_arg(0)));
    builder.emit(abi::store_u64(&object, sp, object_slot));
    builder.emit(abi::compare_immediate(&object, "0"));
    builder.emit(abi::branch_eq(&done));
    let env = builder.allocate_register();
    builder.emit(abi::load_u64(&env, &object, CLOSURE_OFFSET_ENV));
    builder.emit(abi::store_u64(&env, sp, env_slot));
    // A static descriptor: shared by every reference to its function, never freed.
    builder.emit(abi::compare_immediate(&env, "0"));
    builder.emit(abi::branch_eq(&done));
    emit_env_routine_call(&mut builder, env_slot, CLOSURE_ENV_OP_DROP);
    builder.emit(abi::load_u64(abi::c_arg(0), sp, object_slot));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        &CLOSURE_OBJECT_SIZE.to_string(),
    ));
    builder.emit_arena_free_call();
    builder.emit(abi::label(&done));
    builder.emit(abi::return_());
    finish_helper(builder, "runtime.closureDrop", CLOSURE_DROP_SYMBOL)
}

/// `_mfb_rt_closure_copy(object) -> object`.
pub(crate) fn lower_closure_copy(
    tables: &ClosureHelperTables<'_>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut builder = tables.builder(CLOSURE_COPY_SYMBOL, type_model);
    let sp = abi::stack_pointer();
    let object_slot = builder.allocate_stack_object("closure_copy_object", 8);
    let env_slot = builder.allocate_stack_object("closure_copy_env", 8);
    let new_env_slot = builder.allocate_stack_object("closure_copy_new_env", 8);
    let result_slot = builder.allocate_stack_object("closure_copy_result", 8);
    let shared = builder.label("closure_copy_shared");
    let finish = builder.label("closure_copy_finish");
    let alloc_ok = builder.label("closure_copy_alloc_ok");
    let object = builder.allocate_register();
    builder.emit(abi::move_register(&object, abi::c_arg(0)));
    builder.emit(abi::store_u64(&object, sp, object_slot));
    builder.emit(abi::store_u64(&object, sp, result_slot));
    builder.emit(abi::compare_immediate(&object, "0"));
    builder.emit(abi::branch_eq(&shared));
    let env = builder.allocate_register();
    builder.emit(abi::load_u64(&env, &object, CLOSURE_OFFSET_ENV));
    builder.emit(abi::store_u64(&env, sp, env_slot));
    // A static descriptor is the one value of its function: every holder shares it.
    builder.emit(abi::compare_immediate(&env, "0"));
    builder.emit(abi::branch_eq(&shared));
    emit_env_routine_call(&mut builder, env_slot, CLOSURE_ENV_OP_COPY);
    builder.emit(abi::store_u64(abi::return_register(), sp, new_env_slot));
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &CLOSURE_OBJECT_SIZE.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(abi::mfb_return(1), sp, result_slot));
    let fresh = builder.allocate_register();
    let word = builder.allocate_register();
    let source = builder.allocate_register();
    builder.emit(abi::load_u64(&fresh, sp, result_slot));
    builder.emit(abi::load_u64(&source, sp, object_slot));
    builder.emit(abi::load_u64(&word, &source, CLOSURE_OFFSET_CODE));
    builder.emit(abi::store_u64(&word, &fresh, CLOSURE_OFFSET_CODE));
    builder.emit(abi::load_u64(&word, sp, new_env_slot));
    builder.emit(abi::store_u64(&word, &fresh, CLOSURE_OFFSET_ENV));
    builder.emit(abi::branch(&finish));
    builder.emit(abi::label(&shared));
    builder.emit(abi::label(&finish));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, sp, result_slot));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());
    finish_helper(builder, "runtime.closureCopy", CLOSURE_COPY_SYMBOL)
}

/// bug-843: the resource operations of an environment routine (see
/// `resource::closure_carry`). Arguments 2 and 3 are saved first; an operation past
/// copy and drop runs its walk over `layout.resource_words` and branches to `finish`,
/// and copy and drop fall through to the routine's own dispatch.
fn emit_resource_operations(
    builder: &mut CodeBuilder<'_>,
    layout: &ClosureEnvLayout,
    op_slot: usize,
    env_slot: usize,
    finish: &str,
) -> Result<(), String> {
    let sp = abi::stack_pointer();
    let first_slot = builder.allocate_stack_object("closure_env_arg2", 8);
    let second_slot = builder.allocate_stack_object("closure_env_arg3", 8);
    builder.emit(abi::store_u64(abi::c_arg(2), sp, first_slot));
    builder.emit(abi::store_u64(abi::c_arg(3), sp, second_slot));
    let ordinary = builder.label("closure_env_ordinary_op");
    let op = builder.allocate_register();
    builder.emit(abi::load_u64(&op, sp, op_slot));
    builder.emit(abi::compare_immediate(
        &op,
        &CLOSURE_ENV_OP_COPY.to_string(),
    ));
    builder.emit(abi::branch_ls(&ordinary));
    let module_float = builder.module_globals_reach_closures();
    let operations: [(usize, Vec<usize>, bool); 4] = [
        (CLOSURE_ENV_OP_RELEASE, vec![first_slot], true),
        (CLOSURE_ENV_OP_CLOSE_UNOWNED, Vec::new(), true),
        // Only a store into a global that can hold a function value runs it; a
        // module with no such global never calls it, and has no list to raise to.
        (CLOSURE_ENV_OP_MODULE_FLOAT, Vec::new(), module_float),
        (CLOSURE_ENV_OP_ADOPT, vec![first_slot, second_slot], true),
    ];
    for (code, args, emitted) in operations {
        if !emitted || layout.resource_words.is_empty() {
            continue;
        }
        let next = builder.label("closure_env_next_op");
        let op = builder.allocate_register();
        builder.emit(abi::load_u64(&op, sp, op_slot));
        builder.emit(abi::compare_immediate(&op, &code.to_string()));
        builder.emit(abi::branch_ne(&next));
        builder.emit_closure_env_resource_op(code, env_slot, &args, &layout.resource_words)?;
        builder.emit(abi::branch(finish));
        builder.emit(abi::label(&next));
    }
    builder.emit(abi::branch(finish));
    builder.emit(abi::label(&ordinary));
    Ok(())
}

/// The environment routine for one `LAMBDA`'s `layout`.
pub(crate) fn lower_closure_env_ops(
    tables: &ClosureHelperTables<'_>,
    type_model: TypeModel,
    layout: &ClosureEnvLayout,
) -> Result<CodeFunction, String> {
    let mut builder = tables.builder(&layout.symbol, type_model);
    let sp = abi::stack_pointer();
    let total_words = layout.words.len() + layout.auxiliary_words;
    let env_size = closure_env_word_offset(total_words);
    let op_slot = builder.allocate_stack_object("closure_env_op", 8);
    let env_slot = builder.allocate_stack_object("closure_env_source", 8);
    let result_slot = builder.allocate_stack_object("closure_env_result", 8);
    let copy = builder.label("closure_env_copy");
    let finish = builder.label("closure_env_finish");
    let alloc_ok = builder.label("closure_env_alloc_ok");
    let op = builder.allocate_register();
    let env = builder.allocate_register();
    builder.emit(abi::move_register(&op, abi::c_arg(0)));
    builder.emit(abi::move_register(&env, abi::c_arg(1)));
    builder.emit(abi::store_u64(&op, sp, op_slot));
    builder.emit(abi::store_u64(&env, sp, env_slot));
    builder.emit(abi::store_u64(abi::ZERO, sp, result_slot));
    // bug-843: in a module whose closures carry resources, every routine also
    // answers the four resource operations (`resource::closure_carry`), which walk
    // the captures that reach a resource. Any other module keeps the two-operation
    // routine exactly.
    let op = if builder.closures_carry_resources() {
        emit_resource_operations(&mut builder, layout, op_slot, env_slot, &finish)?;
        let reloaded = builder.allocate_register();
        builder.emit(abi::load_u64(&reloaded, sp, op_slot));
        reloaded
    } else {
        op
    };
    builder.emit(abi::compare_immediate(
        &op,
        &CLOSURE_ENV_OP_DROP.to_string(),
    ));
    builder.emit(abi::branch_ne(&copy));

    // Drop: every owned capture, as its own owner would drop it, then the block.
    for (index, word) in layout.words.iter().enumerate() {
        if *word == ClosureEnvWord::ResourceUnionBox {
            // bug-843: the environment's own copy of a resource-union box; the record
            // it names belongs to its own scope, so only the box is freed.
            let skip = builder.label("closure_env_union_box_absent");
            let source = builder.allocate_register();
            builder.emit(abi::load_u64(&source, sp, env_slot));
            builder.emit(abi::load_u64(
                &source,
                &source,
                closure_env_word_offset(index),
            ));
            builder.emit(abi::compare_immediate(&source, "0"));
            builder.emit(abi::branch_eq(&skip));
            builder.emit(abi::move_register(abi::c_arg(0), &source));
            builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "16"));
            builder.emit_arena_free_call();
            builder.emit(abi::label(&skip));
            continue;
        }
        let ClosureEnvWord::Owned(type_) = word else {
            continue;
        };
        let capture_slot = builder.allocate_stack_object("closure_env_capture", 8);
        let source = builder.allocate_register();
        builder.emit(abi::load_u64(&source, sp, env_slot));
        builder.emit(abi::load_u64(
            &source,
            &source,
            closure_env_word_offset(index),
        ));
        builder.emit(abi::store_u64(&source, sp, capture_slot));
        builder.emit_owned_value_drop(&OwnedValueCleanup {
            type_: type_.clone(),
            stack_offset: capture_slot,
            capacity_slot: None,
            loop_alias_slot: None,
            result_wrapper: None,
        })?;
    }
    builder.emit(abi::load_u64(abi::c_arg(0), sp, env_slot));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        &env_size.to_string(),
    ));
    builder.emit_arena_free_call();
    builder.emit(abi::branch(&finish));

    // Copy: a block of the same size; the header and plain words verbatim, every owned
    // capture deep-copied as an owning store of its type copies it.
    builder.emit(abi::label(&copy));
    builder.emit(abi::move_immediate(
        abi::c_arg(0),
        "Integer",
        &env_size.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    builder.emit(abi::store_u64(abi::mfb_return(1), sp, result_slot));
    for word_index in 0..=total_words {
        // Word 0 is the header (the routine address); capture `i` is word `i + 1`.
        let offset = word_index * 8;
        let word = word_index
            .checked_sub(CLOSURE_ENV_HEADER_WORDS)
            .and_then(|capture| layout.words.get(capture));
        let owned = word.and_then(|word| match word {
            ClosureEnvWord::Owned(type_) => Some(type_.clone()),
            ClosureEnvWord::Plain | ClosureEnvWord::ResourceUnionBox => None,
        });
        let value = builder.allocate_register();
        builder.emit(abi::load_u64(&value, sp, env_slot));
        builder.emit(abi::load_u64(&value, &value, offset));
        let stored = match owned {
            // bug-843: each environment owns its own copy of a captured union box.
            None if word == Some(&ClosureEnvWord::ResourceUnionBox) => {
                let box_slot = builder.spill_to_slot("closure_env_union_box", &value);
                builder.emit_copy_resource_union_box(box_slot)?
            }
            None => value,
            Some(type_) => {
                let copied_slot = builder.allocate_stack_object("closure_env_copied", 8);
                let absent = builder.label("closure_env_copy_absent");
                builder.emit(abi::store_u64(&value, sp, copied_slot));
                builder.emit(abi::compare_immediate(&value, "0"));
                builder.emit(abi::branch_eq(&absent));
                let copied = builder.copy_value_to_current_arena(&type_, &value)?;
                builder.emit(abi::store_u64(&copied, sp, copied_slot));
                builder.emit(abi::label(&absent));
                let reloaded = builder.allocate_register();
                builder.emit(abi::load_u64(&reloaded, sp, copied_slot));
                reloaded
            }
        };
        let destination = builder.allocate_register();
        builder.emit(abi::load_u64(&destination, sp, result_slot));
        builder.emit(abi::store_u64(&stored, &destination, offset));
    }
    builder.emit(abi::label(&finish));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, sp, result_slot));
    builder.emit(abi::move_register(abi::return_register(), &result));
    builder.emit(abi::return_());
    finish_helper(builder, "runtime.closureEnv", &layout.symbol)
}
