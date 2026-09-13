//! plan-134-C: the owning stores that read a place for the LAST time.
//!
//! `mfb spec language memory-semantics` §14.2: "If the value's type is copyable and the
//! binding is not used again, the compiler may move it." Letters D and E copy a recursive
//! value at every owning store; this analysis is what lets a store whose source is never
//! read again MOVE instead — the decoders' `acc = collections::append(acc, item)` loops
//! would otherwise deep-copy every element they build.
//!
//! **Places, not just locals.** The decoders store fields as often as locals:
//! `regex`'s parser appends `q.node` and then reads `q.nxt`, `q.groups` and `q.names`;
//! `json` binds `LET item = parsed.value` and then reads `parsed.index`. A whole-local
//! analysis sees `q` and `parsed` as live and moves nothing. So a place is a local (`x`)
//! or one field of a local (`x.f`): reading `x` reads every `x.f`, reading `x.f` reads
//! only that field, and binding or assigning `x` kills `x` and all its fields.
//!
//! **Backward liveness over the structured NIR.** Every op's live-out set is computed
//! once per function, before lowering. Loops iterate to a fixed point, `EXIT`/`CONTINUE`
//! flow to the matching loop's exit/condition, and a function-level `TRAP` handler is a
//! successor of every op (an error can reach it from anywhere), so a place its body
//! reads is live everywhere.
//!
//! **A site is `(op, place)`** where the op is a simple statement (`Bind`, `Assign`,
//! `StoreGlobal`, `StateAssign`, `Eval`, `Return`, `Fail`, `ExitProgram`), it reads the
//! place exactly once, and the place is not live after the op. Compound statements
//! (`IF`, loops, `MATCH`) produce no sites: a store in a loop condition is copied. The
//! op is identified by its address in the function body the lowering walks, which cannot
//! drift the way a counted index could; a desugar that lowers a synthesized op simply
//! finds no site and copies.
//!
//! **Fail closed.** A missed move is a copy; a wrong move is a shared graph that a later
//! free double-frees. So a place whose root local is any of these is never a site: a
//! parameter (the caller owns it), address-taken (`LocalRef`), captured by a closure, a
//! `FOR EACH` element or a `FOR` variable, bound from a `Capture` or `UnionExtract` (an
//! alias into another value), a borrow-`get` local, read inside a `UnionExtract` or a
//! borrow-`get` initializer (an alias of it may be live), a `TRAP` error name, a
//! `STATE` resource, a local of a resource-bearing type, or a name this function never
//! binds. The op match below has no wildcard, so a new `NirOp` is a build error here.

use crate::ast::LoopKind;
use crate::codegen::collection::layout::type_contains_resource;
use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::function::function_lowering::{
    collect_address_taken_locals, collect_borrow_get_locals,
};
use crate::target::shared::nir::visit::{walk_op, walk_value, NirVisitor};
use crate::target::shared::nir::{NirFunction, NirMatchPattern, NirOp, NirValue};
use std::collections::{HashMap, HashSet};

/// A place an op reads: a whole local, or one field of a local.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Place {
    Local(String),
    Field(String, String),
}

impl Place {
    fn root(&self) -> &str {
        match self {
            Place::Local(name) | Place::Field(name, _) => name,
        }
    }
}

type Places = HashSet<Place>;

/// The `(op, place)` pairs whose read is the place's last.
pub(crate) struct MoveSites {
    sites: HashSet<(usize, Place)>,
}

impl MoveSites {
    /// Whether `op` reads `place` for the last time, so a store of it may move.
    pub(crate) fn is_last_use(&self, op: &NirOp, place: &Place) -> bool {
        self.sites.contains(&(op_key(op), place.clone()))
    }
}

fn op_key(op: &NirOp) -> usize {
    op as *const NirOp as usize
}

/// Every place `value` reads, one entry per read.
fn value_reads(value: &NirValue, out: &mut Vec<Place>) {
    struct Reads<'a> {
        out: &'a mut Vec<Place>,
    }
    impl NirVisitor for Reads<'_> {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) | NirValue::LocalRef { name, .. } => {
                    self.out.push(Place::Local(name.clone()));
                }
                NirValue::MemberAccess { target, member } => {
                    if let NirValue::Local(name) = target.as_ref() {
                        self.out.push(Place::Field(name.clone(), member.clone()));
                    } else {
                        walk_value(self, value);
                    }
                }
                _ => walk_value(self, value),
            }
        }
    }
    Reads { out }.visit_value(value);
}

fn reads_of(value: &NirValue) -> Vec<Place> {
    let mut out = Vec::new();
    value_reads(value, &mut out);
    out
}

fn add_reads(live: &mut Places, value: &NirValue) {
    live.extend(reads_of(value));
}

/// Remove `name` and every field of it.
fn kill(live: &mut Places, name: &str) {
    live.retain(|place| place.root() != name);
}

/// Whether a read of `place` could be observed again through `live`.
fn place_live(live: &Places, place: &Place) -> bool {
    match place {
        Place::Local(name) => live.iter().any(|other| other.root() == name),
        Place::Field(name, _) => live.contains(&Place::Local(name.clone())) || live.contains(place),
    }
}

/// How many of `reads` read `place` (a whole-local read reads every field of it).
fn read_count(reads: &[Place], place: &Place) -> usize {
    match place {
        Place::Local(name) => reads.iter().filter(|read| read.root() == name).count(),
        Place::Field(name, _) => reads
            .iter()
            .filter(|read| *read == place || **read == Place::Local(name.clone()))
            .count(),
    }
}

struct LoopFrame {
    kind: LoopKind,
    exit: Places,
    continue_: Places,
}

struct Liveness<'f> {
    /// What any `TRAP` handler in the function reads: live after every op.
    trap_live: Places,
    /// Every place read anywhere — the fail-closed answer for a jump with no target.
    universe: Places,
    loops: Vec<LoopFrame>,
    /// Each op's live-out, keyed by address, with the op itself.
    after: HashMap<usize, (&'f NirOp, Places)>,
}

impl<'f> Liveness<'f> {
    fn ops_in(&mut self, ops: &'f [NirOp], out: Places) -> Places {
        let mut live = out;
        for op in ops.iter().rev() {
            let mut op_out = live;
            op_out.extend(self.trap_live.iter().cloned());
            live = self.op_in(op, op_out);
        }
        live
    }

    fn loop_target(&self, kind: LoopKind, exit: bool) -> Places {
        match self.loops.iter().rev().find(|frame| frame.kind == kind) {
            Some(frame) if exit => frame.exit.clone(),
            Some(frame) => frame.continue_.clone(),
            None => self.universe.clone(),
        }
    }

    /// A loop whose back edge reaches `header`: iterate `step` to a fixed point.
    fn fixed_point(
        &mut self,
        kind: LoopKind,
        exit: &Places,
        seed: Places,
        mut step: impl FnMut(&mut Self, &Places) -> Places,
    ) -> Places {
        let mut header = seed;
        loop {
            self.loops.push(LoopFrame {
                kind,
                exit: exit.clone(),
                continue_: header.clone(),
            });
            let next = step(self, &header);
            self.loops.pop();
            if next == header {
                return header;
            }
            header = next;
        }
    }

    fn op_in(&mut self, op: &'f NirOp, out: Places) -> Places {
        self.after.insert(op_key(op), (op, out.clone()));
        match op {
            NirOp::Bind { name, value, .. } => {
                let mut live = out;
                kill(&mut live, name);
                if let Some(value) = value {
                    add_reads(&mut live, value);
                }
                live
            }
            NirOp::StoreGlobal { value, .. } => {
                let mut live = out;
                if let Some(value) = value {
                    add_reads(&mut live, value);
                }
                live
            }
            NirOp::Assign { name, value } => {
                let mut live = out;
                kill(&mut live, name);
                add_reads(&mut live, value);
                live
            }
            NirOp::StateAssign { resource, value } => {
                let mut live = out;
                live.insert(Place::Local(resource.clone()));
                add_reads(&mut live, value);
                live
            }
            NirOp::Return { value } => {
                let mut live = self.trap_live.clone();
                if let Some(value) = value {
                    add_reads(&mut live, value);
                }
                live
            }
            NirOp::ExitLoop { kind } => self.loop_target(*kind, true),
            NirOp::ContinueLoop { kind } => self.loop_target(*kind, false),
            NirOp::ExitProgram { code } => reads_of(code).into_iter().collect(),
            NirOp::Fail { error } => {
                let mut live = self.trap_live.clone();
                add_reads(&mut live, error);
                live
            }
            NirOp::Eval { value } => {
                let mut live = out;
                add_reads(&mut live, value);
                live
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                let mut live = self.ops_in(then_body, out.clone());
                live.extend(self.ops_in(else_body, out));
                add_reads(&mut live, condition);
                live
            }
            NirOp::Match { value, cases } => {
                // A scrutinee no case matches falls through.
                let mut live = out.clone();
                for case in cases {
                    live.extend(self.ops_in(&case.body, out.clone()));
                    match &case.pattern {
                        NirMatchPattern::Else => {}
                        NirMatchPattern::Value(pattern) => add_reads(&mut live, pattern),
                        NirMatchPattern::OneOf(patterns) => {
                            for pattern in patterns {
                                add_reads(&mut live, pattern);
                            }
                        }
                    }
                    if let Some(guard) = &case.guard {
                        add_reads(&mut live, guard);
                    }
                }
                add_reads(&mut live, value);
                live
            }
            NirOp::While {
                kind,
                condition,
                body,
            } => {
                // Condition first; the body falls back to it.
                let mut seed = out.clone();
                add_reads(&mut seed, condition);
                self.fixed_point(*kind, &out, seed, |this, header| {
                    let mut next = out.clone();
                    add_reads(&mut next, condition);
                    next.extend(this.ops_in(body, header.clone()));
                    next
                })
            }
            NirOp::DoUntil { body, condition } => {
                // Body first, then the condition, which loops back to the body. The
                // fixed point is over the condition's live-in; the body's live-in
                // against it is the statement's.
                let mut seed = out.clone();
                add_reads(&mut seed, condition);
                let check = self.fixed_point(LoopKind::Do, &out, seed, |this, check| {
                    let mut next = out.clone();
                    add_reads(&mut next, condition);
                    next.extend(this.ops_in(body, check.clone()));
                    next
                });
                self.loops.push(LoopFrame {
                    kind: LoopKind::Do,
                    exit: out.clone(),
                    continue_: check.clone(),
                });
                let live = self.ops_in(body, check);
                self.loops.pop();
                live
            }
            NirOp::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                // `end` and `step` are re-read at every test and increment; the
                // variable is read by the increment.
                let mut seed = out.clone();
                add_reads(&mut seed, end);
                add_reads(&mut seed, step);
                seed.insert(Place::Local(name.clone()));
                let header = self.fixed_point(LoopKind::For, &out, seed.clone(), |this, header| {
                    let mut next = seed.clone();
                    next.extend(this.ops_in(body, header.clone()));
                    next
                });
                let mut live = header;
                kill(&mut live, name);
                add_reads(&mut live, start);
                add_reads(&mut live, end);
                add_reads(&mut live, step);
                live
            }
            NirOp::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                // The iterable is walked for the whole loop, so what it reads stays live
                // through the body.
                let mut seed = out.clone();
                add_reads(&mut seed, iterable);
                let header = self.fixed_point(LoopKind::For, &out, seed.clone(), |this, header| {
                    let mut next = seed.clone();
                    next.extend(this.ops_in(body, header.clone()));
                    next
                });
                let mut live = header;
                kill(&mut live, name);
                add_reads(&mut live, iterable);
                live
            }
            // Normal flow never enters a handler; what it reads is in `trap_live`.
            NirOp::Trap { .. } => out,
        }
    }
}

/// The roots no site may name (module doc, "Fail closed").
fn excluded_roots(function: &NirFunction, model: &TypeModel) -> HashSet<String> {
    struct Scan<'a> {
        model: &'a TypeModel,
        borrow_get: &'a HashSet<String>,
        bound: HashSet<String>,
        read: HashSet<String>,
        excluded: HashSet<String>,
    }
    impl Scan<'_> {
        fn exclude_reads(&mut self, value: &NirValue) {
            for place in reads_of(value) {
                self.excluded.insert(place.root().to_string());
            }
        }
    }
    impl NirVisitor for Scan<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name, type_, value, ..
                } => {
                    self.bound.insert(name.clone());
                    if type_contains_resource(self.model, type_) {
                        self.excluded.insert(name.clone());
                    }
                    if matches!(
                        value,
                        Some(NirValue::Capture { .. }) | Some(NirValue::UnionExtract { .. })
                    ) {
                        self.excluded.insert(name.clone());
                    }
                    if self.borrow_get.contains(name) {
                        if let Some(value) = value {
                            self.exclude_reads(value);
                        }
                    }
                }
                NirOp::ForEach { name, .. } | NirOp::For { name, .. } | NirOp::Trap { name, .. } => {
                    self.bound.insert(name.clone());
                    self.excluded.insert(name.clone());
                }
                NirOp::StateAssign { resource, .. } => {
                    self.excluded.insert(resource.clone());
                }
                _ => {}
            }
            walk_op(self, op);
        }

        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Closure { captures, .. } => {
                    for capture in captures {
                        self.exclude_reads(capture);
                    }
                }
                NirValue::UnionExtract { value: inner, .. } => self.exclude_reads(inner),
                NirValue::Local(name) | NirValue::LocalRef { name, .. } => {
                    self.read.insert(name.clone());
                }
                NirValue::MemberAccess { target, .. } => {
                    if let NirValue::Local(name) = target.as_ref() {
                        self.read.insert(name.clone());
                    }
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }

    let mut address_taken = HashSet::new();
    collect_address_taken_locals(&function.body, &mut address_taken);
    let borrow_get = collect_borrow_get_locals(&function.body, &address_taken);
    let mut scan = Scan {
        model,
        borrow_get: &borrow_get,
        bound: HashSet::new(),
        read: HashSet::new(),
        excluded: HashSet::new(),
    };
    scan.visit_ops(&function.body);
    let mut excluded = scan.excluded;
    excluded.extend(address_taken);
    excluded.extend(borrow_get.iter().cloned());
    excluded.extend(function.params.iter().map(|param| param.name.clone()));
    // A name this function never binds has an owner this analysis cannot see.
    excluded.extend(scan.read.difference(&scan.bound).cloned());
    excluded
}

/// The move sites of `function` (module doc).
pub(crate) fn collect_last_use_moves(function: &NirFunction, model: &TypeModel) -> MoveSites {
    let excluded = excluded_roots(function, model);

    // Every place read anywhere: the fail-closed live set for a jump with no target.
    struct AllReads {
        places: Places,
    }
    impl NirVisitor for AllReads {
        fn visit_value(&mut self, value: &NirValue) {
            self.places.extend(reads_of(value));
        }
    }
    let mut all = AllReads {
        places: Places::new(),
    };
    all.visit_ops(&function.body);

    let mut liveness = Liveness {
        trap_live: Places::new(),
        universe: all.places.clone(),
        loops: Vec::new(),
        after: HashMap::new(),
    };
    // A handler runs to the end of the function; what it reads, it reads after any op.
    let mut handler_live = Places::new();
    collect_trap_handlers(&function.body, &mut |body| {
        handler_live.extend(liveness.ops_in(body, Places::new()));
    });
    liveness.trap_live = handler_live;
    liveness.after.clear();
    liveness.ops_in(&function.body, Places::new());

    let mut sites = HashSet::new();
    for (key, (op, out)) in &liveness.after {
        let (values, killed): (Vec<&NirValue>, Option<&str>) = match op {
            NirOp::Bind { name, value, .. } => (value.iter().collect(), Some(name)),
            NirOp::Assign { name, value } => (vec![value], Some(name)),
            NirOp::StoreGlobal { value, .. } => (value.iter().collect(), None),
            NirOp::StateAssign { value, .. } => (vec![value], None),
            NirOp::Eval { value } => (vec![value], None),
            NirOp::Return { value } => (value.iter().collect(), None),
            NirOp::Fail { error } => (vec![error], None),
            NirOp::ExitProgram { code } => (vec![code], None),
            NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. }
            | NirOp::If { .. }
            | NirOp::Match { .. }
            | NirOp::While { .. }
            | NirOp::For { .. }
            | NirOp::DoUntil { .. }
            | NirOp::ForEach { .. }
            | NirOp::Trap { .. } => continue,
        };
        let reads: Vec<Place> = values.into_iter().flat_map(reads_of).collect();
        // What can still observe a place after this op reads it: what is live after the
        // op (minus the op's own rebinding, which names a NEW value), and the handlers.
        let mut after = match op {
            NirOp::Return { .. } | NirOp::Fail { .. } => Places::new(),
            NirOp::ExitProgram { .. } => Places::new(),
            _ => out.clone(),
        };
        if let Some(name) = killed {
            kill(&mut after, name);
        }
        after.extend(liveness.trap_live.iter().cloned());
        for place in &reads {
            if excluded.contains(place.root())
                || read_count(&reads, place) != 1
                || place_live(&after, place)
            {
                continue;
            }
            sites.insert((*key, place.clone()));
        }
    }
    MoveSites { sites }
}

/// Call `visit` with the body of every `TRAP` handler in `ops`, at any depth.
fn collect_trap_handlers<'f>(ops: &'f [NirOp], visit: &mut impl FnMut(&'f [NirOp])) {
    for op in ops {
        match op {
            NirOp::Trap { body, .. } => {
                visit(body);
                collect_trap_handlers(body, visit);
            }
            NirOp::If {
                then_body,
                else_body,
                ..
            } => {
                collect_trap_handlers(then_body, visit);
                collect_trap_handlers(else_body, visit);
            }
            NirOp::Match { cases, .. } => {
                for case in cases {
                    collect_trap_handlers(&case.body, visit);
                }
            }
            NirOp::While { body, .. }
            | NirOp::For { body, .. }
            | NirOp::DoUntil { body, .. }
            | NirOp::ForEach { body, .. } => collect_trap_handlers(body, visit),
            NirOp::Bind { .. }
            | NirOp::StoreGlobal { .. }
            | NirOp::Assign { .. }
            | NirOp::StateAssign { .. }
            | NirOp::Return { .. }
            | NirOp::ExitLoop { .. }
            | NirOp::ContinueLoop { .. }
            | NirOp::ExitProgram { .. }
            | NirOp::Fail { .. }
            | NirOp::Eval { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::NirModule;
    use crate::testutil::{nir_for_src, CodeTarget};
    use crate::types::ParameterType;

    fn lower(source: &str) -> NirModule {
        nir_for_src(
            source,
            CodeTarget::MacosAarch64,
            crate::target::NativeBuildMode::Console,
        )
        .unwrap_or_else(|error| panic!("the probe lowers to NIR: {error}"))
    }

    fn function<'m>(module: &'m NirModule, name: &str) -> &'m NirFunction {
        module
            .functions
            .iter()
            .find(|f| f.name == name || f.name.rsplit(|c| c == '.' || c == ':').next() == Some(name))
            .unwrap_or_else(|| {
                panic!(
                    "no function `{name}` in {:?}",
                    module.functions.iter().map(|f| f.name.as_str()).collect::<Vec<_>>()
                )
            })
    }

    fn find_ops<'f>(ops: &'f [NirOp], wanted: &dyn Fn(&NirOp) -> bool, out: &mut Vec<&'f NirOp>) {
        for op in ops {
            if wanted(op) {
                out.push(op);
            }
            match op {
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    find_ops(then_body, wanted, out);
                    find_ops(else_body, wanted, out);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        find_ops(&case.body, wanted, out);
                    }
                }
                NirOp::While { body, .. }
                | NirOp::For { body, .. }
                | NirOp::DoUntil { body, .. }
                | NirOp::ForEach { body, .. }
                | NirOp::Trap { body, .. } => find_ops(body, wanted, out),
                _ => {}
            }
        }
    }

    fn ops<'f>(function: &'f NirFunction, wanted: &dyn Fn(&NirOp) -> bool) -> Vec<&'f NirOp> {
        let mut out = Vec::new();
        find_ops(&function.body, wanted, &mut out);
        out
    }

    fn binds(name: &'static str) -> impl Fn(&NirOp) -> bool {
        move |op| matches!(op, NirOp::Bind { name: bound, .. } if bound == name)
    }

    fn is_append_call(value: &NirValue) -> bool {
        match value {
            NirValue::Call { target, .. }
            | NirValue::CallResult { target, .. }
            | NirValue::RuntimeCall { target, .. } => target.contains("append"),
            _ => false,
        }
    }

    fn appends_to(name: &'static str) -> impl Fn(&NirOp) -> bool {
        move |op| {
            matches!(op, NirOp::Assign { name: target, value } if target == name && is_append_call(value))
        }
    }

    fn returns_value(op: &NirOp) -> bool {
        matches!(op, NirOp::Return { value: Some(_) })
    }

    fn local(name: &str) -> Place {
        Place::Local(name.to_string())
    }

    fn member(name: &str, field: &str) -> Place {
        Place::Field(name.to_string(), field.to_string())
    }

    /// One function per shape of plan-134-C §Phase 1, called from `main` so every one
    /// is lowered.
    const SHAPES: &str = "IMPORT io
IMPORT collections

TYPE Node
  kids AS List OF Node
  tag AS Integer
END TYPE

FUNC moveAfterBind() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = a
  RETURN b.tag
END FUNC

FUNC readAgain() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET b AS Node = a
  RETURN a.tag + b.tag
END FUNC

FUNC nextIteration() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  MUT xs AS List OF Node = []
  MUT i AS Integer = 0
  WHILE i < 3
    xs = collections::append(xs, a)
    i = i + 1
  END WHILE
  RETURN len(xs)
END FUNC

FUNC reboundEachIteration() AS Integer
  MUT xs AS List OF Node = []
  MUT i AS Integer = 0
  WHILE i < 3
    LET item AS Node = Node[kids := [], tag := i]
    xs = collections::append(xs, item)
    i = i + 1
  END WHILE
  RETURN len(xs)
END FUNC

FUNC readInHandler() AS Integer
  LET a AS Node = Node[kids := [], tag := toInt(\"7\")]
  LET b AS Node = a
  RETURN b.tag
  TRAP(err)
    RETURN a.tag
  END TRAP
END FUNC

FUNC capturedEarlier() AS Integer
  LET a AS Node = Node[kids := [], tag := 1]
  LET f = LAMBDA(n AS Integer) -> a.tag + n
  LET b AS Node = a
  RETURN f(b.tag)
END FUNC

FUNC forEachLive() AS Integer
  LET xs AS List OF Node = [Node[kids := [], tag := 1]]
  MUT n AS Integer = 0
  FOR EACH e IN xs
    LET copy AS List OF Node = xs
    n = n + len(copy) + e.tag
  NEXT
  RETURN n
END FUNC

FUNC fromParam(p AS Node) AS Integer
  LET q AS Node = p
  RETURN q.tag
END FUNC

FUNC fieldThenOtherField() AS Integer
  LET h AS Node = Node[kids := [], tag := 3]
  LET k AS List OF Node = h.kids
  RETURN h.tag + len(k)
END FUNC

FUNC returnLocal() AS Node
  LET n AS Node = Node[kids := [], tag := 1]
  RETURN n
END FUNC

FUNC main AS Integer
  LET r AS Node = returnLocal()
  io::print(toString(moveAfterBind() + readAgain() + nextIteration() + reboundEachIteration() + readInHandler() + capturedEarlier() + forEachLive() + fromParam(r) + fieldThenOtherField()))
  RETURN 0
END FUNC
";

    /// The analysis's answer for every shape, against the table derived by hand from
    /// the source above. Each row names the store op, the place it reads, and whether
    /// that read is the last.
    #[test]
    fn collect_last_use_moves_follows_the_hand_derived_table() {
        let module = lower(SHAPES);
        let model = TypeModel::from_module(&module).expect("the probe's type model builds");
        let rows: Vec<(&str, &str, Box<dyn Fn(&NirOp) -> bool>, Place, bool)> = vec![
            ("never read again", "moveAfterBind", Box::new(binds("b")), local("a"), true),
            ("read again after", "readAgain", Box::new(binds("b")), local("a"), false),
            ("read on the next iteration", "nextIteration", Box::new(appends_to("xs")), local("a"), false),
            ("rebound each iteration, then appended", "reboundEachIteration", Box::new(appends_to("xs")), local("item"), true),
            ("read in the TRAP handler", "readInHandler", Box::new(binds("b")), local("a"), false),
            ("captured by a lambda", "capturedEarlier", Box::new(binds("b")), local("a"), false),
            ("the live FOR EACH iterable", "forEachLive", Box::new(binds("copy")), local("xs"), false),
            ("a parameter", "fromParam", Box::new(binds("q")), local("p"), false),
            ("a field, another field read after", "fieldThenOtherField", Box::new(binds("k")), member("h", "kids"), true),
            ("RETURN of an owned local", "returnLocal", Box::new(returns_value), local("n"), true),
        ];
        for (shape, name, wanted, place, expected) in rows {
            let f = function(&module, name);
            let sites = collect_last_use_moves(f, &model);
            let found = ops(f, wanted.as_ref());
            assert!(!found.is_empty(), "{shape}: `{name}` has no op of the expected shape");
            for op in found {
                assert_eq!(
                    sites.is_last_use(op, &place),
                    expected,
                    "{shape} (`{name}`): is {place:?} read for the last time?"
                );
            }
        }
    }

    fn hand_built(body: Vec<NirOp>) -> NirFunction {
        NirFunction {
            name: "handBuilt".to_string(),
            visibility: "private".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: Vec::new(),
            returns: ParameterType::Integer,
            body,
            file: String::new(),
            resource_owners: HashMap::new(),
        }
    }

    fn zero() -> NirOp {
        NirOp::Return {
            value: Some(NirValue::Const {
                type_: ParameterType::Integer,
                value: "0".to_string(),
            }),
        }
    }

    /// `r` is bound from a by-ref capture — it is a pointer to a binding in the parent
    /// frame, which the parent still owns. The control shows the same shape bound from
    /// an ordinary value IS a move, so the exclusion is what the first answer measures.
    #[test]
    fn collect_last_use_moves_never_moves_a_by_ref_capture() {
        let node = ParameterType::named("Node");
        let bind = |value: NirValue| NirOp::Bind {
            mutable: true,
            name: "r".to_string(),
            type_: node.clone(),
            value: Some(value),
        };
        let copy_out = NirOp::Bind {
            mutable: false,
            name: "s".to_string(),
            type_: node.clone(),
            value: Some(NirValue::Local("r".to_string())),
        };
        let by_ref = hand_built(vec![
            bind(NirValue::Capture {
                index: 0,
                type_: node.clone(),
                by_ref: true,
            }),
            copy_out.clone(),
            zero(),
        ]);
        let sites = collect_last_use_moves(&by_ref, TypeModel::builtin_records());
        assert!(!sites.is_last_use(&by_ref.body[1], &local("r")));

        let owned = hand_built(vec![
            bind(NirValue::Const {
                type_: node.clone(),
                value: "0".to_string(),
            }),
            copy_out,
            zero(),
        ]);
        let sites = collect_last_use_moves(&owned, TypeModel::builtin_records());
        assert!(sites.is_last_use(&owned.body[1], &local("r")));
    }

    /// A resource is move-only by its own rules and never copied (plan-134-A
    /// non-goals), so the analysis never offers it; the control is an `Integer`.
    #[test]
    fn collect_last_use_moves_never_moves_a_resource_bearing_value() {
        let model = TypeModel::builtin_records();
        let run = |type_: ParameterType| {
            let f = hand_built(vec![
                NirOp::Bind {
                    mutable: false,
                    name: "a".to_string(),
                    type_: type_.clone(),
                    value: Some(NirValue::Const {
                        type_: type_.clone(),
                        value: "0".to_string(),
                    }),
                },
                NirOp::Bind {
                    mutable: false,
                    name: "b".to_string(),
                    type_,
                    value: Some(NirValue::Local("a".to_string())),
                },
                zero(),
            ]);
            collect_last_use_moves(&f, model).is_last_use(&f.body[1], &local("a"))
        };
        // The builtin resource table is keyed by the qualified name.
        let file = ParameterType::declared("fs.File");
        assert!(
            type_contains_resource(model, &file),
            "the probe type must be a resource"
        );
        assert!(!run(file));
        assert!(run(ParameterType::Integer));
    }

    /// The five `append`s the decoders build their trees with (plan-134-A §2.1), in the
    /// real helper bodies: each appends a place read for the last time, so letter D can
    /// move instead of deep-copying every element. Three of them append a FIELD of a
    /// record whose other fields are read afterwards, and json's element is itself a
    /// field read — which is why places are field-sensitive.
    const DECODERS: &str = "IMPORT io
IMPORT json
IMPORT regex

FUNC keep(key AS String, value AS json::Json) AS json::Json
  RETURN value
END FUNC

FUNC main AS Integer
  LET doc AS json::Json = json::parse(\"[1,[2]]\")
  LET revived AS json::Json = json::parse(\"[1]\", keep)
  io::print(json::stringify(doc) & json::stringify(revived) & toString(len(regex::findAll(\"ab\", \"a|b\"))))
  RETURN 0
END FUNC
";

    #[test]
    fn collect_last_use_moves_finds_the_five_decoder_append_sites() {
        let module = lower(DECODERS);
        let model = TypeModel::from_module(&module).expect("the probe's type model builds");
        let expect = |name: &str, wanted: &dyn Fn(&NirOp) -> bool, place: Place, count: usize| {
            let f = function(&module, name);
            let sites = collect_last_use_moves(f, &model);
            let found = ops(f, wanted);
            assert_eq!(found.len(), count, "`{name}`: expected {count} such op(s)");
            for op in found {
                assert!(
                    sites.is_last_use(op, &place),
                    "`{name}`: the store of {place:?} must be its last read"
                );
            }
        };
        // A registry helper's `__pkg_name` is spelled `#pkg_name` once lowered.
        expect("#json_parseArrayItems", &appends_to("acc"), local("item"), 1);
        expect("#json_parseArrayItems", &binds("item"), member("parsed", "value"), 1);
        expect("#json_revive", &appends_to("items"), local("revivedItem"), 1);
        expect("#regex_parseAlt", &appends_to("opts"), member("nextc", "node"), 1);
        expect("#regex_parseConcat", &appends_to("parts"), member("q", "node"), 2);
    }
}
