//! plan-176-N: thread sendability — why a value cannot cross a thread boundary, and the
//! diagnostics for worker and resource-plane transfers. Resource-move checking lives in
//! `ir/verify/resources/mod.rs`.

use super::*;

impl TypeEnv {
    /// The first leaf of `type_` that cannot cross a thread boundary, or `None`
    /// when the whole type can — i.e. `None` *is* "thread-sendable" (the former
    /// source checker's `is_thread_sendable_type`): primitives and the built-in
    /// nominals yes; collections and `Result` by their elements; a `RES`-marked
    /// element, a FUNC and a thread handle never; a resource by its declared
    /// sendability; a record by every field, a union by every variant (a
    /// resource variant by its own bit, bug-173 F); enums yes. A name the tables
    /// do not know is treated as sendable — only a positively known type may
    /// reject.
    ///
    /// Structural arms report the **first** blocking child, left to right, so
    /// the named leaf is deterministic.
    ///
    /// plan-114-A: this replaced the boolean `is_thread_sendable`, rather than
    /// joining it. Answering "is it sendable" and "why is it not" from two
    /// independent walks would let them disagree, and a leaf that one rejects
    /// but the other reports as `None` silently drops a diagnostic. What the
    /// cause carries is which of two rules the plane emits — see [`Unsendable`]
    /// and [`Self::require_thread_sendable`].
    pub(in crate::ir::verify) fn thread_unsendable_cause(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> Option<Unsendable> {
        match type_ {
            ParameterType::Boolean
            | ParameterType::Byte
            | ParameterType::Fixed
            | ParameterType::Float
            | ParameterType::Integer
            | ParameterType::Money
            | ParameterType::Nothing
            | ParameterType::String
            | ParameterType::Unknown => None,
            ParameterType::ListOf(element) | ParameterType::SetOf(element) => {
                self.thread_unsendable_cause(element, seen)
            }
            ParameterType::MapOf(key, value) => self
                .thread_unsendable_cause(key, seen)
                .or_else(|| self.thread_unsendable_cause(value, seen)),
            ParameterType::ResultOf(success) => self.thread_unsendable_cause(success, seen),
            // Sharing a resource collection across threads is out of scope (§15.6).
            ParameterType::Res(_) => Some(Unsendable::Resource(type_.clone())),
            ParameterType::Func(..) => Some(Unsendable::Other(type_.clone())),
            // plan-156-D: a parent view is a sendable resource (its planes are
            // checked where the type is declared); a worker's own view never crosses.
            ParameterType::ThreadHandle { worker: true, .. } => {
                Some(Unsendable::Worker(type_.clone()))
            }
            ParameterType::ThreadHandle { worker: false, .. } => {
                self.resource_sendability_cause(type_)
            }
            other => {
                // The built-in nominals are plain values.
                if other.is_named("AttributedString")
                    || other.is_named("Error")
                    || other.is_named("ErrorLoc")
                    || other.is_named("Scalar")
                {
                    return None;
                }
                if self.is_resource_base(other) {
                    return self.resource_sendability_cause(other);
                }
                if !seen.insert(other.clone()) {
                    return None;
                }
                let result = match self.unions.get(other) {
                    Some(union) => union.variant_order.iter().find_map(|variant| {
                        let variant = ParameterType::declared(variant);
                        if self.is_resource_base(&variant) {
                            return self.resource_sendability_cause(&variant);
                        }
                        self.record_fields_unsendable_cause(&variant, seen)
                    }),
                    None => self.record_fields_unsendable_cause(other, seen),
                };
                seen.remove(other);
                result
            }
        }
    }

    /// A bare resource nominal blocks the boundary unless its type is registered
    /// thread-sendable. The cause is always `Resource`: on a data plane that is a
    /// plane mix-up, on the resource plane it means the resource type itself may
    /// not cross. The emitter decides which rule says so.
    fn resource_sendability_cause(&self, base: &ParameterType) -> Option<Unsendable> {
        (!self.is_resource_sendable(base)).then(|| Unsendable::Resource(base.clone()))
    }

    /// The first field of record `type_` that cannot cross; `None` when every
    /// field can. A name that is not a record (an enum, or a type the tables do
    /// not know) is vacuously sendable.
    fn record_fields_unsendable_cause(
        &self,
        type_: &ParameterType,
        seen: &mut HashSet<ParameterType>,
    ) -> Option<Unsendable> {
        self.record_field_lists.get(type_).and_then(|fields| {
            fields
                .iter()
                .find_map(|(_, field_type)| self.thread_unsendable_cause(field_type, seen))
        })
    }

    /// Reject `type_` if it cannot cross `plane`, naming the blocking leaf.
    ///
    /// plan-114-A: a resource that reaches a **data** plane is a plane mix-up
    /// with a specific remedy, so it gets its own rule. Every other rejection
    /// keeps `TYPE_THREAD_NOT_SENDABLE` — a `Func` or thread handle anywhere,
    /// and anything at all on the resource plane, where "not sendable" means the
    /// resource type itself may not cross and the data-plane remedy would be
    /// nonsense (it would name the plane the resource is already on).
    fn require_thread_sendable(&self, context: &str, type_: &ParameterType, plane: Plane) {
        // A resource named directly on a data plane is a plane mix-up even when the
        // resource itself may cross (the cause walk answers `None` for a sendable
        // one): `thread::start`'s input and a thread's output are data planes. The
        // declared message plane checks this itself before calling here.
        if plane == Plane::Data && self.belongs_on_the_resource_plane(type_) {
            self.emit_thread_resource_plane_required(context, type_);
            return;
        }
        match self.thread_unsendable_cause(type_, &mut HashSet::new()) {
            None => {}
            // A worker's view on a RESOURCE plane is the one mistake with a rule of
            // its own; on a data plane it is simply not sendable (2-203-0063).
            Some(Unsendable::Worker(worker)) if plane == Plane::Resource => {
                self.emit_thread_worker_not_transferable(context, &worker)
            }
            Some(cause @ Unsendable::Resource(_)) if plane == Plane::Data => {
                self.emit_thread_resource_plane_required(context, cause.leaf());
            }
            Some(_) => self.emit(
                "TYPE_THREAD_NOT_SENDABLE",
                format!(
                    "{context} requires a thread-sendable type, got `{}`.",
                    type_.name()
                ),
            ),
        }
    }

    /// Whether a resource on a DATA plane is a plane mix-up whose remedy is the
    /// resource plane (`TYPE_THREAD_RESOURCE_PLANE_REQUIRED`).
    ///
    /// bug-975: a `ThreadWorker` is a resource, but it crosses on NEITHER plane —
    /// it belongs to its worker for the worker's whole run — so naming the
    /// resource plane as its remedy would be wrong advice. Its own rule keeps
    /// precedence, as on the resource plane: on a data plane it stays
    /// `TYPE_THREAD_NOT_SENDABLE`.
    fn belongs_on_the_resource_plane(&self, type_: &ParameterType) -> bool {
        self.is_resource_type(type_)
            && !matches!(
                self.thread_unsendable_cause(type_, &mut HashSet::new()),
                Some(Unsendable::Worker(_))
            )
    }

    /// The one place `2-203-0140` is worded.
    fn emit_thread_worker_not_transferable(&self, context: &str, worker: &ParameterType) {
        self.emit(
            "TYPE_THREAD_WORKER_NOT_TRANSFERABLE",
            format!(
                "{context} carries `{}`; a ThreadWorker belongs to its own worker for that worker's whole run and never crosses to another thread.",
                worker.name()
            ),
        );
    }

    /// The one place `2-203-0138` is worded. `resource` is the offending leaf,
    /// which for a nested type is not `type_` itself — naming the leaf rather
    /// than the enclosing spelling is the point of the cause walk.
    fn emit_thread_resource_plane_required(&self, context: &str, resource: &ParameterType) {
        let name = resource.name();
        let bare = strip_res(resource).name();
        self.emit(
            "TYPE_THREAD_RESOURCE_PLANE_REQUIRED",
            format!(
                "{context} carries resource `{name}`; a resource crosses a thread only on the RES plane. Declare `RES {bare}` on the thread and move it with `thread::transfer` / `thread::accept`."
            ),
        );
    }

    /// The thread-handle arm of the former source checker's declared-type walk
    /// (`check_type_reference`): every `Thread OF M [RES R [STATE S]] TO O`
    /// nested anywhere in a declared type (a collection element, a Map key or
    /// value, a FUNC signature, a `RES` element) must name sendable planes, and
    /// the data plane must not carry a resource (§7: resources ride the resource
    /// plane only).
    pub(in crate::ir::verify) fn check_thread_sendability(&self, type_: &ParameterType) {
        fn strip_res(type_: &ParameterType) -> &ParameterType {
            match type_ {
                ParameterType::Res(inner) => inner.as_ref(),
                other => other,
            }
        }
        match type_ {
            ParameterType::ListOf(element) => self.check_thread_sendability(strip_res(element)),
            ParameterType::SetOf(element) => self.check_thread_sendability(element),
            ParameterType::MapOf(key, value) => {
                self.check_thread_sendability(key);
                self.check_thread_sendability(strip_res(value));
            }
            ParameterType::Res(inner) => self.check_thread_sendability(inner),
            ParameterType::Func(params, returns, _) => {
                for param in params {
                    self.check_thread_sendability(param);
                }
                self.check_thread_sendability(returns);
            }
            ParameterType::ThreadHandle { msg, res, out, .. } => {
                self.check_thread_sendability(msg);
                self.check_thread_sendability(out);
                // A resource named directly on the message plane is a plane
                // mix-up, and it is reported here rather than by the cause walk:
                // the sendable resources (`fs::File`, `tcp`, `tls`, `udp`) are
                // *sendable*, so the walk answers `None` for them. Rejecting the
                // plane is a separate rule from rejecting the type, and the two
                // are mutually exclusive so one mistake yields one diagnostic.
                if self.belongs_on_the_resource_plane(msg) {
                    self.emit_thread_resource_plane_required("Thread message type", msg);
                } else {
                    self.require_thread_sendable("Thread message type", msg, Plane::Data);
                }
                self.require_thread_sendable("Thread output type", out, Plane::Data);
                // An absent resource plane is `Nothing`; a present one carries
                // its ` STATE T` inside the nominal's spelling.
                let (plane_resource, plane_state) = res.split_state();
                if !matches!(plane_resource, ParameterType::Nothing) {
                    self.check_thread_sendability(&plane_resource);
                    // The resource plane: `Resource` here means this resource
                    // type may not cross at all, not that it is on the wrong
                    // plane — it is already on the right one.
                    self.require_thread_sendable(
                        "Thread resource type",
                        &plane_resource,
                        Plane::Resource,
                    );
                }
                if let Some(plane_state) = &plane_state {
                    self.check_thread_sendability(plane_state);
                    // The `STATE` payload is deep-copied data riding the resource
                    // plane, so it is a DATA plane for this rule (bug-301 G4).
                    self.require_thread_sendable(
                        "Thread resource STATE type",
                        plane_state,
                        Plane::Data,
                    );
                }
            }
            _ => {}
        }
    }

    /// the former source checker's `check_thread_boundary_sendability`: the values that cross
    /// at `thread::start` (the input and the new handle's planes),
    /// `thread::send` (the message) and `thread::transfer`/`accept` (the
    /// resource plane and its STATE). Runs only for a call that resolved (its
    /// lowered type is known): an unresolvable call is an arity/argument
    /// rejection, and the former source checker never reached the boundary rules for it.
    pub(in crate::ir::verify) fn check_thread_boundary_sendability(
        &self,
        target: &str,
        args: &[IrValue],
        call: &IrValue,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::codegen::builtins::thread::{ACCEPT_RESOURCE, TRANSFER_RESOURCE};
        let display = match target {
            TRANSFER_RESOURCE => "thread.transfer",
            ACCEPT_RESOURCE => "thread.accept",
            "thread.start" | "thread.send" => target,
            _ => return,
        };
        // bug-482, the `In` rule. This runs BEFORE the `return_type` guard below,
        // and it is the only boundary rule that may. The `data` argument's
        // sendability is a property of the argument alone — the call's own type
        // says nothing about it — so gating it on a derived return type made it
        // unreachable exactly where it matters. Measured on the repro: in the
        // source pass `infer_type(call)` is `None` (the entry is an imported
        // function whose signature that pass does not hold) while the argument
        // infers fine, so the guard returned before this rule and only the
        // merged/native pass ever reached it — see the comment on the gate below.
        //
        // Keeping it here is also what makes the diagnostic USABLE: the source
        // pass has `current_file`/`current_line` set (measured: `src/main.mfb:18`),
        // so the rule reports at the call site, whereas the native pass reports
        // through `verify_semantics`'s error-return channel, which carries no span
        // and is skipped entirely for `-ast`/`-ir` dumps — the very invocation
        // `scripts/test-accept.sh` uses for a `tests/syntax/` fixture. A rule that
        // only fires in the native pass cannot be pinned by a syntax golden.
        if target == "thread.start" {
            if let Some(Some(input)) = args.get(1).map(|arg| self.infer_type(arg, locals)) {
                self.require_thread_sendable(
                    &format!("Call to `{display}` input"),
                    &input,
                    Plane::Data,
                );
            }
        }
        let Some(return_type) = self.infer_type(call, locals) else {
            return;
        };
        if matches!(return_type, ParameterType::Unknown) {
            return;
        }
        let arg_types: Vec<Option<ParameterType>> = args
            .iter()
            .map(|arg| self.infer_type(arg, locals))
            .collect();
        match target {
            // bug-482: `thread.start`'s only call-site rule is the `In` rule, and
            // it runs above (before the `return_type` guard, which it must not
            // depend on).
            //
            // There used to be three more here — message/resource/output — behind
            // an `imported_entry` gate whose premise ("a `FunctionRef` name absent
            // from `functions` came from an import") is backwards: `TypeEnv::build`
            // inserts every function of `IrProject::functions`, and an imported
            // package's functions are lowered into that same list under the
            // package-keyed name the entry's `FunctionRef` carries (measured:
            // `first="04187eb9abe9cc82.wpkg.w"`,
            // `fnkeys=["$lambda0", "04187eb9abe9cc82.wpkg.w", "main"]`). The gate
            // was false for an imported entry and a same-project one alike, so all
            // four rules were dead.
            //
            // Only the `In` rule was resurrected, because only it has no other
            // guard. The other three are strictly redundant with the declared-type
            // walk (`check_thread_sendability`): `thread::start`'s handle planes
            // are read off the entry's own `ThreadWorker OF Msg TO Out` parameter
            // and return type, which are declarations the walk already checks —
            // whether or not the binding is annotated, and whether the entry is
            // local or imported (the merged pass walks the imported declarations
            // too, so a crafted `.mfp` stays covered). Reviving them reported the
            // same defect twice at one line, measured on
            // `tests/syntax/threads/func_thread_start_invalid` line 18:
            //     …:18 … `Call to `thread.start` message type … got `BadStartMessage`.`
            //     …:18 … `Thread message type … got `BadStartMessage`.`
            // so they are deleted rather than restored into duplicates.
            "thread.send" => {
                if let Some(Some(ParameterType::ThreadHandle { msg, .. })) = arg_types.first() {
                    // The data plane is resource-free: a resource moves across a
                    // thread only via `thread::transfer` (§7). As on the declared
                    // message plane, the plane rule and the sendability rule are
                    // mutually exclusive — a sendable resource still fails the
                    // plane rule, and the cause walk would not report it.
                    if self.belongs_on_the_resource_plane(msg) {
                        self.emit_thread_resource_plane_required(
                            &format!("Call to `{display}` message type"),
                            msg,
                        );
                    } else {
                        self.require_thread_sendable(
                            &format!("Call to `{display}` message type"),
                            msg,
                            Plane::Data,
                        );
                    }
                }
            }
            _ => {
                let Some(Some(ParameterType::ThreadHandle { res, .. })) = arg_types.first() else {
                    return;
                };
                let (resource, resource_state) = res.split_state();
                // bug-301 G4: the plane's `STATE T` payload crosses the boundary
                // with the resource (deep-copied into the receiver's arena), so it
                // must be sendable too.
                if let Some(resource_state) = &resource_state {
                    // Deep-copied data riding the resource plane: a DATA plane
                    // for this rule.
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource STATE type"),
                        resource_state,
                        Plane::Data,
                    );
                }
                if matches!(resource, ParameterType::Nothing) {
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Call to `{display}` requires a thread with a resource plane (`Thread OF … RES Res TO …`); this thread has no resource channel."
                        ),
                    );
                } else if let Some(cause @ Unsendable::Worker(_)) =
                    self.thread_unsendable_cause(&resource, &mut HashSet::new())
                {
                    self.emit_thread_worker_not_transferable(
                        &format!("Call to `{display}` resource type"),
                        cause.leaf(),
                    );
                } else if self.is_resource_type(&resource) {
                    // The resource plane carries only thread-sendable resources.
                    self.require_thread_sendable(
                        &format!("Call to `{display}` resource type"),
                        &resource,
                        Plane::Resource,
                    );
                } else {
                    self.emit(
                        "TYPE_THREAD_NOT_SENDABLE",
                        format!(
                            "Call to `{display}` carries `{}`, which is not a resource; the resource plane moves only resources.",
                            resource.name()
                        ),
                    );
                }
            }
        }
    }
}
