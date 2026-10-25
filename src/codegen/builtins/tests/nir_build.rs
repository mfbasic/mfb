//! Builders for hand-written NIR, shared by the optimizer-row suites in this
//! directory (`opt_*.rs`). A row's contract is stated over the NIR it is
//! handed, so its tests build exactly the shape the row must accept or
//! decline rather than hoping a whole program lowers to it.

use std::collections::HashMap;

use crate::operators::BinaryOp;
use crate::target::shared::nir::{NirFunction, NirModule, NirOp, NirSourceLoc, NirValue};
use crate::types::ParameterType;

pub(super) fn int_const(value: &str) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Integer,
        value: value.to_string(),
    }
}

pub(super) fn bool_const(value: bool) -> NirValue {
    NirValue::Const {
        type_: ParameterType::Boolean,
        value: value.to_string(),
    }
}

pub(super) fn local(name: &str) -> NirValue {
    NirValue::Local(name.to_string())
}

pub(super) fn binary(op: BinaryOp, left: NirValue, right: NirValue) -> NirValue {
    NirValue::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
        loc: NirSourceLoc::default(),
    }
}

pub(super) fn bind(name: &str, value: NirValue) -> NirOp {
    NirOp::Bind {
        mutable: false,
        name: name.to_string(),
        type_: ParameterType::Integer,
        value: Some(value),
    }
}

pub(super) fn assign(name: &str, value: NirValue) -> NirOp {
    NirOp::Assign {
        name: name.to_string(),
        value,
    }
}

/// A `FOR <var> = <start> TO <end> STEP <step>` over Integer.
pub(super) fn for_loop(
    var: &str,
    start: NirValue,
    end: NirValue,
    step: NirValue,
    body: Vec<NirOp>,
) -> NirOp {
    NirOp::For {
        name: var.to_string(),
        type_: ParameterType::Integer,
        start,
        end,
        step,
        body,
        loc: Default::default(),
    }
}

pub(super) fn function(name: &str, body: Vec<NirOp>) -> NirFunction {
    NirFunction {
        name: name.to_string(),
        visibility: "private".to_string(),
        kind: "function".to_string(),
        isolated: false,
        params: vec![],
        returns: ParameterType::Integer,
        body,
        file: "main.mfb".to_string(),
        resource_owners: HashMap::new(),
    }
}

/// A module holding `functions`, with every other table empty.
pub(super) fn module(functions: Vec<NirFunction>) -> NirModule {
    NirModule {
        target: "linux-x86_64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        stdin_log_cap: 0,
        debug: crate::codegen::debug::DebugOptions::OFF,
        project: "test".to_string(),
        entry: None,
        globals: vec![],
        types: vec![],
        imports: vec![],
        runtime_helpers: vec![],
        functions,
        link_functions: vec![],
        link_cstructs: vec![],
        native_resources: vec![],
        native_libraries: crate::binary_repr::NativeLibraryTable { entries: vec![] },
        max_buffer_bytes: 0,
        canvas_auto_paint_ms: crate::manifest::DEFAULT_CANVAS_AUTO_PAINT_MS,
        arena_memory: crate::ir::ArenaMemoryLimits::default(),
    }
}

/// Run `row` over a one-function module holding `body` at dial `level`,
/// returning the rewritten body.
pub(super) fn run_row(level: u8, body: Vec<NirOp>, row: impl FnOnce(&mut NirModule)) -> Vec<NirOp> {
    let mut module = module(vec![function("f", body)]);
    crate::optimizer::with_opt_level(crate::optimizer::OptLevel(level), || row(&mut module));
    module.functions.remove(0).body
}
