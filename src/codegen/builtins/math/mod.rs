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

const MODULE_INTRO: &str = r#"Scalar and vectorized numeric functions and constants"#;
const MODULE_DESC: &str = r#"The `math` package provides the scalar and vectorized (SIMD) numeric functions the
language operator set does not spell — absolute value, min/max/clamp, the rounding
family, square root, the transcendentals (exp/log/trig), power, and a per-thread
pseudo-random generator — together with 14 compile-time constants (`pi`, `e`,
`ln2`, and friends, each in a `Float` and a `Fixed` form).

Every function lowers inline at the call site, like `bits::*`, rather than calling a
runtime helper, and produces identical results on the native and Binary
Representation execution paths. The argument-type-preserving members (`abs`,
`min`/`max`/`clamp`, `sqrt`, the transcendentals, `pow`, `atan2`) return the operand
type; `floor`/`ceil`/`round` exit to `Integer`; `rand` draws an `Integer` (or, over
two `Money` bounds, a `Money`); `seed` returns Nothing.

`math` is a built-in package: `IMPORT math` needs no manifest dependency."#;

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

/// The argument-type-preserving unary shape: a member accepting a single numeric
/// scalar (each of `scalars`) or its `List OF` form (each of `lists`) and echoing
/// the operand type (`Arg(0)`). `errors` is declared on every overload.
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
            errors.to_vec(),
            lower,
        ));
    }
    for ty in scalars {
        impls.push(overload(
            vec![req("value", &[], ty.clone(), value_desc)],
            ParameterType::Arg(0),
            errors.to_vec(),
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
pub(crate) fn rounding(
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
            errors.to_vec(),
            lower,
        ));
    }
    for ty in scalars {
        impls.push(overload(
            vec![req("value", &[], ty.clone(), value_desc)],
            ParameterType::Integer,
            errors.to_vec(),
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
    errors: &[&'static str],
    lower: AbiInline,
    pkg: &mut RegistryPackage,
) {
    // List overloads first (see `preserving_unary`).
    let mut impls = Vec::new();
    for ty in lists {
        let list = ParameterType::list_of(ty.clone());
        impls.push(overload(
            vec![
                req(p0.0, p0.1, list.clone(), p0.2),
                req(p1.0, p1.1, list, p1.2),
            ],
            ParameterType::Arg(0),
            errors.to_vec(),
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
            errors.to_vec(),
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
}
