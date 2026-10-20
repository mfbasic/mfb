//! plan-176-M: call-site unification — matching argument types against a signature's type
//! variables, unions and resource slots, and substituting the result. `Registry` and `build()`
//! live in `codegen/registry/mod.rs`.

use super::*;

/// Whether a concrete leaf type is compatible with a *scalar or nominal* parameter
/// type (the [`unify`] leaf case). Exact types match, and two *different known scalars*
/// never do. LENIENT mode accepts a nominal against anything else; STRICT mode holds a
/// scalar-vs-nominal pair, a resource parameter and a registry value-type parameter to
/// identity, and accepts any other nominal pair. Container,
/// [`Var`](ParameterType::Var), and [`Unknown`](ParameterType::Unknown) cases never
/// reach here; [`unify`] handles them first.
pub(super) fn leaf_matches(
    pattern: &ParameterType,
    concrete: &ParameterType,
    strict: bool,
) -> bool {
    // The `RES ` collection-element ownership marker is transparent to matching:
    // historically `parse` stripped it before the matcher ever saw it, so a
    // `Res(inner)` on either side matches exactly as `inner` would. Unwrap
    // symmetrically to reproduce that behavior byte-for-byte.
    if let ParameterType::Res(inner) = concrete {
        return leaf_matches(pattern, inner, strict);
    }
    if let ParameterType::Res(inner) = pattern {
        return leaf_matches(inner, concrete, strict);
    }
    if pattern == concrete {
        return true;
    }
    // A container CONCRETE against a RESOURCE leaf pattern matches only when their spelled
    // names are equal. A genuine resource nominal (`Named("tls.Socket")`) is NOT a
    // container spelling, so it is correctly rejected against a `List OF RES tls::Socket`
    // concrete — which lets a same-arity resource-nominal-vs-list overload pair
    // (`tls::poll`: scalar `tls::Socket → Boolean` vs `List OF RES tls::Socket → tls::Socket`)
    // select by argument shape on the lenient dispatch / return-inference path.
    // Gated on the pattern being a RESOURCE (not any nominal): a NON-resource leaf keeps the
    // pre-existing coarse `true` below — a value nominal or a string-blob spelling like
    // fs::pathJoin's `Named("List OF String")` must still accept a container arg, and
    // broadening the name-equal rule to all nominals perturbs the lenient overload dispatch
    // of every package with container-arg overloads (json/crypto/http/…). (Container
    // PATTERNS are handled by the arms in `unify`; this is the mirror case.)
    if is_resource_type_name(&pattern.name())
        && matches!(
            concrete,
            ParameterType::ListOf(_)
                | ParameterType::SetOf(_)
                | ParameterType::MapOf(_, _)
                | ParameterType::Func(_, _, _)
        )
    {
        return pattern.name() == concrete.name();
    }
    // Two unequal scalars never match, in either mode.
    if pattern.is_scalar() && concrete.is_scalar() {
        return false;
    }
    // STRICT (argument validation): a scalar and a nominal/non-scalar leaf are never
    // compatible either — a `Named("Json")` parameter cannot accept a `String` argument
    // (bug-443). LENIENT (overload dispatch / return-type inference): stay coarse so a
    // nominally-spelled or not-yet-resolved argument is not rejected, which would perturb
    // overload selection and type propagation on valid programs (e.g. a `csv::stringify`
    // nested-list argument degrading to `List OF Unknown`).
    if strict && (pattern.is_scalar() || concrete.is_scalar()) {
        return false;
    }
    // STRICT (argument validation): a RESOURCE parameter demands exact base-resource
    // identity — bug-427 STATE/ownership-agnostic, so a `File STATE Cursor` argument still
    // satisfies a `File` parameter, but an unrelated resource or a resource UNION does NOT
    // satisfy a concrete resource close-op parameter (`fs::close(<Stream union>)` must be
    // rejected — a use-after-free class error the legacy exact-name `DefaultResolver`
    // caught). A NON-resource value type is the next rule's; lenient dispatch/inference
    // stays coarse everywhere so overload selection and type propagation are unperturbed.
    if strict && is_resource_type_name(&pattern.name()) {
        return resource_base_eq(pattern, concrete);
    }
    // STRICT (argument validation): a REGISTRY value type — a built-in record, enum or
    // union — is a nominal identity too (bug-638). `datetime::toMillis(datetime::DateTime)`
    // resolved against its `datetime.Instant` parameter and read the `DateTime`'s leading
    // words as an `Instant`. The union arm keeps the widening this rule used to stay
    // coarse for (`json::stringify(JsonNull)`). A nominal the registry does not declare
    // (`Scalar`, `AttributedString`) stays coarse.
    if strict {
        if let Some(accepts) = value_type_accepts(&pattern.name(), concrete) {
            return accepts;
        }
    }
    true
}

/// Whether `concrete` is a member of `union` -- one of its own variants (a variant that
/// is itself a union widening transitively), or a member of a union it `INCLUDES`.
fn union_has_member(
    package: &str,
    union: &RegistryUnion,
    concrete: &ParameterType,
    actual: &str,
) -> bool {
    union.variants.iter().any(|variant| {
        let variant = match variant.name.split_once("::") {
            Some((owner, leaf)) => format!("{owner}.{leaf}"),
            None => format!("{package}.{}", variant.name),
        };
        variant == actual || value_type_accepts(&variant, concrete) == Some(true)
    }) || union.includes.iter().any(|included| {
        matches!(
            registry().resolve_type(&format!("{package}.{included}")),
            Some(ResolvedType::Union(inner)) if union_has_member(package, inner, concrete, actual)
        )
    })
}

/// Whether the registry value type `declared` (package-qualified, `datetime.Instant`)
/// accepts a `concrete` argument in strict matching, or `None` when `declared` names no
/// registry record, enum or union. A record or enum accepts only itself; a union accepts
/// itself or any of its variants, a variant spelled bare being local to the union's
/// package (`JsonNull` is `json.JsonNull`) and one spelled `pkg::Name` naming that
/// package's type (`http.Stream`'s `tcp::Socket`). A variant that is itself a union
/// widens transitively. A resource variant's `STATE` clause is transparent (bug-427).
///
/// plan-172-K: the members of a union this one `INCLUDES` are its members too
/// ([`union_has_member`]) -- but the included union's own TYPE is not: `INCLUDES` is not
/// subtyping.
fn value_type_accepts(declared: &str, concrete: &ParameterType) -> Option<bool> {
    let package = declared.split_once('.')?.0;
    let actual = concrete.without_state().name().into_owned();
    match registry().resolve_type(declared)? {
        ResolvedType::Record(_) | ResolvedType::Enum(_) => Some(actual == declared),
        ResolvedType::Union(union) => {
            Some(actual == declared || union_has_member(package, union, concrete, &actual))
        }
        ResolvedType::Resource(_) => None,
    }
}

/// Whether `union` names a REGISTRY union (a built-in package's, `canvas.DrawItem`)
/// that has `actual` among its variants — the answer [`TypeKinds::union_accepts`] gives
/// when the caller's oracle knows no declared unions (bug-988).
pub(crate) fn registry_union_accepts(union: &ParameterType, actual: &ParameterType) -> bool {
    let union = union.without_state().name().into_owned();
    matches!(
        registry().resolve_type(&union),
        Some(ResolvedType::Union(_))
    ) && value_type_accepts(&union, actual) == Some(true)
}

/// Whether `name` (a parameter's type leaf, possibly carrying a `STATE` clause) names a
/// resource handle — a legacy builtin resource (`net.Socket`) or a migrated-package
/// resource registered via [`RegistryPackage::add_resource`] (`fs.File`, addressed by its
/// package-qualified id whose bare tail is the `RegistryResource::name`). Only a resource
/// parameter triggers the strict exact-base match in [`leaf_matches`]; value nominals
/// (unions, records) stay coarse.
fn is_resource_type_name(name: &str) -> bool {
    // A single bare-name scan of the registry: a qualified builtin id (`fs.File`) and
    // its bare base (`File`) both reduce to the same tail, so one scan answers both
    // the package-qualified and bare-base cases the two former branches split out.
    let base = crate::codegen::resource::base_resource_name(name);
    let bare = base.rsplit('.').next().unwrap_or(base);
    registry()
        .packages()
        .iter()
        .any(|package| package.resources().iter().any(|r| r.name == bare))
}

/// Structurally unify a parameter-type `pattern` — which may contain
/// [`ParameterType::Var`] type variables — against a `concrete` argument type,
/// recording every variable binding in `bindings`. Returns `false` on a structural or
/// scalar mismatch, or on a variable bound inconsistently to two different types
/// (`get(List OF Integer, String)` fails: `T` is bound to `Integer` by arg 0, then
/// contradicted by `String`). An [`Unknown`](ParameterType::Unknown) concrete (an
/// unresolved argument) matches any pattern; a bare variable it meets is bound to
/// `Unknown` so [`substitute`] propagates the unresolved-ness rather than inventing a
/// type.
pub(super) fn unify(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
) -> bool {
    unify_with_kinds(
        pattern,
        concrete,
        bindings,
        strict,
        &NoTypeKinds,
        &RefCell::new(BTreeSet::new()),
    )
}

/// [`unify`] with the caller's declared-type oracle and the call's pinned-variable
/// set. `kinds` answers whether a union accepts a variant
/// ([`TypeKinds::union_accepts`]); `pinned` must be the same set for every parameter
/// of one overload attempt, like `bindings`. Together they carry union-variant
/// subsumption into a generic call, in ONE direction — see [`VarPosition`].
pub(super) fn unify_with_kinds(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
    kinds: &dyn TypeKinds,
    pinned: &RefCell<BTreeSet<Symbol>>,
) -> bool {
    unify_at(
        pattern,
        concrete,
        bindings,
        strict,
        NothingBinding::Allowed,
        Position {
            kinds,
            var: VarPosition::Value,
            pinned,
        },
    )
}

/// Where in a parameter a type variable is met, which decides what
/// union-variant subsumption may do there (bug-988, bug-989).
///
/// Subsumption goes one way, variant to union — a member type fits a union
/// bucket, a union never fits a member bucket — and lowering realizes it by
/// wrapping a whole argument (`UnionWrap`) or converting a `List`'s elements or a
/// `Map`'s values in a copy (`ir::lower::union_conversion`).
///
/// * `Value`: the variable is the whole parameter (`append`'s item `T`, `reduce`'s
///   initial `U`). One value arrives here, and it can be wrapped.
/// * `Element`: the variable is a `List` element or `Map` value type, at any depth
///   (`append`'s `List OF T`). The collection can be converted.
/// * `Invariant`: everywhere else — a `Set` element or `Map` key (a union is not
///   comparable, so no union form exists), a callable's parameters and result, a
///   `Result`, a user generic's argument, a thread slot. Nothing converts here.
///
/// Two rules follow, and no others:
///
/// 1. A variable already bound to a union form accepts its VARIANT form at a
///    `Value` or `Element` position and stays bound to the union. The bucket is a
///    union; the member fits: `append(shapes, Square[1.0])` and `append(shapes,
///    squares)` are `T = Shape`.
/// 2. A variable bound ONLY from `Value` occurrences (it is not in `pinned`), to a
///    variant, is rebound to a union a later occurrence brings. The earlier values
///    were single values, so they are wrapped: `reduce(shapes, Square[1.0],
///    LAMBDA(acc AS Shape, s AS Shape) -> acc)` binds `U = Square` from the initial
///    value and then `U = Shape` from the callback, which is the bucket.
///
/// A variable bound from a collection argument is the BUCKET and is pinned at
/// once: it never widens to fit a later argument. `append(squares, aShape)` is
/// `T = Square` from the list, and the `Shape` item does not fit — a mismatch,
/// never `T = Shape` with the list converted (owner decision 2026-10-10: "`T`
/// *must* match"). An `Invariant` occurrence pins the same way.
#[derive(Clone, Copy, PartialEq, Eq)]
enum VarPosition {
    Value,
    Element,
    Invariant,
}

/// The per-position context [`unify_at`] threads: the declared-type oracle, where
/// in the parameter a variable met here sits, and the variables that are pinned
/// (bound or matched anywhere but a `Value` position).
#[derive(Clone, Copy)]
struct Position<'k> {
    kinds: &'k dyn TypeKinds,
    var: VarPosition,
    pinned: &'k RefCell<BTreeSet<Symbol>>,
}

impl<'k> Position<'k> {
    /// A `List` element or `Map` value of this position: convertible when the
    /// collection itself sits at a convertible position.
    fn element(self) -> Position<'k> {
        Position {
            var: match self.var {
                VarPosition::Value | VarPosition::Element => VarPosition::Element,
                VarPosition::Invariant => VarPosition::Invariant,
            },
            ..self
        }
    }

    fn nested(self) -> Position<'k> {
        Position {
            var: VarPosition::Invariant,
            ..self
        }
    }

    /// Whether a variant form is accepted here for a variable bound to its union.
    fn subsumes(self) -> bool {
        self.var != VarPosition::Invariant
    }

    /// Record that `name` has now been bound or matched somewhere other than a
    /// `Value` position, so its binding is fixed and never widens to a union.
    fn pin_unless_value(self, name: Symbol) {
        if self.var != VarPosition::Value {
            self.pinned.borrow_mut().insert(name);
        }
    }
}

/// Whether a value of type `from` converts to `to` by union-variant subsumption:
/// `to` is a union with `from` among its variants, or both are `List`s (or `Map`s of
/// one key type) whose elements (values) convert. One direction only, and never
/// true for equal types.
pub(super) fn converts_to_union_form(
    kinds: &dyn TypeKinds,
    from: &ParameterType,
    to: &ParameterType,
) -> bool {
    match (from, to) {
        (ParameterType::ListOf(from), ParameterType::ListOf(to)) => {
            converts_to_union_form(kinds, from, to)
        }
        (ParameterType::MapOf(from_key, from), ParameterType::MapOf(to_key, to)) => {
            from_key == to_key && converts_to_union_form(kinds, from, to)
        }
        (ParameterType::Res(from), _) => converts_to_union_form(kinds, from, to),
        (_, ParameterType::Res(to)) => converts_to_union_form(kinds, from, to),
        _ => from != to && kinds.union_accepts(to, from),
    }
}

/// Whether a type variable met at this position may bind to `Nothing` under STRICT
/// validation (bug-745).
///
/// `Nothing` is a first-class type (owner decision 2026-09-26), so an element, key,
/// value or plain argument position binds it like any other type:
/// `collections::contains(List OF Nothing, NOTHING)` is `T = Nothing`. Two positions
/// still refuse it, each for a reason of its own:
///
/// * a callback's RESULT (`transform`'s `FUNC(T) AS U`): an effect-only callback
///   collects nothing, so a `FUNC … AS Nothing` does not satisfy a
///   value-producing callback (`mfb spec language subs` §7; bug-443);
/// * a thread-handle slot: a data-only handle's `Nothing` resource plane must fail a
///   member that captures `res` (`accept`/`transfer`), see [`thread_slot_unifies`].
#[derive(Clone, Copy, PartialEq, Eq)]
enum NothingBinding {
    Allowed,
    Refused,
}

fn unify_at(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
    nothing: NothingBinding,
    at: Position<'_>,
) -> bool {
    // `RES ` is transparent to unification (historically stripped by `parse` before
    // the matcher): unwrap it on either side so a `Var` binds to the unwrapped inner
    // (dropping the marker, exactly as before) and container recursion is unperturbed.
    if let ParameterType::Res(inner) = concrete {
        return unify_at(pattern, inner, bindings, strict, nothing, at);
    }
    if let ParameterType::Res(inner) = pattern {
        return unify_at(inner, concrete, bindings, strict, nothing, at);
    }
    // plan-156-E: a thread view's STATE is the view's own, not part of its channel
    // types, so a thread-handle pattern matches a stateful view by its handle (and
    // `thread::start` never compares a parent's STATE with its worker's). A `Var`
    // still binds the whole type.
    if let ParameterType::Stateful { base, .. } = concrete {
        if matches!(**base, ParameterType::ThreadHandle { .. })
            && !matches!(pattern, ParameterType::Var(_))
        {
            return unify_at(pattern, base, bindings, strict, nothing, at);
        }
    }
    if matches!(concrete, ParameterType::Unknown) {
        if let ParameterType::Var(name) = pattern {
            bindings.entry(*name).or_insert(ParameterType::Unknown);
        }
        return true;
    }

    match (pattern, concrete) {
        (ParameterType::Var(name), _) => {
            // STRICT validation: a type variable in a callback-result or thread-slot
            // position never binds to `Nothing` — a value-returning callback
            // parameter (`transform`'s `FUNC(T) AS U`) is not satisfied by a
            // `FUNC(..) AS Nothing` argument (bug-443). Everywhere else `Nothing` is an
            // ordinary type (bug-745, [`NothingBinding`]). The lenient
            // dispatch/inference path binds it everywhere, so `Nothing`-returning
            // callbacks (`forEach`) keep lowering.
            if strict
                && nothing == NothingBinding::Refused
                && matches!(concrete, ParameterType::Nothing)
            {
                return false;
            }
            match bindings.get(name) {
                // A prior binding to `Unknown` (an earlier *unresolved* occurrence of this
                // variable) is REFINED by a later concrete occurrence, not treated as a
                // conflict: `Unknown` means "not yet resolved", so a re-occurring variable
                // whose first sighting was an unknown-typed argument still unifies with a
                // later concrete one — `send(Thread OF Unknown TO Out, Integer)` binds
                // `Msg = Unknown` from the handle then refines it to `Integer` from the
                // message arg, instead of failing `resource_base_eq(Unknown, Integer)`.
                Some(ParameterType::Unknown) => {
                    bindings.insert(*name, concrete.clone());
                    at.pin_unless_value(*name);
                    true
                }
                // A re-occurring variable must match its binding — but resource element
                // types compare STATE/ownership-agnostically (bug-427): a bound element
                // `Handle STATE Cursor` accepts an item spelled `Handle`
                // (`resource_base_eq`, plain `==` for every non-resource type).
                Some(bound) if resource_base_eq(bound, concrete) => {
                    at.pin_unless_value(*name);
                    true
                }
                // bug-988 / bug-989: at a convertible position, a variable bound to a
                // union form accepts its variant form and stays bound to the union —
                // `append(List OF Shape, Square)` and `append(List OF Shape, List OF
                // Square)` are both `T = Shape`. The argument's expected type is then
                // the union form, and lowering converts it ([`VarPosition`]).
                Some(bound)
                    if at.subsumes() && converts_to_union_form(at.kinds, concrete, bound) =>
                {
                    true
                }
                // The other order, for a variable bound only from whole VALUES so far
                // (not pinned): this occurrence is the union form of what they bound,
                // so the variable IS the union and those values are wrapped
                // (`reduce`'s variant initial value). A variable bound from a
                // collection argument is the bucket, is pinned, and never gets here:
                // `append(squares, aShape)` is a mismatch. Never the reverse either: a
                // union does not become its variant.
                Some(bound)
                    if !at.pinned.borrow().contains(name)
                        && converts_to_union_form(at.kinds, bound, concrete) =>
                {
                    bindings.insert(*name, concrete.clone());
                    at.pin_unless_value(*name);
                    true
                }
                Some(_) => false,
                None => {
                    bindings.insert(*name, concrete.clone());
                    at.pin_unless_value(*name);
                    true
                }
            }
        }
        (ParameterType::ListOf(elem), ParameterType::ListOf(concrete_elem)) => unify_at(
            elem,
            concrete_elem,
            bindings,
            strict,
            NothingBinding::Allowed,
            at.element(),
        ),
        (ParameterType::SetOf(elem), ParameterType::SetOf(concrete_elem)) => unify_at(
            elem,
            concrete_elem,
            bindings,
            strict,
            NothingBinding::Allowed,
            at.nested(),
        ),
        (ParameterType::MapOf(key, value), ParameterType::MapOf(concrete_key, concrete_value)) => {
            unify_at(
                key,
                concrete_key,
                bindings,
                strict,
                NothingBinding::Allowed,
                at.nested(),
            ) && unify_at(
                value,
                concrete_value,
                bindings,
                strict,
                NothingBinding::Allowed,
                at.element(),
            )
        }
        (
            ParameterType::MapEntryOf(key, value),
            ParameterType::MapEntryOf(concrete_key, concrete_value),
        ) => {
            unify_at(
                key,
                concrete_key,
                bindings,
                strict,
                NothingBinding::Allowed,
                at.nested(),
            ) && unify_at(
                value,
                concrete_value,
                bindings,
                strict,
                NothingBinding::Allowed,
                at.nested(),
            )
        }
        (ParameterType::ResultOf(success), ParameterType::ResultOf(concrete_success)) => unify_at(
            success,
            concrete_success,
            bindings,
            strict,
            NothingBinding::Allowed,
            at.nested(),
        ),
        // A user generic unifies head-then-arguments, exactly like the container arms
        // (plan-105-B). Same template name and same arity, then each argument
        // recursively — so `Stack OF T` binds `T` from `Stack OF Integer` instead of
        // comparing two opaque `Named` blobs by string equality.
        (
            ParameterType::UserOf(name, args),
            ParameterType::UserOf(concrete_name, concrete_args),
        ) => {
            name == concrete_name
                && args.len() == concrete_args.len()
                && args.iter().zip(concrete_args.iter()).all(|(a, c)| {
                    unify_at(a, c, bindings, strict, NothingBinding::Allowed, at.nested())
                })
        }
        (
            ParameterType::Func(params, ret, isolated),
            ParameterType::Func(concrete_params, concrete_ret, concrete_isolated),
        ) => {
            // bug-824 / owner rule (2026-10-04): "an ISOLATED FUNC is a FUNC. a
            // FUNC is NOT an ISOLATED FUNC." Isolation is an extra guarantee, not
            // part of the callable's shape, so it widens in exactly one direction
            // — the same direction `compat::Scope::assignable` already applies to
            // a `LET`/argument/return/field (bug-786): a plain pattern accepts
            // either an isolated or a plain concrete, but an isolated pattern
            // (`thread::start`'s worker) accepts only an isolated concrete.
            (!isolated || *concrete_isolated)
                && params.len() == concrete_params.len()
                && params.iter().zip(concrete_params.iter()).all(|(p, c)| {
                    unify_at(p, c, bindings, strict, NothingBinding::Allowed, at.nested())
                })
                && unify_at(
                    ret,
                    concrete_ret,
                    bindings,
                    strict,
                    NothingBinding::Refused,
                    at.nested(),
                )
        }
        // Two thread handles unify structurally, exactly like the container arms: the
        // kind (parent `Thread` vs worker `ThreadWorker`) must match — the two never
        // interconvert — and each slot unifies via [`thread_slot_unifies`]. A slot the
        // member does NOT echo is spelled `Unknown` and wildcards (accepting any
        // concrete slot, including a `Nothing` message/output or an absent resource
        // plane); an echoed slot is a `Var`/concrete and unifies STATE-agnostically
        // (bug-427), so a `File STATE Cursor` handle satisfies a `File` plane and a
        // data-only handle's `Nothing` resource plane fails a `Var` `res` under strict.
        (
            ParameterType::ThreadHandle {
                worker: p_worker,
                msg: p_msg,
                res: p_res,
                out: p_out,
            },
            ParameterType::ThreadHandle {
                worker: c_worker,
                msg: c_msg,
                res: c_res,
                out: c_out,
            },
        ) => {
            p_worker == c_worker
                && thread_slot_unifies(p_msg, c_msg, bindings, strict, at)
                && thread_slot_unifies(p_out, c_out, bindings, strict, at)
                && thread_slot_unifies(p_res, c_res, bindings, strict, at)
        }
        // A container/function/thread pattern against a non-matching concrete fails.
        (
            ParameterType::ListOf(_)
            | ParameterType::SetOf(_)
            | ParameterType::MapOf(_, _)
            | ParameterType::MapEntryOf(_, _)
            | ParameterType::ResultOf(_)
            | ParameterType::UserOf(_, _)
            | ParameterType::Func(_, _, _)
            | ParameterType::ThreadHandle { .. },
            _,
        ) => false,
        // Scalar or nominal leaf.
        (leaf, _) => leaf_matches(leaf, concrete, strict),
    }
}

/// Unify one slot (`msg`/`res`/`out`) of a [`ParameterType::ThreadHandle`]. A member
/// that does NOT echo the slot spells it [`ParameterType::Unknown`], which wildcards —
/// it accepts any concrete slot, including a `Nothing` message/output (a resource-only
/// or `Nothing`-returning thread) or an absent resource plane — so the strict-`Nothing`
/// guard only bites where a slot is genuinely captured. A member that ECHOES the slot
/// spells it a `Var`/concrete, which unifies STATE-agnostically through the normal
/// recursion (the `Var` arm uses [`resource_base_eq`]); a data-only concrete `res`
/// (`Nothing`) then fails the `Var`'s strict-`Nothing` guard, correctly rejecting
/// `accept`/`transfer` on a resource-free handle exactly as the legacy resolver did.
fn thread_slot_unifies(
    pattern: &ParameterType,
    concrete: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
    strict: bool,
    at: Position<'_>,
) -> bool {
    if matches!(pattern, ParameterType::Unknown) {
        return true;
    }
    unify_at(
        pattern,
        concrete,
        bindings,
        strict,
        NothingBinding::Refused,
        at.nested(),
    )
}

/// STATE/ownership-agnostic type equality (bug-427): two resource types with the same
/// base name (a
/// trailing `STATE T` clause stripped) are compatible, and every non-resource type
/// reduces to plain `==` (`base_resource_name` is the identity there).
fn resource_base_eq(a: &ParameterType, b: &ParameterType) -> bool {
    if a == b {
        return true;
    }
    a.without_state() == b.without_state()
}

/// Substitute `bindings` into a (possibly generic) type `pattern`, producing a
/// concrete type — or `None` if it names a variable that never got bound (a
/// `List OF T` return whose `T` no argument pinned down, e.g. `get` on an `Unknown`).
pub(super) fn substitute(
    pattern: &ParameterType,
    bindings: &BTreeMap<Symbol, ParameterType>,
) -> Option<ParameterType> {
    Some(match pattern {
        ParameterType::Var(name) => bindings.get(name)?.clone(),
        ParameterType::ListOf(elem) => ParameterType::list_of(substitute(elem, bindings)?),
        ParameterType::SetOf(elem) => ParameterType::set_of(substitute(elem, bindings)?),
        ParameterType::MapOf(key, value) => {
            ParameterType::map_of(substitute(key, bindings)?, substitute(value, bindings)?)
        }
        ParameterType::MapEntryOf(key, value) => {
            ParameterType::map_entry_of(substitute(key, bindings)?, substitute(value, bindings)?)
        }
        ParameterType::ResultOf(success) => {
            ParameterType::result_of(substitute(success, bindings)?)
        }
        ParameterType::UserOf(name, args) => ParameterType::UserOf(
            *name,
            args.iter()
                .map(|a| substitute(a, bindings))
                .collect::<Option<Vec<_>>>()?,
        ),
        ParameterType::Func(params, ret, isolated) => {
            let params = params
                .iter()
                .map(|p| substitute(p, bindings))
                .collect::<Option<Vec<_>>>()?;
            let ret = substitute(ret, bindings)?;
            if *isolated {
                ParameterType::func_isolated(params, ret)
            } else {
                ParameterType::func(params, ret)
            }
        }
        // A thread-handle return (only `start` builds one) rebuilds each slot with the
        // bindings unified from the call — the fresh parent `Thread` handle whose
        // `msg`/`res`/`out` echo the worker's, exactly like the `List`/`Map` arms.
        ParameterType::ThreadHandle {
            worker,
            msg,
            res,
            out,
        } => ParameterType::thread_handle(
            *worker,
            substitute(msg, bindings)?,
            substitute(res, bindings)?,
            substitute(out, bindings)?,
        ),
        // A `RES`-wrapped pattern substitutes its inner (descriptors don't build one
        // today, but keep it structural rather than falling through to a bare clone).
        ParameterType::Res(inner) => ParameterType::res(substitute(inner, bindings)?),
        other => other.clone(),
    })
}

/// Whether an overload has a **self-update form** `x = f(x, …)` for a collection `x`
/// (plan-142-A) or a `String` `x` (plan-146-A): its first parameter is, or can be
/// instantiated to, a `List`, `Map` or `Set` — or is a `String` or an
/// `AttributedString` — and its return type can equal that first parameter under one
/// substitution of the signature's type variables. An `Arg(n)` return echoes
/// parameter `n`'s type. The census in `collection::assign::self_update` requires a
/// `SELF_UPDATE_TABLE` row for every function with such an overload.
///
/// Unlike [`unify`], both sides are patterns here — `transform(List OF T, …) AS
/// List OF U` is self-update-shaped because `U` can be `T` — so this runs its own
/// two-sided unification over one binding map.
///
/// Test-only: its one consumer is the census test; the compiler never asks it.
#[cfg(test)]
pub(crate) fn self_update_shaped(imp: &Implementation) -> bool {
    let Some(first) = imp.params.first() else {
        return false;
    };
    let ret = match &imp.return_type {
        ParameterType::Arg(n) => match imp.params.get(*n) {
            Some(p) => &p.ty,
            None => return false,
        },
        other => other,
    };
    let mut bindings = BTreeMap::new();
    if !types_coincide(&first.ty, ret, &mut bindings) {
        return false;
    }
    matches!(
        resolve_bound(strip_res(&first.ty), &bindings),
        ParameterType::ListOf(_) | ParameterType::MapOf(_, _) | ParameterType::SetOf(_)
    ) || matches!(
        resolve_bound(strip_res(&first.ty), &bindings),
        ParameterType::Var(_)
    ) || is_string_self_update_type(strip_res(&first.ty))
}

/// plan-146-A: a `String` (or `astrings::AttributedString`) first parameter whose
/// return type is the same type is a `String` self-update form `s = f(s, …)`.
/// The declared type itself, not a binding: no generic overload's result can be
/// the first argument's `String` type (plan-143 findings, the generic census).
#[cfg(test)]
fn is_string_self_update_type(ty: &ParameterType) -> bool {
    *ty == ParameterType::String || ty.is_named("AttributedString")
}

#[cfg(test)]
fn strip_res(ty: &ParameterType) -> &ParameterType {
    match ty {
        ParameterType::Res(inner) => strip_res(inner),
        other => other,
    }
}

#[cfg(test)]
/// Follow `Var` bindings until an unbound variable or a non-variable type.
fn resolve_bound<'a>(
    ty: &'a ParameterType,
    bindings: &'a BTreeMap<Symbol, ParameterType>,
) -> &'a ParameterType {
    let mut ty = ty;
    while let ParameterType::Var(name) = ty {
        match bindings.get(name) {
            Some(bound) => ty = strip_res(bound),
            None => break,
        }
    }
    ty
}

#[cfg(test)]
fn occurs(name: Symbol, ty: &ParameterType, bindings: &BTreeMap<Symbol, ParameterType>) -> bool {
    match resolve_bound(strip_res(ty), bindings) {
        ParameterType::Var(other) => *other == name,
        ParameterType::ListOf(e) | ParameterType::SetOf(e) | ParameterType::ResultOf(e) => {
            occurs(name, e, bindings)
        }
        ParameterType::MapOf(k, v) | ParameterType::MapEntryOf(k, v) => {
            occurs(name, k, bindings) || occurs(name, v, bindings)
        }
        ParameterType::UserOf(_, args) => args.iter().any(|a| occurs(name, a, bindings)),
        ParameterType::Func(params, ret, _) => {
            params.iter().any(|p| occurs(name, p, bindings)) || occurs(name, ret, bindings)
        }
        _ => false,
    }
}

#[cfg(test)]
/// Two-sided unification: can `a` and `b` (either may hold type variables) be made
/// equal by one substitution? Records the substitution in `bindings`.
fn types_coincide(
    a: &ParameterType,
    b: &ParameterType,
    bindings: &mut BTreeMap<Symbol, ParameterType>,
) -> bool {
    let a = resolve_bound(strip_res(a), bindings).clone();
    let b = resolve_bound(strip_res(b), bindings).clone();
    match (&a, &b) {
        (ParameterType::Var(x), ParameterType::Var(y)) if x == y => true,
        (ParameterType::Var(x), other) | (other, ParameterType::Var(x)) => {
            if occurs(*x, other, bindings) {
                return false;
            }
            bindings.insert(*x, other.clone());
            true
        }
        (ParameterType::ListOf(x), ParameterType::ListOf(y))
        | (ParameterType::SetOf(x), ParameterType::SetOf(y))
        | (ParameterType::ResultOf(x), ParameterType::ResultOf(y)) => {
            types_coincide(x, y, bindings)
        }
        (ParameterType::MapOf(k1, v1), ParameterType::MapOf(k2, v2))
        | (ParameterType::MapEntryOf(k1, v1), ParameterType::MapEntryOf(k2, v2)) => {
            types_coincide(k1, k2, bindings) && types_coincide(v1, v2, bindings)
        }
        (ParameterType::UserOf(n1, a1), ParameterType::UserOf(n2, a2)) => {
            n1 == n2
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2)
                    .all(|(x, y)| types_coincide(x, y, bindings))
        }
        (ParameterType::Func(p1, r1, i1), ParameterType::Func(p2, r2, i2)) => {
            i1 == i2
                && p1.len() == p2.len()
                && p1
                    .iter()
                    .zip(p2)
                    .all(|(x, y)| types_coincide(x, y, bindings))
                && types_coincide(r1, r2, bindings)
        }
        (x, y) => x == y,
    }
}

/// Whether a type mentions any [`ParameterType::Var`] — i.e. it is generic and has no
/// single static nominal type independent of a call's arguments.
pub(crate) fn contains_var(ty: &ParameterType) -> bool {
    match ty {
        // `Var` is arg-dependent; `Arg(_)` echoes an argument verbatim — neither has a
        // single static nominal type independent of the call.
        ParameterType::Var(_) | ParameterType::Arg(_) => true,
        ParameterType::ListOf(elem) | ParameterType::SetOf(elem) => contains_var(elem),
        ParameterType::ResultOf(success) => contains_var(success),
        ParameterType::MapOf(key, value) | ParameterType::MapEntryOf(key, value) => {
            contains_var(key) || contains_var(value)
        }
        ParameterType::UserOf(_, args) => args.iter().any(contains_var),
        ParameterType::Func(params, ret, _) => params.iter().any(contains_var) || contains_var(ret),
        ParameterType::ThreadHandle { msg, res, out, .. } => {
            contains_var(msg) || contains_var(res) || contains_var(out)
        }
        ParameterType::Res(inner) => contains_var(inner),
        _ => false,
    }
}

/// The spelling form of [`resolve_call_typed`], for the per-package
/// registration tests **only**.
///
/// plan-111-C collapsed this query's dual API: `resolve_call_typed` is the one
/// production entry, and the `&[String]`-in/`Option<String>`-out original is
/// gone. What is left is ~140 registration assertions across the per-package
/// modules, and a SPELLING is the right thing for those to assert — a
/// descriptor's job is to resolve to a particular type, and its name is how the
/// test says which. Rewriting each as `.map(|t| t.name().into_owned())` would
/// make them harder to read for no behavioural gain, so the shim is
/// `#[cfg(test)]` and cannot become a second production API.
#[cfg(test)]
pub(crate) fn resolve_call(qualified: &str, arg_types: &[String], strict: bool) -> Option<String> {
    let args: Vec<ParameterType> = arg_types.iter().map(|a| ParameterType::parse(a)).collect();
    resolve_call_typed(qualified, &args, strict).map(|type_| type_.name().into_owned())
}

/// The typed resolution entry (plan-104-C): the same selection as
/// [`resolve_call`] with **no parse** — codegen already holds `ParameterType`
/// arguments. An [`ParameterType::Arg`]-marked return echoes the caller's typed
/// argument (whose `name()` round-trips the original spelling, `RES` markers
/// included). The string [`resolve_call`] is a thin wrapper over the same
/// [`resolved_return_type`] core, so there is exactly one algorithm.
pub(crate) fn resolve_call_typed(
    qualified: &str,
    arg_types: &[ParameterType],
    strict: bool,
) -> Option<ParameterType> {
    resolve_call_typed_with_kinds(qualified, arg_types, strict, &NoTypeKinds)
}

/// [`resolve_call_typed`] with the caller's declared-type oracle, which lets a
/// top-level type variable bound to a union accept one of its variants (bug-988).
pub(crate) fn resolve_call_typed_with_kinds(
    qualified: &str,
    arg_types: &[ParameterType],
    strict: bool,
    kinds: &dyn TypeKinds,
) -> Option<ParameterType> {
    let call = CallShape {
        args: arg_types.to_vec(),
    };
    resolved_return_type(qualified, &call, strict, kinds).map(|return_type| match return_type {
        ParameterType::Arg(n) => arg_types[n].clone(),
        other => other,
    })
}
