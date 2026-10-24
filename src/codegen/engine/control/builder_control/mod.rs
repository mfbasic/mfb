// --- codegen tier imports (migration) ---
use crate::codegen::collection::assign::inplace_dest::{InPlaceDest, WriteBack};
use crate::codegen::collection::assign::self_update::{
    field_owner_is, field_place_is, is_global_self_update_call, is_self_update_call,
    FieldContainer, FieldLevel, FieldSite, SelfUpdateSite,
};
use crate::codegen::collection::layout::*;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::function::*;
use crate::codegen::engine::operand::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::value::store_reach::{global_root, local_root, StoreLeaf};
use crate::codegen::error::constants::*;
use crate::operators::BinaryOp;
use crate::target::shared::abi;
use crate::target::shared::nir::*;
use crate::types::ParameterType;

mod assign_op;
mod bind_op;
mod element_update;
mod field_replace;
mod field_with;
mod for_each;
mod loops;
mod statement_dispatch;
mod store_global;
mod string_shadow;
mod value_effects;
/// bug-567: what becomes of the pending temporaries a statement registered, once
/// that statement is done.
///
/// This is the whole vocabulary, and it is closed by the compiler:
/// [`CodeBuilder::transfer_temp_disposition`] matches every `NirOp` variant with
/// **no wildcard arm**, so a new statement kind cannot be added without choosing a
/// class here — it is a build error, not a silent inheritance of the previous
/// default. `the_transfer_temp_classifier_has_no_wildcard_arm` keeps the wildcard
/// out.
///
/// The default is what bug-567 was: every control transfer truncated every temp,
/// so `RETURN "v" & toString(n MOD 10)` dropped `toString`'s block unfreed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TransferTemps {
    /// Not a control transfer. The statement falls through, so its own temps are
    /// freed right there by `drop_pending_temps_to` — the ordinary case, and the
    /// one whose soundness argument every arm below borrows.
    StatementScope,
    /// The statement branches away, and its exit emitter frees the interior temps
    /// **itself, before the branch**, handing the escaping value out around them
    /// (`drop_interior_temps_before_branch`). Reaching the end of the statement
    /// with temps still above the watermark means the emitter missed a shape, and
    /// `forget_pending_temps_to` reports that as a codegen error instead of
    /// truncating it into a leak.
    FreedBeforeBranch,
    /// The statement lowers no expression at all, so it can register no temp:
    /// `EXIT`/`CONTINUE` carry only a `LoopKind`. Enforced, not assumed — if one
    /// ever grows an operand this reds rather than leaking it.
    NoExpression,
    /// The temp is **adopted** by whoever catches the error:
    /// `emit_direct_error_return` parks the `Error` block in the per-thread
    /// current-error slot *precisely because* the `Fail`'s control transfer
    /// FORGETS it, and the catcher then frees it exactly once (design "b").
    /// Freeing it here is the double free that design exists to avoid, so this
    /// class must keep truncating — and that is the only reason `RETURN` and
    /// `FAIL` cannot share one answer.
    ///
    /// bug-567's report predicted a residual interior leak here, and MEASUREMENT
    /// says there is none: `error(...)`'s own constructor lowering already frees
    /// the message temps before the branch. `FAIL error(7, repeat("x",4000) & "y")`
    /// and `FAIL error(7, repeat("x",4000))` — the second has no interior temp at
    /// all — read 626 MB at 20 000 trapped errors and 1251 MB at 40 000, the same
    /// to within 16 KB, where an abandoned 4 KB interior block would have shown as
    /// +80 MB. (Both grow: that growth is bug-565's open error path, not this.)
    /// `a_failing_trap_does_not_leak_its_interior_temp` pins the equality.
    AdoptedByTheCatcher,
    /// The process ends at this statement. Any abandoned temp is an O(1) leak in a
    /// heap that is about to be torn down whole, and the escaping value is the
    /// exit code — a scalar with no block.
    ProcessTerminates,
}

/// plan-145-F: how `try_inplace_scalar_fields` writes one updated field.
enum FieldStoreKind {
    /// A scalar, stored in its slot.
    Scalar,
    /// A pointer field (plan-145-C): the new owned pointee is stored, the old one
    /// dropped.
    Pointer(ParameterType),
    /// A fixed-size inlined record: its bytes are copied over the sub-block.
    Overwrite(ParameterType),
    /// bug-766 D: a variable-width inlined `String` / record that is the owner's
    /// LAST inlined sub-block (`replace_field_admitted`). Its new bytes go over
    /// the old span when the owner's walked size rounds to the same 16-byte
    /// granule as before; otherwise the owner is rebuilt at its new walked size.
    /// bug-784: over an element-bound owner, any variable-width inlined field
    /// (`element_replace_admitted`), in place only when its exact size is
    /// unchanged; otherwise the element's write-back runs as written.
    Replace(ParameterType),
    /// bug-784: `inner := WITH <owner>.inner { <scalars> }` on an inlined record
    /// field — each scalar is stored into the field's sub-block, which keeps its
    /// size, so no field moves and no record is built.
    Nested(Vec<(usize, NirValue)>),
}

/// bug-784: where one update's new value was spilled, for an element's
/// size-change branch to read back. (`pub(crate)`, like [`FieldNested`] and
/// [`FieldReplace`], for `engine/tests/element_replace_refusals.rs`.)
pub(crate) enum SpilledValue {
    /// The field's value, in one slot.
    One(usize),
    /// A [`FieldStoreKind::Nested`] update's scalars, one slot each in the inner
    /// `WITH`'s order.
    Nested(Vec<usize>),
}

/// bug-784: a [`FieldStoreKind::Nested`] update's spilled values.
pub(crate) struct FieldNested {
    /// The inlined record field's index in the owner record.
    index: usize,
    /// `(sub-field index, value slot)`, in source order.
    stores: Vec<(usize, usize)>,
}

/// bug-766 D: the replaced last sub-block's new value, spilled before any store.
pub(crate) struct FieldReplace {
    /// The field's index in the owner record.
    pub(crate) index: usize,
    /// The new value's block pointer (a `String` block or a flat record).
    pub(crate) slot: usize,
    /// The new value's byte size (`len + 9`, or the record's walked size).
    pub(crate) size_slot: usize,
}

/// plan-145-F: a fixed-size inlined field's new value, spilled before any store.
enum FieldOverwrite {
    /// A register-native vector's lanes, one slot each, in field order.
    Lanes(Vec<usize>),
    /// A block and its byte size; `owned` when this store frees it after the copy
    /// (a fresh value, or a snapshot of a borrowed one).
    Block {
        slot: usize,
        size_slot: usize,
        owned: bool,
    },
}

impl CodeBuilder<'_> {
    /// Classify `op` for [`TransferTemps`]. **Exhaustive on purpose** — see that
    /// type. Every arm is a decision about ownership; none may become a wildcard.
    fn transfer_temp_disposition(op: &NirOp) -> TransferTemps {
        match op {
            NirOp::Return { .. } => TransferTemps::FreedBeforeBranch,
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => TransferTemps::NoExpression,
            NirOp::Fail { .. } => TransferTemps::AdoptedByTheCatcher,
            NirOp::ExitProgram { .. } => TransferTemps::ProcessTerminates,
            NirOp::Bind { .. }
            | NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Eval { .. }
            | NirOp::If { .. }
            | NirOp::Match { .. }
            | NirOp::While { .. }
            | NirOp::For { .. }
            | NirOp::DoUntil { .. }
            | NirOp::ForEach { .. }
            | NirOp::Trap { .. } => TransferTemps::StatementScope,
        }
    }

    /// Retire the pending temporaries a control-transfer statement registered.
    ///
    /// The two classes that claim their temps are already gone — `FreedBeforeBranch`
    /// froze them before branching, `NoExpression` never made any — are *checked*,
    /// not trusted. Anything left is a shape the exit emitter did not recognise,
    /// and an unrecognised ownership shape is exactly what must not fall back to
    /// the old truncation: that is the leak this bug was.
    pub(crate) fn forget_pending_temps_to(
        &mut self,
        watermark: usize,
        disposition: TransferTemps,
    ) -> Result<(), String> {
        if matches!(
            disposition,
            TransferTemps::FreedBeforeBranch | TransferTemps::NoExpression
        ) && self.pending_temp_frees.len() > watermark
        {
            return Err(format!(
                "native code control transfer classified {disposition:?} left {} \
                 pending temporaries unfreed; its exit emitter must free them \
                 before it branches (bug-567)",
                self.pending_temp_frees.len() - watermark
            ));
        }
        self.clear_pending_temps_to(watermark);
        Ok(())
    }

    pub(crate) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        let cleanup_scope_start = self.active_cleanups.len();
        self.cleanup_scope_starts.push(cleanup_scope_start);
        let result = self.lower_ops_inner(ops, cleanup_scope_start);
        self.cleanup_scope_starts.pop();
        result
    }
}

/// If `value` is a left-associated string-concat chain whose leftmost leaf
/// satisfies `is_self` — the local, or a global (plan-142-H) — return the operands
/// to append in source order (`[a, b, …]`); otherwise `None`. This is the in-place
/// self-append idiom `name = name & …` (plan-02 §4.1); `&` is string
/// concatenation, so a match guarantees the binding is a `String`.
pub(crate) fn string_self_append_operands_of<'v>(
    value: &'v NirValue,
    is_self: &dyn Fn(&NirValue) -> bool,
) -> Option<Vec<&'v NirValue>> {
    let NirValue::Binary {
        op, left, right, ..
    } = value
    else {
        return None;
    };
    if *op != BinaryOp::Concat {
        return None;
    }
    let mut operands = vec![right.as_ref()];
    let mut cursor = left.as_ref();
    loop {
        match cursor {
            root if is_self(root) => {
                operands.reverse();
                return Some(operands);
            }
            NirValue::Binary {
                op, left, right, ..
            } if *op == BinaryOp::Concat => {
                operands.push(right.as_ref());
                cursor = left.as_ref();
            }
            _ => return None,
        }
    }
}

/// True when `name` is read anywhere within `value`. Used to reject the in-place
/// string self-append fast path when the target reappears as (or inside) a later
/// operand of the concat chain (`s = s & x & s`): the operands are lowered one at
/// a time *after* earlier ones have mutated the buffer, so a later read of `name`
/// would see the already-extended value (bug-143). A `LocalRef` reference to the
/// same slot is equally hazardous.
pub(crate) fn nir_value_reads_local(value: &NirValue, name: &str) -> bool {
    match value {
        NirValue::Local(local) | NirValue::LocalRef { name: local, .. } => local == name,
        NirValue::Const { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. }
        | NirValue::Capture { .. } => false,
        NirValue::Closure { captures, .. } => {
            captures.iter().any(|v| nir_value_reads_local(v, name))
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args.iter().any(|v| nir_value_reads_local(v, name)),
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. }
        | NirValue::Unary { operand: value, .. } => nir_value_reads_local(value, name),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            nir_value_reads_local(target, name)
                || updates
                    .iter()
                    .any(|u| nir_value_reads_local(&u.value, name))
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            values.iter().any(|v| nir_value_reads_local(v, name))
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| nir_value_reads_local(k, name) || nir_value_reads_local(v, name)),
        NirValue::MemberAccess { target, .. } => nir_value_reads_local(target, name),
        NirValue::Binary { left, right, .. } => {
            nir_value_reads_local(left, name) || nir_value_reads_local(right, name)
        }
    }
}

pub(crate) fn nir_op_context(op: &NirOp) -> String {
    match op {
        NirOp::Bind { name, type_, .. } => format!("bind {name} AS {type_}"),
        NirOp::StoreGlobal { name, .. } => format!("store global {name}"),
        NirOp::Assign { name, .. } => format!("assign {name}"),
        NirOp::StateAssign { resource, .. } => format!("state assign {resource}"),
        NirOp::Return { .. } => "return".to_string(),
        NirOp::ExitLoop { .. } => "exit loop".to_string(),
        NirOp::ContinueLoop { .. } => "continue loop".to_string(),
        NirOp::ExitProgram { .. } => "exit program".to_string(),
        NirOp::Fail { .. } => "fail".to_string(),
        NirOp::Eval { value } => format!("eval {}", nir_value_context(value)),
        NirOp::If { .. } => "if".to_string(),
        NirOp::Match { .. } => "match".to_string(),
        NirOp::While { .. } => "while".to_string(),
        NirOp::For { name, .. } => format!("for {name}"),
        NirOp::DoUntil { .. } => "do until".to_string(),
        NirOp::ForEach { name, .. } => format!("for each {name}"),
        NirOp::Trap { .. } => "trap".to_string(),
    }
}

pub(crate) fn nir_value_context(value: &NirValue) -> String {
    match value {
        NirValue::Call { target, .. }
        | NirValue::CallResult { target, .. }
        | NirValue::RuntimeCall { target, .. } => format!("call {target}"),
        NirValue::Constructor { type_, .. } => format!("construct {type_}"),
        NirValue::MemberAccess { member, .. } => format!("member {member}"),
        NirValue::Local(name) => format!("local {name}"),
        NirValue::LocalRef { name, .. } => format!("local ref {name}"),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::FunctionRef { name, .. } => format!("function {name}"),
        NirValue::Closure { name, .. } => format!("closure {name}"),
        NirValue::Const { type_, .. } => format!("const {type_}"),
        NirValue::ListLiteral { type_, .. }
        | NirValue::SetLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => {
            format!("literal {type_}")
        }
        NirValue::Unary { op, .. } => format!("operator {}", op.name()),
        NirValue::Binary { op, .. } => format!("operator {}", op.name()),
        NirValue::UnionWrap {
            union_type,
            member_type,
            ..
        } => format!("wrap {member_type} AS {union_type}"),
        NirValue::UnionExtract { type_, .. } => format!("extract {type_}"),
        NirValue::ResultIsOk { .. } => "result is ok".to_string(),
        NirValue::ResultValue { .. } => "result value".to_string(),
        NirValue::ResultError { .. } => "result error".to_string(),
        NirValue::Checked { type_, .. } => format!("checked {type_}"),
        NirValue::WithUpdate { type_, .. } => format!("with update {type_}"),
        NirValue::Capture { index, .. } => format!("capture {index}"),
    }
}

/// bug-778: whether `name` is one of the inline-`TRAP` desugar's delivery slots
/// (`ir::lower::lower_inline_trap`): `$trap_valN`, which carries the value on the
/// `Ok` and `RECOVER` paths, or `$trap_errN`, a check chain's shared error. Both
/// are declared without an initializer and assigned before every read. The `$`
/// cannot begin a source identifier, so only the desugar mints these names.
pub(crate) fn is_inline_trap_delivery_slot(name: &str) -> bool {
    ["$trap_val", "$trap_err"].iter().any(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    })
}
