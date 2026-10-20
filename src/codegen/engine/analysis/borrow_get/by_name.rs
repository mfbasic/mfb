//! plan-176-U: the by-name borrow-get classification (`collect_borrow_gets_by_name`) and its
//! per-site tally. Called from `collect_borrow_gets` in `engine/analysis/borrow_get/mod.rs`.

use super::*;

/// The element-bound rules for one `get` binding `e` (see the module doc).
struct SiteCheck<'a> {
    e: &'a str,
    container: &'a ContainerRef,
    index: &'a NirValue,
    /// The locals `index` reads.
    index_names: &'a HashSet<String>,
    /// Whether a local may stand for S1's result: see the through-a-temporary
    /// spelling.
    single_use: &'a dyn Fn(&str) -> bool,
    global_written: &'a dyn Fn(&[NirOp], &str) -> bool,
}

/// The sites [`SiteCheck::segment`] accepted, with the reads of `e` and the
/// `e = WITH e { … }` statements they account for.
#[derive(Default)]
struct SiteTally {
    sites: Vec<ElementSite>,
    reads: usize,
    reassignments: usize,
}

impl SiteCheck<'_> {
    /// `op` writes the container, or is an `IF` with an `ELSE` whose every arm
    /// ends in such a statement.
    fn is_terminal(&self, op: &NirOp) -> bool {
        match op {
            NirOp::If {
                then_body,
                else_body,
                ..
            } => [then_body, else_body]
                .iter()
                .all(|arm| arm.last().is_some_and(|last| self.is_terminal(last))),
            _ => write_back_of(op, self.container).is_some(),
        }
    }

    /// Nothing `ops` runs writes the container or a local of the index.
    fn holds_still(&self, ops: &[NirOp]) -> bool {
        !writes_any(ops, self.index_names)
            && container_untouched(ops, self.container, self.global_written)
    }

    /// `ops[start..w]` is a window, and `ops[w]` writes `e` back to its slot — or is
    /// an `IF` each of whose arms is such a segment. Records the accepted sites
    /// and the reads of `e` they account for into `tally`.
    fn segment(&self, ops: &[NirOp], start: usize, w: usize, tally: &mut SiteTally) -> bool {
        let this = HashSet::from([self.e.to_string()]);
        if let NirOp::If {
            condition,
            then_body,
            else_body,
        } = &ops[w]
        {
            // The window and the condition run before either arm: field reads of
            // `e` only, and nothing that writes the container or the index.
            let before = &ops[start..w];
            let condition = [NirOp::Eval {
                value: condition.clone(),
            }];
            let window = reads_in(before, &this);
            let tested = reads_in(&condition, &this);
            if window.all != window.member
                || tested.all != tested.member
                || !self.holds_still(before)
                || !self.holds_still(&condition)
            {
                return false;
            }
            tally.reads += window.all + tested.all;
            return [then_body, else_body]
                .iter()
                .all(|arm| !arm.is_empty() && self.segment(arm, 0, arm.len() - 1, tally));
        }
        let Some((index, item)) = write_back_of(&ops[w], self.container) else {
            return false;
        };
        if !same_index(index, self.index) {
            return false;
        }
        let key = |j: usize| op_key(&ops[j]);
        // S1 (the statement holding `WITH e { … }`, if any) and the site.
        let (s1, site) = match item {
            // `e = WITH e { … }` (optional) / `xs = set(xs, i, e)`.
            NirValue::Local(p) if p == self.e => {
                let s1 = (w > start).then(|| w - 1).filter(|&j| {
                    matches!(&ops[j], NirOp::Assign { name, value }
                        if name == self.e && is_with_over(value, self.e))
                });
                tally.reads += 1;
                tally.reassignments += usize::from(s1.is_some());
                (
                    s1,
                    ElementSite {
                        template: ops[w].clone(),
                        update: s1.map(key),
                        write_back: Some(key(w)),
                    },
                )
            }
            // `LET u = WITH e { … }` / `xs = set(xs, i, u)`.
            NirValue::Local(u) => {
                let bound = (w > start).then(|| w - 1).filter(|&j| {
                    matches!(&ops[j], NirOp::Bind { name, value: Some(value), .. }
                        if name == u && is_with_over(value, self.e))
                });
                let Some(s1) = bound.filter(|_| (self.single_use)(u)) else {
                    return false;
                };
                (
                    Some(s1),
                    ElementSite {
                        template: ops[w].clone(),
                        update: Some(key(s1)),
                        write_back: Some(key(w)),
                    },
                )
            }
            // Fused: `xs = set(xs, i, WITH e { … })`. The window below stops
            // before W, so W's own update values are checked here.
            with if is_with_over(with, self.e) => {
                if !self.holds_still(&[NirOp::Eval {
                    value: with.clone(),
                }]) {
                    return false;
                }
                (
                    Some(w),
                    ElementSite {
                        template: ops[w].clone(),
                        update: Some(key(w)),
                        write_back: None,
                    },
                )
            }
            _ => return false,
        };
        // In S1 `e` is read through fields and once as the WITH's target.
        if let Some(s1) = s1 {
            let update = reads_in(std::slice::from_ref(&ops[s1]), &this);
            if update.with_target != 1 || update.all != update.member + update.with_target {
                return false;
            }
            tally.reads += update.all;
        }
        // Before S1, only through fields; and nothing before W writes the
        // container or a local of the index.
        let window = reads_in(&ops[start..s1.unwrap_or(w)], &this);
        if window.all != window.member || !self.holds_still(&ops[start..w]) {
            return false;
        }
        tally.reads += window.all;
        tally.sites.push(site);
        true
    }
}

/// The name-keyed classification [`collect_borrow_gets`] runs (on a renamed copy
/// when a name is re-bound).
pub(super) fn collect_borrow_gets_by_name(
    ops: &[NirOp],
    address_taken: &HashSet<String>,
    global_written: &dyn Fn(&[NirOp], &str) -> bool,
) -> BorrowGets {
    /// A `get`/`getOr` binding's initializer.
    struct GetBind {
        type_: ParameterType,
        container: ContainerRef,
        get_or: bool,
        /// `getOr`'s default, when it is a plain local.
        default_local: Option<String>,
        index: NirValue,
    }
    struct Collector {
        gets: HashMap<String, GetBind>,
        copy_src: HashMap<String, String>, // m -> src (Bind { value: Local(src) })
        bind_counts: HashMap<String, usize>,
        assign_counts: HashMap<String, usize>,
        by_ref_captures: HashSet<String>,
        total_reads: HashMap<String, usize>,
        member_reads: HashMap<String, usize>,
        scrutinee_reads: HashMap<String, usize>,
        // Reads of a local as the value of a `UnionExtract` — the MATCH's own
        // read-only variant bindings (`n = UnionExtract($matchN)`), part of
        // consuming the scrutinee, not an escape.
        union_extract_reads: HashMap<String, usize>,
        // bug-776: the `UnionExtract`s that ARE a MATCH's own variant bindings —
        // the direct value of a `Bind` at the top of a case body of a `MATCH`
        // over the same local. Place forwarding (plan-149, and Level-3
        // `aggcopy`) deletes the `$matchN = e` temp, leaving `MATCH e` and
        // `UnionExtract(e)`; these are then `e`'s own borrow-transparent reads.
        case_extract_binds: HashMap<String, usize>,
        // bug-766 B: reads of a local as the target of a `WITH` that is a
        // `set`/`append` item (`copied_item`) — a read the item's copy consumes.
        item_with_targets: HashMap<String, usize>,
        // bug-766 B: the declared type of every `get` (not `getOr`) bind of a name.
        get_bind_types: HashMap<String, Vec<ParameterType>>,
    }
    impl NirVisitor for Collector {
        fn visit_op(&mut self, op: &NirOp) {
            match op {
                NirOp::Bind {
                    name, value, type_, ..
                } => {
                    *self.bind_counts.entry(name.clone()).or_insert(0) += 1;
                    match value {
                        Some(value @ NirValue::Call { .. }) => {
                            if let Some((kind, args)) = get_call(value) {
                                if let (Some(container), Some(index)) =
                                    (args.first().and_then(ContainerRef::of), args.get(1))
                                {
                                    let default_local = match args.get(2) {
                                        Some(NirValue::Local(d)) => Some(d.clone()),
                                        _ => None,
                                    };
                                    if kind == "get" {
                                        self.get_bind_types
                                            .entry(name.clone())
                                            .or_default()
                                            .push(type_.clone());
                                    }
                                    self.gets.insert(
                                        name.clone(),
                                        GetBind {
                                            type_: type_.clone(),
                                            container,
                                            get_or: kind == "getOr",
                                            default_local,
                                            index: index.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        Some(NirValue::Local(src)) => {
                            self.copy_src.insert(name.clone(), src.clone());
                        }
                        Some(NirValue::Capture { by_ref: true, .. }) => {
                            self.by_ref_captures.insert(name.clone());
                        }
                        _ => {}
                    }
                }
                NirOp::Assign { name, .. } => {
                    *self.assign_counts.entry(name.clone()).or_insert(0) += 1;
                }
                NirOp::StateAssign { resource, .. } => {
                    *self.assign_counts.entry(resource.clone()).or_insert(0) += 1;
                }
                NirOp::Match {
                    value: NirValue::Local(scrut),
                    cases,
                } => {
                    *self.scrutinee_reads.entry(scrut.clone()).or_insert(0) += 1;
                    for case in cases {
                        for op in &case.body {
                            if let NirOp::Bind {
                                value: Some(NirValue::UnionExtract { value, .. }),
                                ..
                            } = op
                            {
                                if matches!(value.as_ref(), NirValue::Local(l) if l == scrut) {
                                    *self.case_extract_binds.entry(scrut.clone()).or_insert(0) += 1;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            walk_op(self, op);
        }
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::Local(name) => {
                    *self.total_reads.entry(name.clone()).or_insert(0) += 1;
                }
                NirValue::MemberAccess { target, .. } => {
                    if let NirValue::Local(name) = target.as_ref() {
                        *self.member_reads.entry(name.clone()).or_insert(0) += 1;
                    }
                }
                NirValue::UnionExtract { value: inner, .. } => {
                    if let NirValue::Local(name) = inner.as_ref() {
                        *self.union_extract_reads.entry(name.clone()).or_insert(0) += 1;
                    }
                }
                NirValue::Call { .. } => {
                    if let Some(NirValue::WithUpdate { target, .. }) = copied_item(value) {
                        if let NirValue::Local(name) = target.as_ref() {
                            *self.item_with_targets.entry(name.clone()).or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
            walk_value(self, value);
        }
    }
    let mut c = Collector {
        gets: HashMap::new(),
        copy_src: HashMap::new(),
        bind_counts: HashMap::new(),
        assign_counts: HashMap::new(),
        by_ref_captures: HashSet::new(),
        total_reads: HashMap::new(),
        member_reads: HashMap::new(),
        scrutinee_reads: HashMap::new(),
        union_extract_reads: HashMap::new(),
        case_extract_binds: HashMap::new(),
        item_with_targets: HashMap::new(),
        get_bind_types: HashMap::new(),
    };
    c.visit_ops(ops);
    let count = |map: &HashMap<String, usize>, n: &str| map.get(n).copied().unwrap_or(0);
    let reads = |n: &str| count(&c.total_reads, n);
    let scrut = |n: &str| count(&c.scrutinee_reads, n);
    let extract = |n: &str| count(&c.union_extract_reads, n);
    let members = |n: &str| count(&c.member_reads, n);
    let assigned = |n: &str| count(&c.assign_counts, n) > 0;
    // `m` is a MATCH-scrutinee temp: used only by the MATCH — as its scrutinee (≥1)
    // and as the value of the case `UnionExtract`s (its read-only variant bindings),
    // nothing else. The IR desugars `MATCH e` into `$matchN = e; MATCH $matchN`, so
    // the scrutinee is this temp, not `e`.
    let is_match_temp =
        |m: &str| scrut(m) >= 1 && reads(m) == scrut(m) + extract(m) && !address_taken.contains(m);
    // The match temps that copy each source.
    let mut match_temps_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (m, src) in &c.copy_src {
        if is_match_temp(m) {
            match_temps_of
                .entry(src.as_str())
                .or_default()
                .push(m.as_str());
        }
    }
    // A local is immutable through the scope (bound ≤1, never reassigned, not
    // address-taken) — a reassign frees its old block while a borrow points in.
    let immutable = |l: &str| {
        count(&c.bind_counts, l) <= 1
            && !assigned(l)
            && !address_taken.contains(l)
            && !c.by_ref_captures.contains(l)
    };
    let container_ok = |container: &ContainerRef| match container {
        ContainerRef::Local(l) => !address_taken.contains(l) && !c.by_ref_captures.contains(l),
        ContainerRef::Global(_) => true,
    };

    // Where each candidate is bound: (block, index in block).
    let mut sites: HashMap<&str, (&[NirOp], usize)> = HashMap::new();
    for_each_block(ops, &mut |block| {
        for (k, op) in block.iter().enumerate() {
            if let NirOp::Bind { name, .. } = op {
                if c.gets.contains_key(name) {
                    sites.insert(name.as_str(), (block, k));
                }
            }
        }
    });

    let mut out = BorrowGets::default();
    for (e, get) in &c.gets {
        // One binding of this name, never address-taken — every rule below reasons
        // about THE binding, and a second one would share its classification.
        if count(&c.bind_counts, e) != 1
            || address_taken.contains(e)
            || !container_ok(&get.container)
        {
            continue;
        }
        let temps = match_temps_of.get(e.as_str()).cloned().unwrap_or_default();
        // Every read is borrow-transparent: a field read, a MATCH scrutinee, the
        // value of a match temp's copy-bind, or — once forwarding has removed that
        // temp (bug-776) — one of `e`'s own MATCH's variant bindings. Only extracts
        // in that exact position count; any other `UnionExtract(e)` stays an
        // unexplained read, so `e` copies as before.
        let own_case_extracts = count(&c.case_extract_binds, e);
        let read_only = !assigned(e)
            && reads(e) >= 1
            && reads(e) == members(e) + scrut(e) + own_case_extracts + temps.len();
        if read_only {
            // plan-86 E: an immutable local container (and, for `getOr`, an
            // immutable local default — on a miss the binding IS the default, and a
            // default built by the statement is freed at its end).
            let pinned = matches!(&get.container, ContainerRef::Local(l) if immutable(l))
                && (!get.get_or || get.default_local.as_deref().is_some_and(&immutable));
            if pinned {
                out.names.insert(e.clone());
                out.pinned.insert(e.clone());
                continue;
            }
            // bug-689: the windowed borrow. `getOr` is left to the pinned form: its
            // default has a lifetime of its own this window does not bound.
            if get.get_or {
                continue;
            }
            let Some(&(block, k)) = sites.get(e.as_str()) else {
                continue;
            };
            let mut named: HashSet<String> = HashSet::from([e.clone()]);
            named.extend(temps.iter().map(|m| m.to_string()));
            let Some(last) = (k + 1..block.len())
                .rev()
                .find(|&j| reads_in(std::slice::from_ref(&block[j]), &named).all > 0)
            else {
                continue;
            };
            let window = &block[k + 1..=last];
            // Every read of `e` lies in the window (so none survives past its end).
            if reads_in(window, &HashSet::from([e.clone()])).all != reads(e) {
                continue;
            }
            // The bind itself counts: an index operand that stores to a global
            // container makes bug-496 snapshot it, and the borrow would then point
            // into that statement's temporary copy.
            if container_untouched(&block[k..=last], &get.container, global_written) {
                out.names.insert(e.clone());
            }
            continue;
        }
        // bug-689 Phase 3: the element-bound `get → WITH → set`.
        if get.get_or || !temps.is_empty() || scrut(e) != 0 || !pure_index(&get.index) {
            continue;
        }
        let Some(&(block, k)) = sites.get(e.as_str()) else {
            continue;
        };
        let mut index_names = HashSet::new();
        index_locals(&get.index, &mut index_names);
        if index_names.iter().any(|n| address_taken.contains(n)) {
            continue;
        }
        // bug-766: `u` of `LET u = WITH e { … }` / `set(xs, i, u)` — bound once,
        // never reassigned or captured by reference, read only by the `set`.
        let single_use = |u: &str| {
            count(&c.bind_counts, u) == 1
                && !assigned(u)
                && !address_taken.contains(u)
                && !c.by_ref_captures.contains(u)
                && reads(u) == 1
        };
        let check = SiteCheck {
            e,
            container: &get.container,
            index: &get.index,
            index_names: &index_names,
            single_use: &single_use,
            global_written,
        };
        // The first statement after the bind that writes the container (or is an
        // `IF` every arm of which ends by writing it). The window before it runs
        // from the bind itself (see the read-only window).
        let Some(w) = (k + 1..block.len()).find(|&j| check.is_terminal(&block[j])) else {
            continue;
        };
        let mut tally = SiteTally::default();
        // `e` is read only through fields outside its S1s and write-backs, is
        // reassigned only by its S1s, and nothing between the bind and each W
        // writes the container or a local of the index.
        if !check.segment(block, k, w, &mut tally)
            || count(&c.assign_counts, e) != tally.reassignments
            || reads(e) != tally.reads
        {
            continue;
        }
        let mut sites = tally.sites.into_iter();
        let Some(first) = sites.next() else {
            continue;
        };
        out.names.insert(e.clone());
        out.elements.insert(
            e.clone(),
            ElementBinding {
                type_: get.type_.clone(),
                template: first.template,
                update: first.update,
                write_back: first.write_back,
                other_sites: sites.collect(),
            },
        );
    }
    // bug-766 B: a copy nothing can see outlive its binding. Every bind of the
    // name is a `get` of one type (sibling scopes may each bind it — a local is
    // never redeclared in a nested one, so their lifetimes are disjoint), never
    // reassigned, address-taken or captured by reference, and every read a field
    // read or the target of a `WITH` whose result a `set`/`append` copies — so no
    // read hands the binding's block to anything that keeps it. `getOr` is left
    // alone: on a miss its value is the default, whose lifetime is its own.
    for e in c.gets.keys() {
        let with_items = count(&c.item_with_targets, e);
        let types = c.get_bind_types.get(e).map(Vec::as_slice).unwrap_or(&[]);
        let Some(type_) = types.first() else {
            continue;
        };
        if out.names.contains(e)
            || types.len() != count(&c.bind_counts, e)
            || types.iter().any(|other| other != type_)
            || assigned(e)
            || address_taken.contains(e)
            || c.by_ref_captures.contains(e)
            || reads(e) == 0
            || reads(e) != members(e) + with_items
        {
            continue;
        }
        out.scratch.insert(e.clone(), type_.clone());
    }
    // A match temp `$matchN = Local(e)` that copies a get-borrow `e` also borrows
    // (aliases `e`), so the container element flows into `MATCH` with zero copies.
    for (m, src) in &c.copy_src {
        if is_match_temp(m) && out.names.contains(src) && !out.elements.contains_key(src) {
            out.names.insert(m.clone());
            if out.pinned.contains(src) {
                out.pinned.insert(m.clone());
            }
        }
    }
    out
}
