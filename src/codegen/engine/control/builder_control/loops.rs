//! plan-176-U: loop lowering — conditions, bodies, provable indexes, the numeric `FOR` and loop
//! promotion. The statement entry (`lower_ops`) and shared types live in
//! `engine/control/builder_control/mod.rs`.

use super::*;

impl CodeBuilder<'_> {
    #[allow(clippy::too_many_arguments)]
    /// plan-86 G1: resolve an expression to the list `L` whose `len(L)` it is —
    /// either a direct `len(L)` call (depth `None`: re-evaluated at every loop
    /// entry, so no back edge can stale it) or a local `n` bound as `LET n = len(L)`
    /// (depth `Some(d)`: the `enclosing_loop_reassigned` length when the fact was
    /// recorded — see that field for why it matters, bug-495).
    fn resolve_len_list(&self, expr: &NirValue) -> Option<(String, Option<usize>)> {
        match expr {
            NirValue::Call { target, args, .. } if target == "len" && args.len() == 1 => {
                match &args[0] {
                    NirValue::Local(l) => Some((l.clone(), None)),
                    _ => None,
                }
            }
            NirValue::Local(n) => self
                .len_of_local
                .get(n)
                .map(|(l, depth)| (l.clone(), Some(*depth))),
            _ => None,
        }
    }

    /// Branch to `target` when the Boolean `condition` is false (bug-774).
    ///
    /// When `condition` is the result register of the integer comparison emitter
    /// that was just lowered — the stream ends `cmp a, b; b.cc T; mov_imm R, false;
    /// b D; T: mov_imm R, true; D:` with `R == condition` — the Boolean is never
    /// materialized: the tail is cut back to the `cmp` and its branch becomes
    /// `b.!cc target`. The six signed integer conditions invert exactly (`cmp` sets
    /// no unordered state), and `T`/`D` are the emitter's own fresh labels, named
    /// nowhere else, so dropping them leaves no dangling reference. Anything else —
    /// a float or `Fixed` compare, a compare whose temps were freed after it, a
    /// Boolean local or call — takes the general `cmp_imm R, 0; b.eq target`.
    pub(crate) fn emit_branch_unless(
        &mut self,
        condition: &Operand,
        target: &str,
        reason: Option<&'static str>,
    ) {
        if let Some(inverse) = self.fusable_comparison_tail(condition) {
            let len = self.instructions.len();
            self.instructions.truncate(len - 5);
            let source = self.instructions[len - 6].source;
            let mut branch = CodeInstruction::new(inverse).field("target", target);
            if let Some(reason) = reason {
                branch = branch.field("reason", reason);
            }
            branch.source = source;
            self.instructions[len - 6] = branch;
            return;
        }
        self.emit(abi::compare_immediate(condition, "0"));
        let branch = abi::branch_eq(target);
        match reason {
            Some(reason) => self.emit(branch.field("reason", reason)),
            None => self.emit(branch),
        }
    }

    /// The inverted branch mnemonic when the stream ends in the integer
    /// comparison shape [`Self::emit_branch_unless`] fuses, with `condition` its
    /// result register; `None` otherwise.
    fn fusable_comparison_tail(&self, condition: &Operand) -> Option<&'static str> {
        use crate::arch::ops::CodeOp;
        let len = self.instructions.len();
        if len < 7 {
            return None;
        }
        let tail = &self.instructions[len - 7..];
        // No type check: the `cmp_true_`/`cmp_done_` labels below come only from
        // the comparison emitter, whose two writes are the Boolean `true`/`false`.
        let is_bool = |instruction: &CodeInstruction, value: &str| {
            instruction.op == CodeOp::MovImm
                && instruction.operand("dst") == Some(condition)
                && instruction.get("value").as_deref() == Some(value)
        };
        let label_named = |instruction: &CodeInstruction, prefix: &str| {
            (instruction.op == CodeOp::Label)
                .then(|| instruction.get("name"))
                .flatten()
                .filter(|name| name.starts_with(prefix))
        };
        if !matches!(tail[0].op, CodeOp::Cmp | CodeOp::CmpImm) {
            return None;
        }
        let inverse = match tail[1].op {
            CodeOp::BranchEq => "b.ne",
            CodeOp::BranchNe => "b.eq",
            CodeOp::BranchLt => "b.ge",
            CodeOp::BranchGe => "b.lt",
            CodeOp::BranchGt => "b.le",
            CodeOp::BranchLe => "b.gt",
            _ => return None,
        };
        let true_label = label_named(&tail[4], "cmp_true_")?;
        let done_label = label_named(&tail[6], "cmp_done_")?;
        let fits = tail[1].get("target").as_deref() == Some(true_label.as_str())
            && is_bool(&tail[2], "false")
            && tail[3].op == CodeOp::Branch
            && tail[3].get("target").as_deref() == Some(done_label.as_str())
            && is_bool(&tail[5], "true");
        fits.then_some(inverse)
    }

    /// Lower a loop body with its reassigned-locals set pushed on
    /// `enclosing_loop_reassigned` (bug-495), so an inner `FOR`'s provable-index
    /// proof can see every reassignment this loop's back edge may run before
    /// re-entering it. Every loop kind (`WHILE`, `FOR`, `DO … UNTIL`, `FOR EACH`)
    /// lowers its body through here and nowhere else.
    /// Lower a loop condition that runs once per pass, freeing the temps it made before
    /// branching on it (bug-621). The loop statement's own end drop runs once, after the loop,
    /// and each pass overwrites the previous pass's slot — so it freed only the last block.
    /// Freeing here, on the one edge every evaluation takes, leaves no temp for the body, the
    /// exit pass or that end drop. The value is spilled across the `arena_free` calls, which
    /// clobber every caller-saved register; nothing is emitted when the condition made no temp.
    pub(super) fn lower_loop_condition(&mut self, condition: &NirValue) -> Result<Operand, String> {
        let watermark = self.pending_temp_frees.len();
        let condition = self.lower_value(condition)?;
        if self.pending_temp_frees.len() <= watermark {
            return Ok(condition.location);
        }
        let slot = self.allocate_stack_object("loop_condition", 8);
        self.emit(abi::store_u64(
            &condition.location,
            abi::stack_pointer(),
            slot,
        ));
        self.drop_pending_temps_to(watermark)?;
        let reloaded = self.allocate_register();
        self.emit(abi::load_u64(&reloaded, abi::stack_pointer(), slot));
        Ok(Operand::from(reloaded))
    }

    pub(super) fn lower_loop_body(&mut self, body: &[NirOp]) -> Result<(), String> {
        self.enclosing_loop_reassigned.push(
            crate::codegen::engine::function::collect_reassigned_locals(body),
        );
        let result = self.lower_ops(body);
        self.enclosing_loop_reassigned.pop();
        result
    }

    /// plan-86 G1: recognize `FOR i = 0 TO len(L) - k` (`k >= 1`, step 1) where `i`
    /// and `L` are provably unmodified across the whole loop body. Returns
    /// `(L, k)` — then `get/set(L, i)` is in-range (`i <= len-k < len`), and
    /// `get/set(L, i+1)` too when `k >= 2`. Conservative: any deviation returns
    /// `None` (the bounds check is kept). SOUNDNESS rests on the whole-body
    /// no-reassign proof — an unsound elision is a silent OOB.
    /// plan-86 G1: resolve a synthetic `$for_end*`/`$for_step*` local back to its
    /// bound expr; identity for anything else.
    fn resolve_for_local<'a>(&'a self, v: &'a NirValue) -> &'a NirValue {
        if let NirValue::Local(name) = v {
            if let Some(expr) = self.for_bound_expr.get(name) {
                return expr;
            }
        }
        v
    }

    fn recognize_provable_index(
        &self,
        i: &str,
        start: &NirValue,
        end: &NirValue,
        step: &NirValue,
        body: &[NirOp],
    ) -> Option<(String, i64)> {
        let is_int_const = |v: &NirValue, want: &str| matches!(v, NirValue::Const { type_, value } if matches!(type_, ParameterType::Integer) && value == want);
        // The IR desugars the bound/step into synthetic locals — resolve them.
        let start = self.resolve_for_local(start);
        let step = self.resolve_for_local(step);
        let end = self.resolve_for_local(end);
        if !is_int_const(start, "0") || !is_int_const(step, "1") {
            return None;
        }
        // end must be `<lenexpr> - k`, k >= 1.
        let NirValue::Binary {
            op, left, right, ..
        } = end
        else {
            return None;
        };
        if *op != BinaryOp::Subtract {
            return None;
        }
        let NirValue::Const { type_, value } = right.as_ref() else {
            return None;
        };
        if !matches!(type_, ParameterType::Integer) {
            return None;
        }
        let k: i64 = value.parse().ok()?;
        if k < 1 {
            return None;
        }
        let (list, fact_depth) = self.resolve_len_list(left)?;
        // Soundness: `i` and `L` (and the `n` in `n - k`, since `n = len(L)` must
        // still hold) must not be reassigned anywhere in the body.
        let reassigned = crate::codegen::engine::function::collect_reassigned_locals(body);
        if reassigned.contains(i) || reassigned.contains(&list) {
            return None;
        }
        if let NirValue::Local(n) = left.as_ref() {
            if reassigned.contains(n) {
                return None;
            }
        }
        // bug-495: a `LET n = len(L)` fact recorded BEFORE an enclosing loop was
        // entered is re-used on every back edge of that loop without `n` being
        // rebound — so if that loop's body reassigns `L` (or `n`) anywhere, even
        // AFTER this `FOR` in program order, the second entry sees a stale `n`
        // against a shorter `L` and an unchecked `get` reads out of bounds. The
        // body-only proof above cannot see that; the enclosing sets can. Loops
        // entered before the fact was recorded (index `< depth`) contain the `LET`
        // and re-run it each iteration, so they cannot stale it.
        if let Some(depth) = fact_depth {
            let n = match left.as_ref() {
                NirValue::Local(n) => Some(n.as_str()),
                _ => None,
            };
            for enclosing in self.enclosing_loop_reassigned.iter().skip(depth) {
                if enclosing.contains(&list) || n.is_some_and(|n| enclosing.contains(n)) {
                    return None;
                }
            }
        }
        Some((list, k))
    }

    /// plan-86 G1: is `index_arg` a provably-in-range index into list `list_arg`?
    /// True when `list_arg == Local(L)` and `index_arg` is `Local(i)` (needs
    /// headroom `k >= 1`) or `i + 1` (needs `k >= 2`), with
    /// `provable_index_locals[i] == (L, k)`.
    /// Whether pre-lowered `index` provably indexes pre-lowered `list` in range, so a
    /// `get`/`set` may elide the bounds check (plan-86 G1). Reads the args' source
    /// `NirValue` off `ValueResult::origin` — the collection must be a bare local `L`,
    /// and the index a bare local `i` (`k >= 1`) or `i + 1` (`k >= 2`) — against
    /// `provable_index_locals` (set by the `FOR i = 0 TO len(L)-k` recognizer). The
    /// pre-lowered `ValueResult` carries the source node the self-lowering body matched.
    pub(crate) fn is_provable_index_access(
        &self,
        list_arg: &ValueResult,
        index_arg: &ValueResult,
    ) -> bool {
        let Some(NirValue::Local(list)) = &list_arg.origin else {
            return false;
        };
        let (i, need_k) = match &index_arg.origin {
            Some(NirValue::Local(i)) => (i.as_str(), 1i64),
            Some(NirValue::Binary {
                op, left, right, ..
            }) if *op == BinaryOp::Add => match (left.as_ref(), right.as_ref()) {
                (NirValue::Local(i), NirValue::Const { type_, value })
                    if matches!(type_, ParameterType::Integer) && value == "1" =>
                {
                    (i.as_str(), 2i64)
                }
                _ => return false,
            },
            _ => return false,
        };
        matches!(self.provable_index_locals.get(i), Some((l, k)) if l == list && *k >= need_k)
    }

    /// `value` converted to the numeric type `target` by arithmetic promotion
    /// (§4.1): checked into `Fixed`, exact-or-nearest into `Float`, a widening
    /// move from `Byte` to `Integer`. Identity when the types already agree.
    fn numeric_value_as(
        &mut self,
        value: ValueResult,
        target: &ParameterType,
    ) -> Result<ValueResult, String> {
        if value.type_ == *target {
            return Ok(value);
        }
        let converted = self.allocate_register();
        match (target, &value.type_) {
            (ParameterType::Fixed, _) => self.load_numeric_as_fixed(&converted, &value)?,
            (ParameterType::Float, _) => {
                self.load_numeric_as_double(abi::FP_SCRATCH[0], &value)?;
                self.emit(abi::float_move_x_from_d(&converted, abi::FP_SCRATCH[0]));
            }
            (ParameterType::Integer, ParameterType::Byte) => {
                self.emit(abi::move_register(&converted, &value.location))
            }
            (target, source) => {
                return Err(format!(
                    "native code cannot promote a '{source}' value to '{target}'"
                ));
            }
        }
        Ok(ValueResult {
            origin: None,
            type_: target.clone(),
            location: Operand::from(converted),
            text: value.text,
        })
    }

    pub(crate) fn lower_numeric_for(
        &mut self,
        name: &str,
        type_: &ParameterType,
        start: &NirValue,
        end: &NirValue,
        step: &NirValue,
        body: &[NirOp],
        loc: NirSourceLoc,
    ) -> Result<(), String> {
        let local_slot = self.allocate_stack_object(name, 8);
        let start_value = self.lower_value(start)?;
        // Observation boundary: a `Float` loop counter's initial value is a
        // named binding and must be finite (plan-17).
        self.observe_float(start, &start_value)?;
        // The counter takes the loop's promoted type, so a start of another
        // numeric type is converted into it as arithmetic promotion would be —
        // checked into `Fixed` — failing at the `FOR` (bug-816). Stored
        // unconverted, its bits were read as the counter's.
        let saved_loc = self.current_loc;
        self.current_loc = loc;
        let start_value = self.numeric_value_as(start_value, type_)?;
        self.current_loc = saved_loc;
        self.store_value_at(&start_value, abi::stack_pointer(), local_slot);
        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.clone(),
                stack_offset: local_slot,
                constant: None,
                by_ref: false,
            },
        );

        // plan-86 G1: if this is `FOR name = 0 TO len(L)-k` (step 1) with `name` and
        // `L` unmodified across the whole body, record that `name` provably indexes
        // `L` in-range for the body's `get`/`set(L, name[+1])` — cleared right after.
        let g1_provable = self.recognize_provable_index(name, start, end, step, body);
        if let Some((list, k)) = &g1_provable {
            self.provable_index_locals
                .insert(name.to_string(), (list.clone(), *k));
        }

        let promoted = self.begin_loop_promotion(body, Some(name))?;
        let loop_label = self.label("for_loop");
        let continue_label = self.label("for_continue");
        let end_label = self.label("for_end");
        self.emit(abi::label(&loop_label));
        let iter = NirValue::Local(name.to_string());
        let zero = NirValue::Const {
            type_: type_.clone(),
            value: "0".to_string(),
        };
        // The bound and step tests compare in the loop's type, but an optimized
        // `end`/`step` can arrive as the bound's original operand (a `Float` local
        // for a `Fixed` loop), and a comparison promotes like arithmetic: that
        // operand is converted into `Fixed`, which is checked (bug-816). A failure
        // there, like the increment's below, carries the loop's location.
        let cmp = loc;
        let condition = NirValue::Binary {
            op: BinaryOp::Or,
            left: Box::new(NirValue::Binary {
                op: BinaryOp::And,
                left: Box::new(NirValue::Binary {
                    op: BinaryOp::GreaterEqual,
                    left: Box::new(step.clone()),
                    right: Box::new(zero.clone()),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: BinaryOp::LessEqual,
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            right: Box::new(NirValue::Binary {
                op: BinaryOp::And,
                left: Box::new(NirValue::Binary {
                    op: BinaryOp::Less,
                    left: Box::new(step.clone()),
                    right: Box::new(zero),
                    loc: cmp,
                }),
                right: Box::new(NirValue::Binary {
                    op: BinaryOp::GreaterEqual,
                    left: Box::new(iter.clone()),
                    right: Box::new(end.clone()),
                    loc: cmp,
                }),
                loc: cmp,
            }),
            loc: cmp,
        };
        let condition = self.lower_loop_condition(&condition)?;
        self.emit(abi::compare_immediate(&condition, "0"));
        self.emit(abi::branch_eq(&end_label));
        self.clear_local_constants();
        self.loop_stack.push(LoopLabels {
            kind: crate::ast::LoopKind::For,
            name: Some(name.to_string()),
            continue_label: continue_label.clone(),
            exit_label: end_label.clone(),
            cleanup_depth: self.active_cleanups.len(),
            temp_depth: self.pending_temp_frees.len(),
        });
        self.lower_loop_body(body)?;
        // plan-86 G1: the provable-index fact is scoped to this loop's body only.
        if g1_provable.is_some() {
            self.provable_index_locals.remove(name);
        }
        self.loop_stack.pop();
        self.emit(abi::label(&continue_label));
        let increment_node = NirValue::Binary {
            op: BinaryOp::Add,
            left: Box::new(iter),
            right: Box::new(step.clone()),
            loc,
        };
        let increment = self.lower_value(&increment_node)?;
        // Observation boundary: the incremented `Float` counter is written back
        // to its named slot, so a non-finite step must trap (plan-17).
        self.observe_float(&increment_node, &increment)?;
        self.store_value_at(&increment, abi::stack_pointer(), local_slot);
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.end_loop_promotion(promoted)?;
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.clear_local_constants();
        Ok(())
    }

    /// plan-146-G: the lambdas `ops` create a closure for, in source order.
    pub(crate) fn closures_created_in(&self, ops: &[NirOp]) -> Vec<String> {
        use crate::target::shared::nir::visit::{walk_value, NirVisitor};
        struct Finder {
            names: Vec<String>,
        }
        impl NirVisitor for Finder {
            fn visit_value(&mut self, value: &NirValue) {
                if let NirValue::Closure { name, captures, .. } = value {
                    if !captures.is_empty() && !self.names.contains(name) {
                        self.names.push(name.clone());
                    }
                }
                walk_value(self, value);
            }
        }
        let mut finder = Finder { names: Vec::new() };
        finder.visit_ops(ops);
        finder.names
    }

    /// Begin loop-carried promotion of safe float-accumulator locals for a loop
    /// with body `body` (plan-03 Stage D part 2). Each promotable local's slot
    /// value is loaded into a fresh FP virtual register held for the loop, and
    /// its folded constant is cleared so loop reads use the register. Returns the
    /// promoted names for `end_loop_promotion`. Emit this *before* the loop
    /// header so the load runs once on entry.
    pub(crate) fn begin_loop_promotion(
        &mut self,
        body: &[NirOp],
        exclude: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut top_assigns = std::collections::HashSet::new();
        let mut excluded = std::collections::HashSet::new();
        scan_loop_locals(body, 0, &mut top_assigns, &mut excluded);
        // A numeric `FOR` induction variable is reassigned by the loop's own
        // increment (a slot store the lowering adds), so it is never promotable.
        if let Some(name) = exclude {
            excluded.insert(name.to_string());
        }
        let mut names: Vec<String> = top_assigns
            .into_iter()
            .filter(|name| !excluded.contains(name))
            .collect();
        names.sort();
        let mut promoted = Vec::new();
        for name in names {
            if self.address_taken_locals.contains(&name)
                || self.promoted_float_locals.contains_key(&name)
                || self.owner_containers.contains(&name)
            {
                continue;
            }
            let Some(local) = self.locals.get(&name) else {
                continue;
            };
            if !matches!(local.type_, ParameterType::Float) || local.by_ref {
                continue;
            }
            let stack_offset = local.stack_offset;
            let gpr = self.allocate_register();
            let d = self.allocate_fp_register();
            self.emit(abi::load_u64(&gpr, abi::stack_pointer(), stack_offset));
            self.emit(abi::float_move_d_from_x(&d, &gpr));
            if let Some(local) = self.locals.get_mut(&name) {
                local.constant = None;
            }
            self.promoted_float_locals.insert(name.clone(), d.into());
            promoted.push(name);
        }
        Ok(promoted)
    }

    /// Store each promoted local back to its slot and end its promotion. Emit
    /// this *after* the loop's exit label — every loop exit (normal, `EXIT`/
    /// break) reaches that label, and a `RETURN` inside the loop reads the
    /// register directly and leaves the function, so a single store-back covers
    /// all exits for a non-escaping local.
    pub(crate) fn end_loop_promotion(&mut self, promoted: Vec<String>) -> Result<(), String> {
        for name in promoted {
            let Some(d) = self.promoted_float_locals.remove(&name) else {
                continue;
            };
            let Some(stack_offset) = self.locals.get(&name).map(|local| local.stack_offset) else {
                continue;
            };
            let gpr = self.allocate_register();
            self.emit(abi::float_move_x_from_d(&gpr, &d));
            self.emit(abi::store_u64(&gpr, abi::stack_pointer(), stack_offset));
        }
        Ok(())
    }

    /// Update a promoted float local's resident register from an assignment
    /// result (an FP-resident result is a `d`-to-`d` move; a GPR result is moved
    /// in via `fmov d, x`).
    pub(crate) fn update_promoted_float(&mut self, d: impl Into<abi::Reg>, result: &ValueResult) {
        let d: abi::Reg = d.into();
        // A `d`-native result already lives in an FP register: move it `d`-to-`d`
        // (plan-01 float-dnative). Otherwise reuse a recorded FP resident, or
        // shuttle the GPR bits in (`fmov d, x`).
        if Self::float_is_dnative(result) {
            if result.location != d {
                self.emit(abi::float_move_d_from_d(d, &result.location));
            }
        } else if let Some(d_res) = self.float_residents.get(&result.location.render()).cloned() {
            if d_res != d {
                self.emit(abi::float_move_d_from_d(d, &d_res));
            }
        } else {
            self.emit(abi::float_move_d_from_x(d, &result.location));
        }
    }
}
