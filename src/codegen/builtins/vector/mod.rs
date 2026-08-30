//! The built-in `vector` package (clean-room registry migration).
//!
//! `vector` provides nine fixed-width math-vector value **records** — `Float2/3/4`,
//! `Fixed2/3/4`, `Integer2/3/4` — and a set of overloaded geometry / utility / 2D
//! function members and record constants over them. Every member dispatches by its
//! argument record type onto a type-specific internal FUNC in the source companion
//! (`vector.length(Float3)` → `#vector_length_float3`); the descriptor models each
//! member as one `RegistryFunction` whose per-type overloads each carry a
//! [`Body::Rewrite`] to that FUNC, so the registry's own `select`/`rewrite_target`
//! reproduces the pre-migration `VectorResolver::implementation_name` with no custom
//! resolver. The nine records are registered via [`RegistryPackage::add_record`] with
//! element-typed props so the record-constant path (`registry_record_constant` in
//! `ir/lower.rs`) reads each field's element type in declaration order; the FUNC
//! bodies ride each member's `func_*.rs` descriptor as `Body::mfb` (the private
//! isqrt/angleFixed/toString bodies as `helper_*.rs` `add_helper` chunks).
//!
//! The SIMD inline-lowering **carrier** stays shared in
//! `src/codegen/builtins/vector/builder_vector_inline.rs` (the `VECTOR_NATIVE_MARKER`
//! register-native side-table + `try_inline_vector_op`): it is a codegen-wide
//! escape-boundary hook wired into `CodeBuilder`, not a per-call lowering, and it
//! keys on the `#vector_<op>_<type>` rewrite targets this descriptor produces. Its
//! `math.sqrt` / `math.clamp` call emissions resolve through the migrated `math`
//! registry package by name.
//!
//! Man/spec citation anchors (relocated from the deleted `src/builtins/vector.rs`):
//! `VECTOR` (the descriptor authority for the 19 function members and nine records),
//! `resolve_call` (return-type / overload selection, now the registry's `select`),
//! `same_vector` (the same-type argument constraint expressed as identical overload
//! param types), `is_builtin_type` (the nine records recognized by the registry type
//! query), `uses_package` (import-triggered source injection, now
//! `RegistryPackage::is_imported_by` / `augment_project`), `call_param_names`
//! (parameter names + aliases carried on each `Parameter`), `tostring_override_target`
//! (the nine `toString(VecN)` overrides registered via `add_override`), and
//! `constant_components` (the 42 record constants registered via `add_constant`).

use crate::codegen::registry::{
    registry, Body, DefaultValue, Implementation, Parameter, RecordProp, Registry,
    RegistryConstant, RegistryOverride, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

mod func_abs;
mod func_angle;
mod func_clamp_length;
mod func_cross;
mod func_distance;
mod func_dot;
mod func_length;
mod func_lerp;
mod func_lerp_unclamped;
mod func_max;
mod func_min;
mod func_normalize;
mod func_perpendicular;
mod func_project;
mod func_reflect;
mod func_reject;
mod func_rotate_2d;
mod func_scale;
mod func_slerp;

mod helper_angle_fixed_integer2;
mod helper_angle_fixed_integer3;
mod helper_angle_fixed_integer4;
mod helper_isqrt_floor;
mod helper_isqrt_round;
mod helper_to_string_fixed2;
mod helper_to_string_fixed3;
mod helper_to_string_fixed4;
mod helper_to_string_float2;
mod helper_to_string_float3;
mod helper_to_string_float4;
mod helper_to_string_integer2;
mod helper_to_string_integer3;
mod helper_to_string_integer4;

const INTRO: &str =
    r#"Fixed-width math vectors (Float/Fixed/Integer, 2-4D) and geometry over them"#;
const DESC: &str = r#"The `vector` package provides nine fixed-width math-vector value records —
`Float2`/`Float3`/`Float4`, `Fixed2`/`Fixed3`/`Fixed4`, and
`Integer2`/`Integer3`/`Integer4` — together with overloaded geometry, utility, and
2D functions over them (`length`, `normalize`, `distance`, `dot`, `cross`,
`reflect`, `project`, `reject`, `angle`, `lerp`/`lerp_unclamped`/`slerp`,
`clamp_length`, `scale`, `min`/`max`, `abs`, `perpendicular`, `rotate_2d`) and a set
of record constants (`zero`/`one`/`up`/`right`/`forward` in each type).

Each function is overloaded by the exact argument record type: a member takes a
vector of one of the nine types and returns either that type or its scalar element
type. Algebraic operations are correctly rounded on `Float`, use Q32.32 fixed-point
on `Fixed`, and round half away from zero on `Integer`. `vector` is a built-in
package written in MFBASIC over the intrinsic `math` package, so `IMPORT vector`
needs no manifest dependency."#;

// The nine value-record type names, grouped by dimension. VEC_TYPES fixes the
// registration order (2D then 3D then 4D per element, matching the companion).
const VEC_TYPES: &[&str] = &[
    "Float2", "Float3", "Float4", "Fixed2", "Fixed3", "Fixed4", "Integer2", "Integer3", "Integer4",
];
const VEC2_TYPES: &[&str] = &["Float2", "Fixed2", "Integer2"];
const VEC3_TYPES: &[&str] = &["Float3", "Fixed3", "Integer3"];
const VEC4_TYPES: &[&str] = &["Float4", "Fixed4", "Integer4"];

/// The scalar element `ParameterType` of a vector type (`Float3` → `Float`).
fn element_of(ty: &str) -> ParameterType {
    if ty.starts_with("Float") {
        ParameterType::Float
    } else if ty.starts_with("Fixed") {
        ParameterType::Fixed
    } else {
        ParameterType::Integer
    }
}

/// The `__vector_<member>_<type>` internal FUNC a call over `ty` rewrites to, as a
/// leaked `&'static str` (the registry is built once behind a `OnceLock`, so the leak
/// is a bounded one-time allocation — the same idiom the deprecated registry boundary
/// helpers use).
fn rewrite(member: &str, ty: &str) -> &'static str {
    Box::leak(format!("__vector_{member}_{}", ty.to_ascii_lowercase()).into_boxed_str())
}

/// One required parameter with optional keyword aliases.
fn param(name: &'static str, aliases: &'static [&'static str], ty: ParameterType) -> Parameter {
    Parameter {
        name,
        desc: "",
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

fn imp(
    params: Vec<Parameter>,
    return_type: ParameterType,
    errors: Vec<&'static str>,
    body: Body,
) -> Implementation {
    Implementation {
        params,
        return_type,
        errors,
        body,
    }
}

/// The argument-arrangement of a member's overloads. Each shape is enumerated over the
/// member's applicable types (all nine, or the three 2D types) to reproduce the legacy
/// `resolve_call` acceptance and `implementation_name` targets exactly.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    /// `(v AS T_N)` → scalar element `T`. `length`.
    UnaryScalar,
    /// `(v AS T_N)` → `T_N`. `normalize`, `abs`.
    UnaryVector,
    /// `(a AS T_N, b AS T_N)` → scalar element `T`. `distance`, `dot`, `angle`.
    BinaryScalar,
    /// `(a AS T_N, b AS T_N)` → `T_N`. `reflect`, `project`, `reject`, `scale`, `min`, `max`.
    BinaryVector,
    /// `(a AS T_N, b AS T_N, t AS Float)` → `T_N`. `lerp`, `lerp_unclamped`, `slerp`.
    Lerp,
    /// `(v AS T_N, max AS T)` → `T_N`. `clamp_length` (the scalar max is the element type).
    ClampLength,
    /// 2D-only `(v AS T2)` → `T2`. `perpendicular`.
    Perpendicular,
    /// 2D-only `(v AS T2, angle AS Float)` → `T2`. `rotate_2d`.
    Rotate2d,
    /// `cross`: unary over the 2D types, binary over the 3D types, ternary over the 4D
    /// types, each → its own `T_N` (the generalized (n-1)-ary cross product).
    Cross,
}

/// Build a member's overloads for `shape`, one per applicable type, each carrying
/// its type-specific internal FUNC body (`body(ty)`, from the member's `func_*.rs`)
/// as `Body::mfb` with the `__vector_<member>_<type>` rewrite target.
pub(crate) fn implementations(
    member: &'static str,
    shape: Shape,
    errors: &[&'static str],
    body: fn(&str) -> &'static str,
) -> Vec<Implementation> {
    let errs = || errors.to_vec();
    let mut out = Vec::new();
    match shape {
        Shape::UnaryScalar => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![param("v", &[], ParameterType::named(ty))],
                    element_of(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::UnaryVector => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![param("v", &[], ParameterType::named(ty))],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::BinaryScalar => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![
                        param("a", &["v"], ParameterType::named(ty)),
                        param("b", &["n"], ParameterType::named(ty)),
                    ],
                    element_of(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::BinaryVector => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![
                        param("a", &["v"], ParameterType::named(ty)),
                        param("b", &["n"], ParameterType::named(ty)),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::Lerp => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![
                        param("a", &[], ParameterType::named(ty)),
                        param("b", &[], ParameterType::named(ty)),
                        param("t", &[], ParameterType::Float),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::ClampLength => {
            for &ty in VEC_TYPES {
                out.push(imp(
                    vec![
                        param("v", &[], ParameterType::named(ty)),
                        param("max", &[], element_of(ty)),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::Perpendicular => {
            for &ty in VEC2_TYPES {
                out.push(imp(
                    vec![param("v", &[], ParameterType::named(ty))],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::Rotate2d => {
            for &ty in VEC2_TYPES {
                out.push(imp(
                    vec![
                        param("v", &[], ParameterType::named(ty)),
                        param("angle", &[], ParameterType::Float),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
        Shape::Cross => {
            // Unary (2D), binary (3D), ternary (4D); all name position 0 `a`/`v` so the
            // merged `call_param_names` table is `[[a, v], [b], [c]]`.
            for &ty in VEC2_TYPES {
                out.push(imp(
                    vec![param("a", &["v"], ParameterType::named(ty))],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
            for &ty in VEC3_TYPES {
                out.push(imp(
                    vec![
                        param("a", &["v"], ParameterType::named(ty)),
                        param("b", &[], ParameterType::named(ty)),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
            for &ty in VEC4_TYPES {
                out.push(imp(
                    vec![
                        param("a", &["v"], ParameterType::named(ty)),
                        param("b", &[], ParameterType::named(ty)),
                        param("c", &[], ParameterType::named(ty)),
                    ],
                    ParameterType::named(ty),
                    errs(),
                    Body::mfb(body(ty), rewrite(member, ty)),
                ));
            }
        }
    }
    out
}

/// Register the nine value records with element-typed props (in declaration order, so
/// `constant_components` reads each field's element type correctly).
fn add_records(pkg: &mut RegistryPackage) {
    const FIELDS: &[&str] = &["x", "y", "z", "w"];
    const FIELD_DESC: &[&str] = &[
        "The first component.",
        "The second component.",
        "The third component.",
        "The fourth component.",
    ];
    for &ty in VEC_TYPES {
        let dim = ty
            .chars()
            .last()
            .and_then(|c| c.to_digit(10))
            .expect("vector type ends in a dimension digit") as usize;
        let element = element_of(ty);
        let props = (0..dim)
            .map(|i| RecordProp {
                name: FIELDS[i],
                ty: element.clone(),
                description: FIELD_DESC[i],
            })
            .collect();
        // The type name is a compile-time literal from VEC_TYPES.
        pkg.add_record(RegistryRecord {
            name: ty,
            export: true,
            description: "",
            props,
        });
    }
}

// The record constants (`zero`/`one`/`up`/`right`/`forward`, 42 total). Each folds to a
// record constructor at every use site (`registry_record_constant`). `forward` (+z) is
// undefined in 2D. Component literals: decimals for Float/Fixed, integers for Integer.
const D_Z2: &[&str] = &["0.0", "0.0"];
const D_Z3: &[&str] = &["0.0", "0.0", "0.0"];
const D_Z4: &[&str] = &["0.0", "0.0", "0.0", "0.0"];
const D_O2: &[&str] = &["1.0", "1.0"];
const D_O3: &[&str] = &["1.0", "1.0", "1.0"];
const D_O4: &[&str] = &["1.0", "1.0", "1.0", "1.0"];
const D_UP2: &[&str] = &["0.0", "1.0"];
const D_UP3: &[&str] = &["0.0", "1.0", "0.0"];
const D_UP4: &[&str] = &["0.0", "1.0", "0.0", "0.0"];
const D_R2: &[&str] = &["1.0", "0.0"];
const D_R3: &[&str] = &["1.0", "0.0", "0.0"];
const D_R4: &[&str] = &["1.0", "0.0", "0.0", "0.0"];
const D_F3: &[&str] = &["0.0", "0.0", "1.0"];
const D_F4: &[&str] = &["0.0", "0.0", "1.0", "0.0"];
const I_Z2: &[&str] = &["0", "0"];
const I_Z3: &[&str] = &["0", "0", "0"];
const I_Z4: &[&str] = &["0", "0", "0", "0"];
const I_O2: &[&str] = &["1", "1"];
const I_O3: &[&str] = &["1", "1", "1"];
const I_O4: &[&str] = &["1", "1", "1", "1"];
const I_UP2: &[&str] = &["0", "1"];
const I_UP3: &[&str] = &["0", "1", "0"];
const I_UP4: &[&str] = &["0", "1", "0", "0"];
const I_R2: &[&str] = &["1", "0"];
const I_R3: &[&str] = &["1", "0", "0"];
const I_R4: &[&str] = &["1", "0", "0", "0"];
const I_F3: &[&str] = &["0", "0", "1"];
const I_F4: &[&str] = &["0", "0", "1", "0"];

/// The component literals for a base constant over `(dim, is_integer)`, or `None` when
/// the constant is undefined for that shape (`forward` in 2D).
fn constant_axis(base: &str, dim: usize, is_int: bool) -> Option<&'static [&'static str]> {
    Some(match (base, dim, is_int) {
        ("zero", 2, false) => D_Z2,
        ("zero", 3, false) => D_Z3,
        ("zero", 4, false) => D_Z4,
        ("zero", 2, true) => I_Z2,
        ("zero", 3, true) => I_Z3,
        ("zero", 4, true) => I_Z4,
        ("one", 2, false) => D_O2,
        ("one", 3, false) => D_O3,
        ("one", 4, false) => D_O4,
        ("one", 2, true) => I_O2,
        ("one", 3, true) => I_O3,
        ("one", 4, true) => I_O4,
        ("up", 2, false) => D_UP2,
        ("up", 3, false) => D_UP3,
        ("up", 4, false) => D_UP4,
        ("up", 2, true) => I_UP2,
        ("up", 3, true) => I_UP3,
        ("up", 4, true) => I_UP4,
        ("right", 2, false) => D_R2,
        ("right", 3, false) => D_R3,
        ("right", 4, false) => D_R4,
        ("right", 2, true) => I_R2,
        ("right", 3, true) => I_R3,
        ("right", 4, true) => I_R4,
        ("forward", 3, false) => D_F3,
        ("forward", 4, false) => D_F4,
        ("forward", 3, true) => I_F3,
        ("forward", 4, true) => I_F4,
        // `forward` is undefined in 2D (+z axis); every other pairing is covered above.
        _ => return None,
    })
}

/// Register the 42 record constants (`zeroFloat3`, `forwardInteger4`, …).
fn add_constants(pkg: &mut RegistryPackage) {
    for &ty in VEC_TYPES {
        let dim = ty
            .chars()
            .last()
            .and_then(|c| c.to_digit(10))
            .expect("vector type ends in a dimension digit") as usize;
        let is_int = ty.starts_with("Integer");
        for base in ["zero", "one", "up", "right", "forward"] {
            let Some(components) = constant_axis(base, dim, is_int) else {
                continue;
            };
            let name: &'static str = Box::leak(format!("{base}{ty}").into_boxed_str());
            pkg.add_constant(RegistryConstant {
                name,
                type_name: ty,
                value: None,
                components: Some(components),
                message: None,
                symbol: None,
            });
        }
    }
}

/// Register the nine `toString(VecN)` general-builtin overrides.
fn add_overrides(pkg: &mut RegistryPackage) {
    for &ty in VEC_TYPES {
        let helper: &'static str =
            Box::leak(format!("__vector_toString_{}", ty.to_ascii_lowercase()).into_boxed_str());
        pkg.add_override(RegistryOverride {
            builtin: "toString",
            arg_type: ty,
            helper,
        });
    }
}

// ---- exact per-argument-type dispatch --------------------------------------------
//
// The registry's generic overload matcher (`RegistryFunction::select` / `unify` /
// `leaf_matches` in `codegen/registry/mod.rs`) is deliberately COARSE on value
// nominals: two distinct non-resource `Named` types unify (so a union parameter
// accepts a widening variant — `json::stringify(JsonNull)`), in BOTH the strict and
// lenient modes. Vector members dispatch by EXACT record type (`Float2` ≠ `Integer2`)
// and each type has its own return type (`length(Float3) AS Float`,
// `length(Integer3) AS Integer`) and its own `__vector_<op>_<type>` rewrite target — a
// shape the coarse matcher cannot select (it always picks the first of the nine
// per-type overloads) and that no `ParameterType` return can express (there is no
// "element-of-Arg(0)"). So `vector` keeps this thin EXACT selector — reproducing the
// pre-migration `VectorResolver` — over its own registered overload data, and the
// generic `registry::resolve_call` / `registry::rewrite_target` are routed around for
// `vector` in `builtins::resolve_call_return_type` and `ir/lower.rs`. The shared
// matcher is left untouched (no coarse-nominal change ripples onto other packages).

/// The registered overload of `qualified` (`"vector.length"`) whose parameter types
/// EXACTLY equal the call's `arg_types`, or `None`. Exact means same arity and, per
/// position, the same spelled type — so `abs(String)` (scalar vs record) and
/// `clamp_length(Fixed2, Float)` (wrong scalar element) select nothing, exactly as the
/// legacy resolver rejected them.
fn select(qualified: &str, arg_types: &[String]) -> Option<&'static Implementation> {
    let function = registry().resolve_func(qualified)?.function;
    function.implementations().iter().find(|imp| {
        imp.params.len() == arg_types.len()
            && imp
                .params
                .iter()
                .zip(arg_types)
                .all(|(param, arg)| param.ty.name().as_ref() == arg.as_str())
    })
}

/// The exact argument-typed return type of a `vector` call (`vector.length` over a
/// `Float3` → `"Float"`, over an `Integer2` → `"Integer"`; `vector.normalize` over a
/// `Float3` → `"Float3"`), or `None` when the arguments match no overload. Consulted by
/// `builtins::resolve_call_return_type` before the generic coarse registry path.
pub(crate) fn resolve_return_type(qualified: &str, arg_types: &[String]) -> Option<String> {
    Some(
        select(qualified, arg_types)?
            .return_type
            .name()
            .into_owned(),
    )
}

/// The type-specific `__vector_<op>_<type>` rewrite target of a `vector` call, or
/// `None` when the arguments match no overload. Consulted by `ir/lower.rs` before the
/// generic coarse registry rewrite path. Returns the already-interned `&'static`
/// target from the selected overload's `Body::Rewrite` (no per-call allocation).
pub(crate) fn rewrite_target(qualified: &str, arg_types: &[String]) -> Option<&'static str> {
    select(qualified, arg_types)?.body.rewrite_target()
}

/// Register the `vector` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("vector", INTRO, DESC);

    // `vector` imports only the intrinsic `math` package (`math.sqrt`/`math.clamp`).
    pkg.add_imports(vec!["math"]);

    add_records(&mut pkg);

    // The internal rounding integer-sqrt helpers, the Fixed-path integer `angle`
    // cores, and the per-type `toString` renderers (the `add_override` targets) —
    // one `helper_*.rs` per FUNC (`add_helper` — private-only), in the old
    // companion order.
    helper_isqrt_floor::register(&mut pkg);
    helper_isqrt_round::register(&mut pkg);
    helper_angle_fixed_integer2::register(&mut pkg);
    helper_angle_fixed_integer3::register(&mut pkg);
    helper_angle_fixed_integer4::register(&mut pkg);
    helper_to_string_float2::register(&mut pkg);
    helper_to_string_float3::register(&mut pkg);
    helper_to_string_float4::register(&mut pkg);
    helper_to_string_fixed2::register(&mut pkg);
    helper_to_string_fixed3::register(&mut pkg);
    helper_to_string_fixed4::register(&mut pkg);
    helper_to_string_integer2::register(&mut pkg);
    helper_to_string_integer3::register(&mut pkg);
    helper_to_string_integer4::register(&mut pkg);

    // The 19 function members, each a `func_*.rs` descriptor whose per-type
    // overload bodies ride `Body::mfb` (built by the shared `implementations`
    // shape machinery above).
    func_length::register(&mut pkg);
    func_normalize::register(&mut pkg);
    func_distance::register(&mut pkg);
    func_dot::register(&mut pkg);
    func_cross::register(&mut pkg);
    func_reflect::register(&mut pkg);
    func_project::register(&mut pkg);
    func_reject::register(&mut pkg);
    func_angle::register(&mut pkg);
    func_lerp::register(&mut pkg);
    func_lerp_unclamped::register(&mut pkg);
    func_slerp::register(&mut pkg);
    func_clamp_length::register(&mut pkg);
    func_scale::register(&mut pkg);
    func_min::register(&mut pkg);
    func_max::register(&mut pkg);
    func_abs::register(&mut pkg);
    func_perpendicular::register(&mut pkg);
    func_rotate_2d::register(&mut pkg);

    add_constants(&mut pkg);
    add_overrides(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn vector_registered_on_the_clean_room_registry() {
        let pkg = registry()
            .resolve_package("vector")
            .expect("vector package");
        assert_eq!(pkg.functions().len(), 19);
        assert_eq!(pkg.records().len(), 9);
        // The nine records are visible to the generic type query.
        assert!(registry().is_builtin_type("Float3"));
        assert!(registry().is_builtin_type("Integer2"));
        assert!(!registry().is_builtin_type("Float5"));
        // `vector` imports the intrinsic `math` package.
        assert_eq!(pkg.imports(), &["math"]);
    }

    #[test]
    fn exact_overload_selection_reproduces_the_legacy_resolver() {
        // vector dispatches by EXACT record type through `super::resolve_return_type`
        // (the generic coarse registry matcher cannot distinguish the nine nominals).
        let r = |name: &str, args: &[&str]| super::resolve_return_type(name, &strings(args));
        // Scalar-returning members echo the element type.
        assert_eq!(r("vector.length", &["Float3"]).as_deref(), Some("Float"));
        assert_eq!(
            r("vector.length", &["Integer2"]).as_deref(),
            Some("Integer")
        );
        assert_eq!(
            r("vector.dot", &["Fixed4", "Fixed4"]).as_deref(),
            Some("Fixed")
        );
        // Vector-returning members echo the argument type.
        assert_eq!(
            r("vector.normalize", &["Float3"]).as_deref(),
            Some("Float3")
        );
        assert_eq!(
            r("vector.reflect", &["Float3", "Float3"]).as_deref(),
            Some("Float3")
        );
        // cross by dimension/arity.
        assert_eq!(r("vector.cross", &["Float2"]).as_deref(), Some("Float2"));
        assert_eq!(
            r("vector.cross", &["Float3", "Float3"]).as_deref(),
            Some("Float3")
        );
        assert_eq!(
            r("vector.cross", &["Float4", "Float4", "Float4"]).as_deref(),
            Some("Float4")
        );
        assert_eq!(r("vector.cross", &["Float3"]), None);
        // clamp_length: the scalar max is the element type.
        assert_eq!(
            r("vector.clamp_length", &["Float3", "Float"]).as_deref(),
            Some("Float3")
        );
        assert_eq!(r("vector.clamp_length", &["Fixed2", "Float"]), None);
        // 2D-only members.
        assert_eq!(
            r("vector.perpendicular", &["Float2"]).as_deref(),
            Some("Float2")
        );
        assert_eq!(r("vector.perpendicular", &["Float3"]), None);
        assert_eq!(
            r("vector.rotate_2d", &["Float2", "Float"]).as_deref(),
            Some("Float2")
        );
        // Mismatched vector types / scalar args are rejected.
        assert_eq!(r("vector.distance", &["Float3", "Float2"]), None);
        assert_eq!(r("vector.abs", &["String"]), None);
    }

    #[test]
    fn rewrite_targets_are_type_specific() {
        assert_eq!(
            super::rewrite_target("vector.length", &strings(&["Float3"])),
            Some("__vector_length_float3")
        );
        assert_eq!(
            super::rewrite_target("vector.length", &strings(&["Integer2"])),
            Some("__vector_length_integer2")
        );
        assert_eq!(
            super::rewrite_target("vector.cross", &strings(&["Float2"])),
            Some("__vector_cross_float2")
        );
        assert_eq!(
            super::rewrite_target("vector.angle", &strings(&["Integer2", "Integer2"])),
            Some("__vector_angle_integer2")
        );
    }

    #[test]
    fn expected_argument_hints_match_the_legacy_prose() {
        assert_eq!(
            registry::expected_arguments("vector.clamp_length"),
            Some("a vector and a scalar max of the vector's element type")
        );
        assert_eq!(
            registry::expected_arguments("vector.cross"),
            Some("one T2, two T3, or three T4 vectors of the same type")
        );
        assert_eq!(
            registry::expected_arguments("vector.perpendicular"),
            Some("a 2D vector (Float2, Fixed2, Integer2)")
        );
    }

    #[test]
    fn record_constants_fold_to_constructors() {
        assert!(registry::is_package_constant("vector.zeroFloat3"));
        assert_eq!(
            registry::constant_type_name("vector.upFloat3"),
            Some("Float3")
        );
        assert_eq!(
            registry::constant_components("vector.upFloat3"),
            Some(&["0.0", "1.0", "0.0"][..])
        );
        assert_eq!(
            registry::constant_components("vector.oneInteger2"),
            Some(&["1", "1"][..])
        );
        // `forward` is undefined in 2D.
        assert!(!registry::is_package_constant("vector.forwardFloat2"));
        assert!(registry::is_package_constant("vector.forwardFloat3"));
    }

    #[test]
    fn tostring_override_routes_to_the_companion_renderer() {
        assert_eq!(
            registry::general_override_target(
                "toString",
                &crate::types::ParameterType::parse("Float2")
            ),
            Some("__vector_toString_float2")
        );
        assert_eq!(
            registry::general_override_target(
                "toString",
                &crate::types::ParameterType::parse("Integer4")
            ),
            Some("__vector_toString_integer4")
        );
        assert_eq!(
            registry::general_override_target(
                "toString",
                &crate::types::ParameterType::parse("Nope")
            ),
            None
        );
    }

    #[test]
    fn call_param_names_carry_aliases() {
        // Binary members: position 0 = [a, v], position 1 = [b, n].
        assert_eq!(
            registry::call_param_names("vector.dot"),
            Some(vec![vec!["a", "v"], vec!["b", "n"]])
        );
        // cross merges its arity overloads: [[a, v], [b], [c]].
        assert_eq!(
            registry::call_param_names("vector.cross"),
            Some(vec![vec!["a", "v"], vec!["b"], vec!["c"]])
        );
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry()
            .resolve_package("vector")
            .expect("vector")
            .get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-vector>"),
            "builtins/vector.mfb",
            &source,
        )
        .expect("reassembled vector source parses");
    }
}

pub(crate) mod builder_simd_fixed_math;
pub(crate) mod builder_simd_float_math;
pub(crate) mod builder_simd_math;
pub(crate) mod simd_kernel_coeffs;
pub(crate) use simd_kernel_coeffs::*;
pub(crate) mod builder_vector_inline;
pub(crate) use builder_vector_inline::*;
