//! The built-in `math` package (clean-room registry migration).
//!
//! `math` provides scalar and vectorized (SIMD) numeric functions — `abs`,
//! `min`/`max`/`clamp`, the rounding family (`floor`/`ceil`/`round`), `sqrt`, the
//! transcendentals (`exp`/`log`/`log10`/`sin`/`cos`/`tan`/`asin`/`acos`/`atan`/
//! `atan2`), `pow`, and the per-thread PCG64 generator (`rand`/`seed`) — plus 14
//! compile-time constants (`pi`, `e`, `ln2`, …, seven `Float` and seven `Fixed`).
//!
//! Every callable lowers **inline** at the call site (a `Body::abi_inline`
//! self-lowering intrinsic — no runtime helper, no source companion). Each
//! member is enumerated as concrete-type overloads that reproduce the legacy
//! `resolve_call` acceptance and return types byte-for-byte: an argument-type
//! preserving member echoes `Arg(0)` (its operand's type), `floor`/`ceil`/`round`
//! return `Integer` (or `List OF Integer`), `rand` returns `Integer`/`Money`, and
//! `seed` returns `Nothing`. Per-member errors are declared on the fallible
//! overloads so the inline-`TRAP` fallibility census reads them off registry data
//! (`native_member_declares_error`) rather than a `math.` name predicate.
//!
//! The two members that call a STAYS-core helper are `pow`/`atan2` (Float scalar
//! `pow` shares `emit_pow_scalar`/`lower_pow_array` with the `^` operator) and
//! `rand`/`seed` (the PCG64 routines `_mfb_rng_next`/`_mfb_rng_seed` stay core,
//! referenced by symbol). The shared call-site lowering carrier (`lower_math_call`,
//! including the vectorized/SIMD lowerings) lives in [`gen_math`], with the fdlibm
//! `pow`/`fmod` kernels in [`gen_pow`]/[`gen_fmod`] and the PCG64 generator in
//! [`gen_rng_pcg64`].
//!
//! `math.sqrt` / `math.clamp` stay callable **by name**: `builder_vector_inline`
//! emits them as `NirValue::Call`, so they resolve through
//! `try_abi_inline_lower` on the full `"math.sqrt"` spelling.
//!
//! Man/spec citation anchors (the `math/*` man pages and §13 spec ground their
//! per-member facts here): `MATH` (the descriptor authority for the 21 callables),
//! `is_math_call` (membership — a call is a math call iff the registry's
//! `owning_package` is `"math"`), `is_math_constant` / `constant_type_name` /
//! `constant_value` (the 14 constants, now registered via `add_constant`),
//! `call_param_names` (parameter names + aliases carried on each `Parameter`), and
//! `is_numeric` / `is_numeric_list` / `clamp_list` (the per-member numeric
//! acceptance now enumerated as concrete-type overloads below).

use crate::codegen::registry::{
    AbiInline, Body, DefaultValue, Implementation, Parameter, Registry, RegistryConstant,
    RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

// Man/spec citation anchors (relocated from the deleted `src/builtins/math.rs`). The
// per-member numeric acceptance the legacy helper predicates `is_numeric`,
// `is_numeric_list`, `any_numeric_list`, `one_float_or_fixed`, `one_floatish_list`,
// `two_same_float_or_fixed`, and `clamp_list` expressed is now enumerated as the
// concrete-type overloads in the `func_*.rs` files; the constant helpers
// `is_math_constant`, `constant_type_name`, and `constant_value` are now `add_constant`
// data; `call_param_names` is carried on each `Parameter` (name + aliases); `is_math_call`
// is `owning_package == "math"`; `RAND` and `SEED` are the `rand`/`seed` members; and
// `MATH` is this descriptor authority for the 21 callables.

mod func_abs;
mod func_acos;
mod func_asin;
mod func_atan;
mod func_atan2;
mod func_ceil;
mod func_clamp;
mod func_cos;
mod func_exp;
mod func_floor;
mod func_log;
mod func_log10;
mod func_max;
mod func_min;
mod func_pow;
mod func_rand;
mod func_round;
mod func_seed;
mod func_sin;
mod func_sqrt;
mod func_tan;

pub(crate) mod gen_fmod;
pub(crate) mod gen_math;
pub(crate) use gen_math::*;
pub(crate) mod gen_pow;
pub(crate) mod gen_rng_pcg64;
pub(crate) use gen_rng_pcg64::*;

const MODULE_INTRO: &str = r#"Numeric functions and constants"#;
const MODULE_DESC: &str = r#"The `math` package provides the numeric functions the language operator set does
not spell — absolute value, min/max/clamp, the rounding family, square root, the
transcendentals (exp/log/trig), power, and a per-thread random sequence —
together with 14 constants.

Most members take a single number and give back the same type they were given:
`abs`, `min`/`max`/`clamp`, `sqrt`, the transcendentals, `pow`, and `atan2`. Their
list forms differ: `abs`, `min`/`max`/`clamp`, `sqrt`, `log`/`log10` and the
rounding family also accept a `List OF Fixed`, while `exp`, the trigonometric
functions, `pow`, and `atan2` accept only a `List OF Float`. Each function's page
lists its forms. `floor`/`ceil`/`round` give back an
`Integer`, or a `List OF Integer` for their list forms. `rand` gives an `Integer`, or a `Money` when called with two `Money`
bounds; `seed` returns nothing.

**Constants.** Each is written `math::<name>` and needs no call parentheses.
Every one comes in a `Float` form and a `Fixed` form, so you can stay in whichever
type your program already uses: `math::pi` and `math::piFixed`, `math::pi2`
(pi/2), `math::pi4` (pi/4), `math::twoOverPi` (2/pi), `math::e`, `math::ln2` and
`math::ln10`, each with its `Fixed` twin. The Constants table below gives every
value as written. A `Fixed` form holds the nearest value `Fixed` can represent,
which agrees with the written value to at least nine significant digits
(`math::piFixed` is `3.141592653701081`).

A constant has no page of its own: `mfb man math pi` will not resolve, because
`pi` is a value rather than a function.

`math` is a built-in package: `IMPORT math` needs no manifest dependency.

A power, a square root, and a random sequence replayed from the same seed:

```
IMPORT io
IMPORT math

SUB main()
  io::print(toString(math::pow(2.0, 10.0)))
  io::print(toString(math::sqrt(math::pi)))
  math::seed(42)
  LET first AS Integer = math::rand(1, 6)
  math::seed(42)
  io::print(toString(first = math::rand(1, 6)))
END SUB
```

prints:

```
1024.00
1.77
TRUE
```"#;

/// One required parameter with optional keyword aliases and no default. `desc` is
/// the man page's Parameters-table prose — these descriptors ARE the man pages
/// (`src/cli/man.rs` renders them), so an empty one renders as an empty cell.
pub(crate) fn req(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: ParameterType,
    desc: &'static str,
) -> Parameter {
    Parameter {
        name,
        desc,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// A single concrete-type overload lowering inline through `lower`.
pub(crate) fn overload(
    params: Vec<Parameter>,
    return_type: ParameterType,
    errors: Vec<&'static str>,
    lower: AbiInline,
) -> Implementation {
    Implementation {
        params,
        return_type,
        errors,
        body: Body::abi_inline(lower),
    }
}

/// Register the `math` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("math", MODULE_INTRO, MODULE_DESC);

    // The 14 compile-time constants (`is_math_constant` / `constant_type_name` /
    // `constant_value`), seven `Float` and seven `Fixed`. Each folds to its literal
    // at the point of use.
    for (name, type_name, value) in CONSTANTS {
        pkg.add_constant(RegistryConstant {
            name,
            type_name,
            value: Some(value),
            components: None,
            message: None,
            symbol: None,
        });
    }

    func_abs::register(&mut pkg);
    func_min::register(&mut pkg);
    func_max::register(&mut pkg);
    func_clamp::register(&mut pkg);
    func_floor::register(&mut pkg);
    func_ceil::register(&mut pkg);
    func_round::register(&mut pkg);
    func_sqrt::register(&mut pkg);
    func_pow::register(&mut pkg);
    func_exp::register(&mut pkg);
    func_log::register(&mut pkg);
    func_log10::register(&mut pkg);
    func_sin::register(&mut pkg);
    func_cos::register(&mut pkg);
    func_tan::register(&mut pkg);
    func_asin::register(&mut pkg);
    func_acos::register(&mut pkg);
    func_atan::register(&mut pkg);
    func_atan2::register(&mut pkg);
    func_rand::register(&mut pkg);
    func_seed::register(&mut pkg);

    r.add_package(pkg);
}

/// The 14 constants: `(member, type, literal)`. Both the `Float` and the `Fixed`
/// form fold to the same decimal shorthand (the nearest representable value).
const CONSTANTS: &[(&str, &str, &str)] = &[
    ("pi", "Float", "3.141592653589793"),
    ("piFixed", "Fixed", "3.141592653589793"),
    ("twoOverPi", "Float", "0.6366197723675814"),
    ("twoOverPiFixed", "Fixed", "0.6366197723675814"),
    ("pi2", "Float", "1.5707963267948966"),
    ("pi2Fixed", "Fixed", "1.5707963267948966"),
    ("pi4", "Float", "0.7853981633974483"),
    ("pi4Fixed", "Fixed", "0.7853981633974483"),
    ("e", "Float", "2.718281828459045"),
    ("eFixed", "Fixed", "2.718281828459045"),
    ("ln2", "Float", "0.6931471805599453"),
    ("ln2Fixed", "Fixed", "0.6931471805599453"),
    ("ln10", "Float", "2.302585092994046"),
    ("ln10Fixed", "Fixed", "2.302585092994046"),
];

/// Errors declared per OPERAND type rather than smeared across every overload.
///
/// bug-617: one shared `errors` vector per member put errors in the Errors table of
/// overloads whose lowering cannot raise them — `math::asin` promised both
/// `ErrFloatDomain` and `ErrInvalidArgument` on all three forms, when the `Float`
/// forms raise only the first (`77050012`) and the `Fixed` form only the second
/// (`77050002`). That is compiler data, not prose: `mfb spec language error-model`
/// §8.6 rule 11 has the compiler read it.
///
/// Each entry is `(scalar operand type, the errors overloads over that type can
/// raise)`. It applies to BOTH that type's scalar overload and its `List OF` form,
/// because a member's list form runs the same kernel per lane — measured for every
/// member corrected here, including the SIMD scalar tail.
///
/// An absent type declares NOTHING, and that is load-bearing, not an oversight:
/// `math::atan2` on a `Fixed` has no raise path at all (`emit_fixed_atan2` contains
/// zero raise sites), so it must be expressible as the empty set rather than
/// inheriting a shared list.
///
/// The declaration rule is **what the lowering can EMIT**, not what a program can
/// currently trigger. `ErrFloatNaN` stays on the `Float` forms of `atan`/`tan`/`sin`/
/// `cos` even though a NaN operand is unconstructible today (every NaN/Inf-producing
/// expression traps at its own observation boundary first), because the kernel really
/// does emit that check — and because a member whose every overload declared nothing
/// would flip `native_member_declares_error` to `Some(false)` and let
/// `inline_builtin_is_infallible` DELETE a live `TRAP` handler, which is the
/// bug-486 / bug-533 hazard.
pub(crate) type ErrorsByType<'a> = &'a [(ParameterType, &'static [&'static str])];

/// The errors an overload over `ty` declares: the member-wide `shared` set plus
/// whatever `by_type` names for that operand type, deduped (a member can legitimately
/// list an error in both, e.g. an error intrinsic to one element type that is also the
/// list forms' length-mismatch error).
fn errors_for(
    ty: &ParameterType,
    shared: &[&'static str],
    by_type: ErrorsByType,
) -> Vec<&'static str> {
    let mut declared: Vec<&'static str> = shared.to_vec();
    for (key, errors) in by_type {
        if key == ty {
            for error in *errors {
                if !declared.contains(error) {
                    declared.push(error);
                }
            }
        }
    }
    declared
}

/// The argument-type-preserving unary shape: a member accepting a single numeric
/// scalar (each of `scalars`) or its `List OF` form (each of `lists`) and echoing
/// the operand type (`Arg(0)`). `errors` is declared on every overload — correct only
/// for a member whose every form really can raise every listed error. A member whose
/// forms differ (most of the fallible ones) uses
/// [`preserving_unary_typed_errors`] instead.
pub(crate) fn preserving_unary(
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    expected: &'static str,
    value_desc: &'static str,
    scalars: &[ParameterType],
    lists: &[ParameterType],
    errors: &[&'static str],
    lower: AbiInline,
    pkg: &mut RegistryPackage,
) {
    preserving_unary_typed_errors(
        name,
        intro,
        desc,
        example,
        expected,
        value_desc,
        scalars,
        lists,
        errors,
        &[],
        lower,
        pkg,
    );
}

/// [`preserving_unary`], with the errors partitioned by operand type ([`ErrorsByType`]).
///
/// This replaces bug-615's `extra: Option<(ParameterType, &[&str])>` hook, which was
/// additive and scalar-only: it could ADD `ErrOverflow` to `math::tan`'s `Fixed`
/// scalar form, but it could not SUBTRACT `ErrInvalidArgument` from the `Float` forms,
/// and it never reached the `List OF` overloads at all. bug-617 needs both directions
/// on both arms, so the partition is now the primary mechanism and `shared` is the
/// residue.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preserving_unary_typed_errors(
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    expected: &'static str,
    value_desc: &'static str,
    scalars: &[ParameterType],
    lists: &[ParameterType],
    shared: &[&'static str],
    by_type: ErrorsByType,
    lower: AbiInline,
    pkg: &mut RegistryPackage,
) {
    // List overloads are registered BEFORE the scalar overloads: lenient overload
    // resolution (return-type inference) coarsely accepts a scalar pattern against a
    // `List OF` concrete, so a scalar-first order would echo the wrong shape for a
    // list argument — mirror the legacy `resolve_call`, which checked its array arms
    // first. (A `ListOf` pattern never matches a scalar concrete, so scalar calls are
    // unaffected.)
    let mut impls = Vec::new();
    for ty in lists {
        impls.push(overload(
            vec![req(
                "value",
                &[],
                ParameterType::list_of(ty.clone()),
                value_desc,
            )],
            ParameterType::Arg(0),
            // Keyed on the ELEMENT type: `List OF Float` runs the same kernel per
            // lane as the `Float` scalar form, so it raises the same errors.
            errors_for(ty, shared, by_type),
            lower,
        ));
    }
    for ty in scalars {
        impls.push(overload(
            vec![req("value", &[], ty.clone(), value_desc)],
            ParameterType::Arg(0),
            errors_for(ty, shared, by_type),
            lower,
        ));
    }
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        expected_arguments: Some(expected),
        internal_only: false,
        implementations: impls,
    });
}

/// The rounding shape (`floor`/`ceil`/`round`): a single numeric scalar (each of
/// `scalars`) returns `Integer`, a `List OF` (each of `lists`) returns `List OF
/// Integer` — a deliberate dimension exit, so this is not `Arg(0)`.
///
/// Errors are partitioned by operand type ([`ErrorsByType`]) for the same reason the
/// unary shape's are (bug-617): only the `Float` forms range-check the rounded result
/// against `Integer` (`emit_float_rounding_integer_range_check`), while the `Fixed`
/// and `Money` paths always fit and contain no raise at all.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rounding(
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    expected: &'static str,
    value_desc: &'static str,
    scalars: &[ParameterType],
    lists: &[ParameterType],
    shared: &[&'static str],
    by_type: ErrorsByType,
    lower: AbiInline,
    pkg: &mut RegistryPackage,
) {
    // List overloads first (see `preserving_unary`): the array form returns
    // `List OF Integer`, the scalar form `Integer`, so a scalar-first order would
    // mis-infer a list argument's result as the scalar `Integer`.
    let mut impls = Vec::new();
    for ty in lists {
        impls.push(overload(
            vec![req(
                "value",
                &[],
                ParameterType::list_of(ty.clone()),
                value_desc,
            )],
            ParameterType::list_of(ParameterType::Integer),
            errors_for(ty, shared, by_type),
            lower,
        ));
    }
    for ty in scalars {
        impls.push(overload(
            vec![req("value", &[], ty.clone(), value_desc)],
            ParameterType::Integer,
            errors_for(ty, shared, by_type),
            lower,
        ));
    }
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        expected_arguments: Some(expected),
        internal_only: false,
        implementations: impls,
    });
}

/// The argument-type-preserving binary shape (`min`/`max`/`pow`/`atan2`): two
/// same-type numeric scalars `(T, T)` (each `T` in `scalars`) or two same-type
/// `List OF T` (each `T` in `lists`), echoing `Arg(0)`. `p0`/`p1` are the two
/// parameters' `(name, aliases)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preserving_binary(
    name: &'static str,
    intro: &'static str,
    desc: &'static str,
    example: &'static str,
    expected: &'static str,
    p0: (&'static str, &'static [&'static str], &'static str),
    p1: (&'static str, &'static [&'static str], &'static str),
    scalars: &[ParameterType],
    lists: &[ParameterType],
    shared: &[&'static str],
    // bug-617: the LIST forms of a binary member take two lists and raise
    // `ErrInvalidArgument` when their lengths differ (`lower_simd_binary`'s length
    // check). The scalar forms have no length to mismatch, so declaring it on them
    // told a reader of `math::max(3, 5)` to handle an error it cannot get. This
    // channel is separate from `by_type` because it is a property of the ARITY
    // shape, not of the operand type.
    list_only: &[&'static str],
    by_type: ErrorsByType,
    lower: AbiInline,
    pkg: &mut RegistryPackage,
) {
    // List overloads first (see `preserving_unary`).
    let mut impls = Vec::new();
    for ty in lists {
        let list = ParameterType::list_of(ty.clone());
        let mut declared = errors_for(ty, shared, by_type);
        for error in list_only {
            if !declared.contains(error) {
                declared.push(error);
            }
        }
        impls.push(overload(
            vec![
                req(p0.0, p0.1, list.clone(), p0.2),
                req(p1.0, p1.1, list, p1.2),
            ],
            ParameterType::Arg(0),
            declared,
            lower,
        ));
    }
    for ty in scalars {
        impls.push(overload(
            vec![
                req(p0.0, p0.1, ty.clone(), p0.2),
                req(p1.0, p1.1, ty.clone(), p1.2),
            ],
            ParameterType::Arg(0),
            errors_for(ty, shared, by_type),
            lower,
        ));
    }
    pkg.add_function(RegistryFunction {
        name,
        intro,
        desc,
        example,
        expected_arguments: Some(expected),
        internal_only: false,
        implementations: impls,
    });
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn math_registered_on_the_clean_room_registry() {
        let pkg = registry().resolve_package("math").expect("math package");
        assert_eq!(pkg.functions().len(), 21);
        // math injects no source (no records/unions/enums/Mfb bodies/helpers).
        assert!(pkg.get_mfb().is_empty());
        // No value types, no resources.
        assert!(!registry().is_builtin_type("math"));
        // The 14 constants are registered.
        assert!(registry::is_package_constant("math.pi"));
        assert_eq!(
            registry::constant_type_name("math.pi"),
            Some(crate::types::ParameterType::Float)
        );
        assert_eq!(
            registry::constant_type_name("math.piFixed"),
            Some(crate::types::ParameterType::Fixed)
        );
        assert_eq!(
            registry::constant_value("math.pi"),
            Some("3.141592653589793")
        );
    }

    #[test]
    fn every_member_owns_a_self_lowering_inline_body() {
        for name in [
            "abs", "min", "max", "clamp", "floor", "ceil", "round", "sqrt", "pow", "exp", "log",
            "log10", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "rand", "seed",
        ] {
            let q = format!("math.{name}");
            assert_eq!(registry().owning_package(&q), Some("math"), "{name}");
            assert!(
                registry::abi_inline_lower(&q).is_some(),
                "{name} should have a Body::abi_inline lowering"
            );
        }
    }

    #[test]
    fn return_types_reproduce_the_legacy_resolver() {
        let r = |name: &str, args: &[&str]| {
            registry::resolve_call(
                name,
                &args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                true,
            )
        };
        // Argument-type-preserving scalars echo the operand type.
        assert_eq!(r("math.abs", &["Integer"]).as_deref(), Some("Integer"));
        assert_eq!(r("math.abs", &["Money"]).as_deref(), Some("Money"));
        assert_eq!(r("math.sqrt", &["Float"]).as_deref(), Some("Float"));
        assert_eq!(r("math.sqrt", &["Fixed"]).as_deref(), Some("Fixed"));
        // Transcendentals reject Integer and Money (Float|Fixed only).
        assert_eq!(r("math.sqrt", &["Integer"]), None);
        assert_eq!(r("math.exp", &["Integer"]), None);
        assert_eq!(r("math.exp", &["Money"]), None);
        assert_eq!(r("math.sqrt", &["String"]), None);
        // Arrays echo, transcendental arrays restrict element type.
        assert_eq!(
            r("math.abs", &["List OF Integer"]).as_deref(),
            Some("List OF Integer")
        );
        assert_eq!(r("math.abs", &["List OF Money"]), None);
        assert_eq!(
            r("math.exp", &["List OF Float"]).as_deref(),
            Some("List OF Float")
        );
        assert_eq!(r("math.exp", &["List OF Fixed"]), None);
        // Rounding exits to Integer.
        assert_eq!(r("math.floor", &["Float"]).as_deref(), Some("Integer"));
        assert_eq!(r("math.floor", &["Money"]).as_deref(), Some("Integer"));
        assert_eq!(
            r("math.round", &["List OF Fixed"]).as_deref(),
            Some("List OF Integer")
        );
        assert_eq!(r("math.floor", &["Integer"]), None);
        // min/max/clamp same-type; pow/atan2 same float-or-fixed.
        assert_eq!(
            r("math.min", &["Integer", "Integer"]).as_deref(),
            Some("Integer")
        );
        assert_eq!(r("math.min", &["Integer", "Float"]), None);
        assert_eq!(r("math.pow", &["Float", "Float"]).as_deref(), Some("Float"));
        assert_eq!(r("math.pow", &["Float", "Fixed"]), None);
        assert_eq!(r("math.pow", &["Integer", "Integer"]), None);
        // rand / seed.
        assert_eq!(
            r("math.rand", &["Integer", "Integer"]).as_deref(),
            Some("Integer")
        );
        assert_eq!(
            r("math.rand", &["Money", "Money"]).as_deref(),
            Some("Money")
        );
        assert_eq!(r("math.rand", &["Float", "Float"]), None);
        assert_eq!(r("math.seed", &["Integer"]).as_deref(), Some("Nothing"));
        assert_eq!(r("math.seed", &["Float"]), None);
    }

    #[test]
    fn fallibility_census_reads_registry_data() {
        // sqrt declares ErrFloatDomain -> fallible; seed declares none -> infallible.
        assert_eq!(
            registry::native_member_declares_error("math.sqrt"),
            Some(true)
        );
        assert_eq!(
            registry::native_member_declares_error("math.seed"),
            Some(false)
        );
    }

    /// The per-overload declarations bug-617 established, pinned as a table.
    ///
    /// Each row is `(member, error, the overload NUMBERS that declare it)` — the
    /// numbers being the ones `mfb man` renders, i.e. list overloads first (in the
    /// caller's slice order) then scalars. Every row was measured against the real
    /// lowering by probe, not read off the descriptor: a probe that raised gives the
    /// error code, and one that returned a value proves the overload has no path to
    /// it.
    ///
    /// The rule these encode is **what the lowering can EMIT**, which is not always
    /// what a program can currently trigger. `ErrFloatNaN` stays on the `Float` forms
    /// because the kernel emits that check, even though no NaN operand is
    /// constructible today (every NaN/Inf-producing expression traps at its own
    /// observation boundary first).
    #[test]
    fn each_overload_declares_only_the_errors_its_own_lowering_can_raise() {
        // (member, error, overload numbers — 1-based, as `mfb man` renders them)
        let expected: &[(&str, &str, &[usize])] = &[
            // 1 List OF Integer, 2 List OF Float, 3 List OF Fixed,
            // 4 Integer, 5 Float, 6 Fixed, 7 Money. The Float forms (2, 5) clear the
            // sign bit and cannot fail.
            ("abs", "ErrOverflow", &[1, 3, 4, 6, 7]),
            // 1 List OF Float, 2 Float, 3 Fixed — a disjoint split.
            ("acos", "ErrFloatDomain", &[1, 2]),
            ("acos", "ErrInvalidArgument", &[3]),
            ("asin", "ErrFloatDomain", &[1, 2]),
            ("asin", "ErrInvalidArgument", &[3]),
            // 1 List OF Float, 2 List OF Fixed, 3 Float, 4 Fixed.
            ("sqrt", "ErrFloatDomain", &[1, 3]),
            ("sqrt", "ErrInvalidArgument", &[2, 4]),
            ("log", "ErrFloatDomain", &[1, 3]),
            ("log", "ErrInvalidArgument", &[2, 4]),
            ("log10", "ErrFloatDomain", &[1, 3]),
            ("log10", "ErrInvalidArgument", &[2, 4]),
            // 1 List OF Float, 2 List OF Fixed, 3 Float, 4 Fixed, 5 Money — only the
            // Float family range-checks the rounded result against `Integer`.
            ("floor", "ErrOverflow", &[1, 3]),
            ("ceil", "ErrOverflow", &[1, 3]),
            ("round", "ErrOverflow", &[1, 3]),
            // ErrOverflow is Fixed-only; the Float kernel signals an unrepresentable
            // result as ErrFloatInf instead and has no overflow path.
            ("exp", "ErrFloatInf", &[1, 2]),
            ("exp", "ErrOverflow", &[3]),
            // tan lost ErrFloatInf and ErrInvalidArgument entirely — no overload
            // could raise either.
            ("tan", "ErrFloatNaN", &[1, 2]),
            ("tan", "ErrOverflow", &[3]),
            ("atan", "ErrFloatNaN", &[1, 2]),
            ("sin", "ErrFloatNaN", &[1, 2]),
            ("cos", "ErrFloatNaN", &[1, 2]),
            // Binary members: the length-mismatch check belongs to the LIST forms.
            // max/min: 1..3 are the lists, 4..7 the scalars.
            ("max", "ErrInvalidArgument", &[1, 2, 3]),
            ("min", "ErrInvalidArgument", &[1, 2, 3]),
            // pow/atan2: 1 is the only list form, 2 Float, 3 Fixed.
            ("pow", "ErrInvalidArgument", &[1, 3]),
            ("pow", "ErrOverflow", &[3]),
            ("pow", "ErrFloatInf", &[1, 2]),
            ("pow", "ErrFloatNaN", &[1, 2]),
            ("atan2", "ErrInvalidArgument", &[1]),
            ("atan2", "ErrFloatNaN", &[1, 2]),
            // clamp's uniform declaration was already correct: every form guards
            // `low > high` with the same bare check.
            ("clamp", "ErrInvalidArgument", &[1, 2, 3, 4, 5, 6, 7]),
        ];

        let package = registry().resolve_package("math").expect("math package");
        for (member, error, want) in expected {
            let function = package.function(member).expect(member);
            let got: Vec<usize> = function
                .implementations
                .iter()
                .enumerate()
                .filter(|(_, implementation)| implementation.errors.contains(error))
                .map(|(index, _)| index + 1)
                .collect();
            assert_eq!(
                got, *want,
                "math::{member} declares {error} on overloads {got:?}, expected {want:?}"
            );
        }
    }

    /// bug-617 REMOVED declarations, so the hazard to check is the opposite of the
    /// one it fixes: `native_member_declares_error` is a member-level `any` over the
    /// overloads, and it feeds `inline_builtin_is_infallible`. A member whose every
    /// overload ended up declaring nothing would be judged infallible and would have
    /// its live `TRAP` handlers DELETED — the bug-486 / bug-533 failure.
    ///
    /// Every member below keeps at least one declaring overload, so no fallibility
    /// verdict moved. `atan2` is the closest call: its `Fixed` overload now declares
    /// nothing at all, and only the two `Float` forms hold the member fallible.
    #[test]
    fn no_member_lost_its_fallibility_when_the_declarations_were_narrowed() {
        for member in [
            "abs", "acos", "asin", "atan", "atan2", "ceil", "clamp", "cos", "exp", "floor", "log",
            "log10", "max", "min", "pow", "round", "sin", "sqrt", "tan",
        ] {
            let qualified = format!("math.{member}");
            assert_eq!(
                registry::native_member_declares_error(&qualified),
                Some(true),
                "math::{member} became infallible — narrowing its per-overload error \
                 lists must never empty the member, or `inline_builtin_is_infallible` \
                 will delete a live TRAP handler"
            );
        }
        // `rand` is fallible too, and untouched by bug-617: both its overloads
        // guard `min <= max` with the same `ErrInvalidArgument`, which is already a
        // uniform and correct declaration.
        assert_eq!(
            registry::native_member_declares_error("math.rand"),
            Some(true)
        );
        // `seed` is the one genuinely total member.
        assert_eq!(
            registry::native_member_declares_error("math.seed"),
            Some(false)
        );
    }
}
