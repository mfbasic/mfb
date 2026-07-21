//! Built-in `vector::` package seam (plan-06-vector.md).
//!
//! Nine fixed-width math-vector value records — `Float2/3/4`, `Fixed2/3/4`,
//! `Integer2/3/4` — and a set of overloaded geometry / utility / 2D functions
//! and package constants over them. Like `net`/`datetime`, the behaviour lives
//! in the source companion `vector_package.mfb`; this module owns the type
//! registration, the per-call return-type/arity metadata the syntaxcheck needs,
//! and the mapping from a public `vector::` call onto the type-specific internal
//! implementation in the companion.
//!
//! Dispatch is by **exact argument record type**. Because user-FUNC overload
//! resolution runs in the monomorphizer (before the IR-lowering rename), the
//! companion does not overload its `__vector_*` helpers: every (function,
//! element-type, dimension) triple is a distinctly named FUNC, and
//! `implementation_name` selects it from the call's argument types. The public
//! return type is computed here from the same argument types so the syntaxcheck
//! never needs the companion signature.

use std::borrow::Cow;
use std::path::Path;

pub(crate) const FLOAT2_TYPE: &str = "Float2";
pub(crate) const FLOAT3_TYPE: &str = "Float3";
pub(crate) const FLOAT4_TYPE: &str = "Float4";
pub(crate) const FIXED2_TYPE: &str = "Fixed2";
pub(crate) const FIXED3_TYPE: &str = "Fixed3";
pub(crate) const FIXED4_TYPE: &str = "Fixed4";
pub(crate) const INTEGER2_TYPE: &str = "Integer2";
pub(crate) const INTEGER3_TYPE: &str = "Integer3";
pub(crate) const INTEGER4_TYPE: &str = "Integer4";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

/// The nine qualified built-in record types. Referenced as `vector::Float3`,
/// normalized to the bare id at parse time by `qualified_builtin_type`
/// (plan-06-vector.md §5). Their fields are declared by the `EXPORT TYPE`s in
/// the companion source, so — like `net::Url` — they carry no
/// `builtin_type_fields` entry.
pub(crate) fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        FLOAT2_TYPE
            | FLOAT3_TYPE
            | FLOAT4_TYPE
            | FIXED2_TYPE
            | FIXED3_TYPE
            | FIXED4_TYPE
            | INTEGER2_TYPE
            | INTEGER3_TYPE
            | INTEGER4_TYPE
    )
}

/// Split a vector type id into `(element, dimension)`, e.g. `Float3 ->
/// ("Float", 3)`. `None` for any non-vector type.
fn vector_shape(type_name: &str) -> Option<(&'static str, usize)> {
    let shape = match type_name {
        FLOAT2_TYPE => ("Float", 2),
        FLOAT3_TYPE => ("Float", 3),
        FLOAT4_TYPE => ("Float", 4),
        FIXED2_TYPE => ("Fixed", 2),
        FIXED3_TYPE => ("Fixed", 3),
        FIXED4_TYPE => ("Fixed", 4),
        INTEGER2_TYPE => ("Integer", 2),
        INTEGER3_TYPE => ("Integer", 3),
        INTEGER4_TYPE => ("Integer", 4),
        _ => return None,
    };
    Some(shape)
}

// Public function members (qualified, dot form). Each is one logical operation
// with 3..9 type/arity overloads resolved in `resolve_call`.
const LENGTH: &str = "vector.length";
const NORMALIZE: &str = "vector.normalize";
const DISTANCE: &str = "vector.distance";
const DOT: &str = "vector.dot";
const CROSS: &str = "vector.cross";
const REFLECT: &str = "vector.reflect";
const PROJECT: &str = "vector.project";
const REJECT: &str = "vector.reject";
const ANGLE: &str = "vector.angle";
const LERP: &str = "vector.lerp";
const LERP_UNCLAMPED: &str = "vector.lerp_unclamped";
const SLERP: &str = "vector.slerp";
const CLAMP_LENGTH: &str = "vector.clamp_length";
const SCALE: &str = "vector.scale";
const MIN: &str = "vector.min";
const MAX: &str = "vector.max";
const ABS: &str = "vector.abs";
const PERPENDICULAR: &str = "vector.perpendicular";
const ROTATE_2D: &str = "vector.rotate_2d";

pub(crate) fn is_vector_call(name: &str) -> bool {
    is_vector_function(name) || is_vector_constant(name)
}

fn is_vector_function(name: &str) -> bool {
    matches!(
        name,
        LENGTH
            | NORMALIZE
            | DISTANCE
            | DOT
            | CROSS
            | REFLECT
            | PROJECT
            | REJECT
            | ANGLE
            | LERP
            | LERP_UNCLAMPED
            | SLERP
            | CLAMP_LENGTH
            | SCALE
            | MIN
            | MAX
            | ABS
            | PERPENDICULAR
            | ROTATE_2D
    )
}

/// `(min, max)` argument count for a function member. `cross` spans 1..3 (its
/// arity is dimension-specific, validated precisely in `resolve_call`).
pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    if is_vector_constant(name) {
        return Some((0, 0));
    }
    let span = match name {
        LENGTH | NORMALIZE | ABS | PERPENDICULAR => (1, 1),
        DISTANCE | DOT | REFLECT | PROJECT | REJECT | ANGLE | SCALE | MIN | MAX | CLAMP_LENGTH
        | ROTATE_2D => (2, 2),
        CROSS => (1, 3),
        LERP | LERP_UNCLAMPED | SLERP => (3, 3),
        _ => return None,
    };
    Some(span)
}

/// Whether `a` and `b` are the same vector type.
fn same_vector(a: &str, b: &str) -> bool {
    vector_shape(a).is_some() && a == b
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    if let Some(type_name) = constant_type_name(name) {
        return arg_types.is_empty().then_some(ResolvedCall {
            return_type: Cow::Borrowed(type_name),
        });
    }
    let a = arg_types.first().map(String::as_str).unwrap_or("");
    let (element, dim) = vector_shape(a)?;
    let ok_scalar = || {
        Some(ResolvedCall {
            return_type: Cow::Borrowed(element),
        })
    };
    let ok_vector = || {
        Some(ResolvedCall {
            return_type: Cow::Owned(a.to_string()),
        })
    };
    match name {
        // (v AS T_N) -> scalar T
        LENGTH if arg_types.len() == 1 => ok_scalar(),
        // (v AS T_N) -> T_N
        NORMALIZE | ABS if arg_types.len() == 1 => ok_vector(),
        // (a AS T_N, b AS T_N) -> scalar T
        DISTANCE | DOT | ANGLE if arg_types.len() == 2 && same_vector(a, &arg_types[1]) => {
            ok_scalar()
        }
        // (a AS T_N, b AS T_N) -> T_N
        REFLECT | PROJECT | REJECT | SCALE | MIN | MAX
            if arg_types.len() == 2 && same_vector(a, &arg_types[1]) =>
        {
            ok_vector()
        }
        // cross: unary 2D / binary 3D / ternary 4D (n-1 operands), all T_N.
        CROSS if dim == 2 && arg_types.len() == 1 => ok_vector(),
        CROSS if dim == 3 && arg_types.len() == 2 && same_vector(a, &arg_types[1]) => ok_vector(),
        CROSS
            if dim == 4
                && arg_types.len() == 3
                && same_vector(a, &arg_types[1])
                && same_vector(a, &arg_types[2]) =>
        {
            ok_vector()
        }
        // (a AS T_N, b AS T_N, t AS Float) -> T_N, for every element type.
        LERP | LERP_UNCLAMPED | SLERP
            if arg_types.len() == 3 && same_vector(a, &arg_types[1]) && arg_types[2] == "Float" =>
        {
            ok_vector()
        }
        // (v AS T_N, max AS T) -> T_N
        CLAMP_LENGTH if arg_types.len() == 2 && arg_types[1] == element => ok_vector(),
        // 2D-only: (v AS T2) -> T2 / (v AS T2, angle AS Float) -> T2
        PERPENDICULAR if dim == 2 && arg_types.len() == 1 => ok_vector(),
        ROTATE_2D if dim == 2 && arg_types.len() == 2 && arg_types[1] == "Float" => ok_vector(),
        _ => None,
    }
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    if is_vector_constant(name) {
        return Some("()");
    }
    let text = match name {
        LENGTH | NORMALIZE | ABS => "a vector (Float2/3/4, Fixed2/3/4, Integer2/3/4)",
        DISTANCE | DOT | ANGLE | REFLECT | PROJECT | REJECT | SCALE | MIN | MAX => {
            "two vectors of the same type"
        }
        CROSS => "one T2, two T3, or three T4 vectors of the same type",
        LERP | LERP_UNCLAMPED | SLERP => "two vectors of the same type and a Float t",
        CLAMP_LENGTH => "a vector and a scalar max of the vector's element type",
        PERPENDICULAR => "a 2D vector (Float2, Fixed2, Integer2)",
        ROTATE_2D => "a 2D vector and a Float angle",
        _ => return None,
    };
    Some(text)
}

/// Maximal-arity parameter names, used for named-argument diagnostics.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    if is_vector_constant(name) {
        return Some(&[]);
    }
    let params: &'static [&'static [&'static str]] = match name {
        LENGTH | NORMALIZE | ABS | PERPENDICULAR => &[&["v"]],
        DISTANCE | DOT | ANGLE | REFLECT | PROJECT | REJECT | SCALE | MIN | MAX => {
            &[&["a", "v"], &["b", "n"]]
        }
        CROSS => &[&["a", "v"], &["b"], &["c"]],
        LERP | LERP_UNCLAMPED | SLERP => &[&["a"], &["b"], &["t"]],
        CLAMP_LENGTH => &[&["v"], &["max"]],
        ROTATE_2D => &[&["v"], &["angle"]],
        _ => return None,
    };
    Some(params)
}

/// The type-specific internal implementation for a public `vector::` call, e.g.
/// `vector.length` over a `Float3` → `__vector_length_float3`. Constants map to
/// their zero-arg accessor name. Returns `None` when the call does not resolve
/// to a vector overload (the syntaxcheck has already reported the error).
pub(crate) fn implementation_name(name: &str, arg_types: &[String]) -> Option<String> {
    if is_vector_constant(name) {
        let member = name.strip_prefix("vector.")?;
        return Some(format!("__vector_{member}"));
    }
    // Resolve against the same overload table the syntaxcheck used, then build the
    // `<func>_<type>` suffix from the first argument's vector type.
    resolve_call(name, arg_types)?;
    let suffix = arg_types.first()?.to_ascii_lowercase();
    let member = name.strip_prefix("vector.")?;
    Some(format!("__vector_{member}_{suffix}"))
}

// ---- package constants (§4.19) ---------------------------------------------
//
// 42 package values, referenced no-paren as `vector::zeroFloat3` (the
// `math::pi` idiom). Each is a record value; the IR lowering inlines a record
// constructor at every use site (`constant_components`), so a constant copies
// by value on each read with no shared global.

const CONST_BASES: &[&str] = &["zero", "one", "up", "right", "forward"];

/// Per-axis literal components for a base constant over `(element, dim)`, or
/// `None` when the constant is undefined for that shape (`forward` in 2D).
fn constant_axis(base: &str, element: &str, dim: usize) -> Option<Vec<String>> {
    let zero = if element == "Integer" { "0" } else { "0.0" };
    let one = if element == "Integer" { "1" } else { "1.0" };
    let mut out = vec![zero.to_string(); dim];
    match base {
        "zero" => {}
        "one" => out = vec![one.to_string(); dim],
        "up" => out[1] = one.to_string(),    // +y axis
        "right" => out[0] = one.to_string(), // +x axis
        "forward" => {
            if dim < 3 {
                return None; // +z axis is undefined in 2D
            }
            out[2] = one.to_string();
        }
        _ => return None,
    }
    Some(out)
}

/// Parse a constant member name (`zeroFloat3`) into `(base, type_name)`.
fn parse_constant(member: &str) -> Option<(&'static str, String)> {
    for base in CONST_BASES {
        if let Some(rest) = member.strip_prefix(base) {
            // rest is `<Element><Dim>`, e.g. `Float3`.
            if is_builtin_type(rest) {
                return Some((base, rest.to_string()));
            }
        }
    }
    None
}

pub(crate) fn is_vector_constant(name: &str) -> bool {
    let Some(member) = name.strip_prefix("vector.") else {
        return false;
    };
    match parse_constant(member) {
        Some((base, type_name)) => {
            let (element, dim) = vector_shape(&type_name).expect("constant carries a vector type");
            constant_axis(base, element, dim).is_some()
        }
        None => false,
    }
}

/// The vector type a constant evaluates to (`vector.zeroFloat3` → `Float3`).
pub(crate) fn constant_type_name(name: &str) -> Option<&'static str> {
    let member = name.strip_prefix("vector.")?;
    let (_, type_name) = parse_constant(member)?;
    vector_shape(&type_name)?; // confirm it is a real vector type
                               // Re-derive the &'static type id.
    match type_name.as_str() {
        FLOAT2_TYPE => Some(FLOAT2_TYPE),
        FLOAT3_TYPE => Some(FLOAT3_TYPE),
        FLOAT4_TYPE => Some(FLOAT4_TYPE),
        FIXED2_TYPE => Some(FIXED2_TYPE),
        FIXED3_TYPE => Some(FIXED3_TYPE),
        FIXED4_TYPE => Some(FIXED4_TYPE),
        INTEGER2_TYPE => Some(INTEGER2_TYPE),
        INTEGER3_TYPE => Some(INTEGER3_TYPE),
        INTEGER4_TYPE => Some(INTEGER4_TYPE),
        _ => None,
    }
}

/// The record type and per-component `(element_type, literal)` list a constant
/// inlines to, e.g. `vector.upFloat3` → `("Float3", [("Float","0.0"),
/// ("Float","1.0"), ("Float","0.0")])`. The IR lowering builds a constructor
/// from this.
pub(crate) fn constant_components(name: &str) -> Option<(String, Vec<(String, String)>)> {
    let member = name.strip_prefix("vector.")?;
    let (base, type_name) = parse_constant(member)?;
    let (element, dim) = vector_shape(&type_name)?;
    let axis = constant_axis(base, element, dim)?;
    let components = axis
        .into_iter()
        .map(|value| (element.to_string(), value))
        .collect();
    Some((type_name, components))
}

/// The companion `toString` renderer for one of the nine vector types, used by
/// `general_override_target` to route `toString(v)` (plan-06-vector.md §4.18).
pub(crate) fn tostring_override_target(type_name: &str) -> Option<&'static str> {
    match type_name {
        FLOAT2_TYPE => Some("__vector_toString_float2"),
        FLOAT3_TYPE => Some("__vector_toString_float3"),
        FLOAT4_TYPE => Some("__vector_toString_float4"),
        FIXED2_TYPE => Some("__vector_toString_fixed2"),
        FIXED3_TYPE => Some("__vector_toString_fixed3"),
        FIXED4_TYPE => Some("__vector_toString_fixed4"),
        INTEGER2_TYPE => Some("__vector_toString_integer2"),
        INTEGER3_TYPE => Some("__vector_toString_integer3"),
        INTEGER4_TYPE => Some("__vector_toString_integer4"),
        _ => None,
    }
}

pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        Path::new("<builtin-vector>"),
        "builtins/vector.mfb",
        include_str!("vector_package.mfb"),
    )
}

pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "vector")
    })
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &strings(args)).map(|r| r.return_type.into_owned())
    }

    fn project(src: &str) -> crate::ast::AstProject {
        let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
            .expect("parse source");
        crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        }
    }

    #[test]
    fn builtin_types_recognized() {
        for t in [
            FLOAT2_TYPE,
            FLOAT3_TYPE,
            FLOAT4_TYPE,
            FIXED2_TYPE,
            FIXED3_TYPE,
            FIXED4_TYPE,
            INTEGER2_TYPE,
            INTEGER3_TYPE,
            INTEGER4_TYPE,
        ] {
            assert!(is_builtin_type(t), "{t}");
        }
        assert!(!is_builtin_type("Float5"));
        assert!(!is_builtin_type("Float"));
        assert!(!is_builtin_type(""));
    }

    #[test]
    fn vector_shape_splits() {
        assert_eq!(vector_shape("Float3"), Some(("Float", 3)));
        assert_eq!(vector_shape("Fixed2"), Some(("Fixed", 2)));
        assert_eq!(vector_shape("Integer4"), Some(("Integer", 4)));
        assert_eq!(vector_shape("Nope"), None);
    }

    #[test]
    fn is_vector_call_covers_functions_and_constants() {
        assert!(is_vector_call(LENGTH));
        assert!(is_vector_call(ROTATE_2D));
        assert!(is_vector_call("vector.zeroFloat3"));
        assert!(!is_vector_call("vector.bogus"));
        assert!(!is_vector_call("length"));
    }

    #[test]
    fn is_vector_function_flags() {
        for f in [
            LENGTH,
            NORMALIZE,
            DISTANCE,
            DOT,
            CROSS,
            REFLECT,
            PROJECT,
            REJECT,
            ANGLE,
            LERP,
            LERP_UNCLAMPED,
            SLERP,
            CLAMP_LENGTH,
            SCALE,
            MIN,
            MAX,
            ABS,
            PERPENDICULAR,
            ROTATE_2D,
        ] {
            assert!(is_vector_function(f), "{f}");
        }
        assert!(!is_vector_function("vector.zeroFloat3"));
        assert!(!is_vector_function("vector.bogus"));
    }

    #[test]
    fn arity_spans() {
        assert_eq!(arity(LENGTH), Some((1, 1)));
        assert_eq!(arity(NORMALIZE), Some((1, 1)));
        assert_eq!(arity(ABS), Some((1, 1)));
        assert_eq!(arity(PERPENDICULAR), Some((1, 1)));
        assert_eq!(arity(DISTANCE), Some((2, 2)));
        assert_eq!(arity(ROTATE_2D), Some((2, 2)));
        assert_eq!(arity(CROSS), Some((1, 3)));
        assert_eq!(arity(LERP), Some((3, 3)));
        assert_eq!(arity(SLERP), Some((3, 3)));
        assert_eq!(arity("vector.zeroFloat3"), Some((0, 0)));
        assert_eq!(arity("vector.bogus"), None);
    }

    #[test]
    fn same_vector_helper() {
        assert!(same_vector("Float3", "Float3"));
        assert!(!same_vector("Float3", "Float2"));
        assert!(!same_vector("Nope", "Nope"));
    }

    #[test]
    fn resolve_scalar_returns() {
        assert_eq!(ret(LENGTH, &["Float3"]), Some("Float".to_string()));
        assert_eq!(ret(LENGTH, &["Integer2"]), Some("Integer".to_string()));
        assert_eq!(
            ret(DISTANCE, &["Float3", "Float3"]),
            Some("Float".to_string())
        );
        assert_eq!(ret(DOT, &["Fixed4", "Fixed4"]), Some("Fixed".to_string()));
        assert_eq!(
            ret(ANGLE, &["Integer2", "Integer2"]),
            Some("Integer".to_string())
        );
    }

    #[test]
    fn resolve_vector_returns() {
        assert_eq!(ret(NORMALIZE, &["Float3"]), Some("Float3".to_string()));
        assert_eq!(ret(ABS, &["Integer4"]), Some("Integer4".to_string()));
        assert_eq!(
            ret(REFLECT, &["Float3", "Float3"]),
            Some("Float3".to_string())
        );
        assert_eq!(
            ret(SCALE, &["Fixed2", "Fixed2"]),
            Some("Fixed2".to_string())
        );
        assert_eq!(ret(MIN, &["Float2", "Float2"]), Some("Float2".to_string()));
        assert_eq!(ret(MAX, &["Float2", "Float2"]), Some("Float2".to_string()));
        assert_eq!(
            ret(PROJECT, &["Float3", "Float3"]),
            Some("Float3".to_string())
        );
        assert_eq!(
            ret(REJECT, &["Float3", "Float3"]),
            Some("Float3".to_string())
        );
    }

    #[test]
    fn resolve_cross_by_dimension() {
        assert_eq!(ret(CROSS, &["Float2"]), Some("Float2".to_string()));
        assert_eq!(
            ret(CROSS, &["Float3", "Float3"]),
            Some("Float3".to_string())
        );
        assert_eq!(
            ret(CROSS, &["Float4", "Float4", "Float4"]),
            Some("Float4".to_string())
        );
        // wrong arity for the dimension
        assert_eq!(ret(CROSS, &["Float3"]), None);
        assert_eq!(ret(CROSS, &["Float2", "Float2"]), None);
        assert_eq!(ret(CROSS, &["Float4", "Float4"]), None);
    }

    #[test]
    fn resolve_lerp_family() {
        assert_eq!(
            ret(LERP, &["Float3", "Float3", "Float"]),
            Some("Float3".to_string())
        );
        assert_eq!(
            ret(LERP_UNCLAMPED, &["Fixed2", "Fixed2", "Float"]),
            Some("Fixed2".to_string())
        );
        assert_eq!(
            ret(SLERP, &["Float4", "Float4", "Float"]),
            Some("Float4".to_string())
        );
        // t must be Float
        assert_eq!(ret(LERP, &["Float3", "Float3", "Integer"]), None);
        assert_eq!(ret(LERP, &["Float3", "Float2", "Float"]), None);
    }

    #[test]
    fn resolve_clamp_length_and_2d() {
        assert_eq!(
            ret(CLAMP_LENGTH, &["Float3", "Float"]),
            Some("Float3".to_string())
        );
        assert_eq!(ret(CLAMP_LENGTH, &["Float3", "Integer"]), None);
        assert_eq!(ret(PERPENDICULAR, &["Float2"]), Some("Float2".to_string()));
        assert_eq!(ret(PERPENDICULAR, &["Float3"]), None);
        assert_eq!(
            ret(ROTATE_2D, &["Float2", "Float"]),
            Some("Float2".to_string())
        );
        assert_eq!(ret(ROTATE_2D, &["Float3", "Float"]), None);
        assert_eq!(ret(ROTATE_2D, &["Float2", "Integer"]), None);
    }

    #[test]
    fn resolve_constant_and_rejections() {
        assert_eq!(ret("vector.zeroFloat3", &[]), Some("Float3".to_string()));
        // constant with args -> None
        assert_eq!(ret("vector.zeroFloat3", &["Float3"]), None);
        // unknown name
        assert_eq!(ret("vector.bogus", &["Float3"]), None);
        // non-vector first arg
        assert_eq!(ret(LENGTH, &["Float"]), None);
        // wrong arity
        assert_eq!(ret(LENGTH, &["Float3", "Float3"]), None);
        assert_eq!(ret(DISTANCE, &["Float3"]), None);
        // mismatched vector types
        assert_eq!(ret(DISTANCE, &["Float3", "Float2"]), None);
    }

    #[test]
    fn expected_arguments_text() {
        assert!(expected_arguments(LENGTH).unwrap().contains("vector"));
        assert!(expected_arguments(DISTANCE).unwrap().contains("two"));
        assert!(expected_arguments(CROSS).unwrap().contains("T2"));
        assert!(expected_arguments(LERP).unwrap().contains("Float"));
        assert!(expected_arguments(CLAMP_LENGTH).unwrap().contains("scalar"));
        assert!(expected_arguments(PERPENDICULAR).unwrap().contains("2D"));
        assert!(expected_arguments(ROTATE_2D).unwrap().contains("angle"));
        assert_eq!(expected_arguments("vector.zeroFloat3"), Some("()"));
        assert_eq!(expected_arguments("vector.bogus"), None);
    }

    #[test]
    fn call_param_names_shapes() {
        assert_eq!(call_param_names(LENGTH), Some(&[&["v"][..]][..]));
        assert!(call_param_names(DISTANCE).is_some());
        assert!(call_param_names(CROSS).is_some());
        assert!(call_param_names(LERP).is_some());
        assert!(call_param_names(CLAMP_LENGTH).is_some());
        assert!(call_param_names(ROTATE_2D).is_some());
        assert_eq!(call_param_names("vector.zeroFloat3"), Some(&[][..]));
        assert_eq!(call_param_names("vector.bogus"), None);
    }

    #[test]
    fn implementation_name_function_and_constant() {
        assert_eq!(
            implementation_name(LENGTH, &strings(&["Float3"])),
            Some("__vector_length_float3".to_string())
        );
        assert_eq!(
            implementation_name(CROSS, &strings(&["Float2"])),
            Some("__vector_cross_float2".to_string())
        );
        assert_eq!(
            implementation_name("vector.zeroFloat3", &[]),
            Some("__vector_zeroFloat3".to_string())
        );
        // unresolved overload
        assert_eq!(implementation_name(LENGTH, &strings(&["Float"])), None);
        assert_eq!(implementation_name("vector.bogus", &[]), None);
    }

    #[test]
    fn constant_axis_variants() {
        assert_eq!(
            constant_axis("zero", "Float", 3),
            Some(strings(&["0.0", "0.0", "0.0"]))
        );
        assert_eq!(
            constant_axis("one", "Integer", 2),
            Some(strings(&["1", "1"]))
        );
        assert_eq!(
            constant_axis("up", "Float", 3),
            Some(strings(&["0.0", "1.0", "0.0"]))
        );
        assert_eq!(
            constant_axis("right", "Float", 2),
            Some(strings(&["1.0", "0.0"]))
        );
        assert_eq!(
            constant_axis("forward", "Float", 3),
            Some(strings(&["0.0", "0.0", "1.0"]))
        );
        // forward undefined in 2D
        assert_eq!(constant_axis("forward", "Float", 2), None);
        assert_eq!(constant_axis("bogus", "Float", 3), None);
    }

    #[test]
    fn parse_constant_variants() {
        assert_eq!(
            parse_constant("zeroFloat3"),
            Some(("zero", "Float3".to_string()))
        );
        assert_eq!(
            parse_constant("upInteger2"),
            Some(("up", "Integer2".to_string()))
        );
        assert_eq!(parse_constant("bogus"), None);
        assert_eq!(parse_constant("zeroFloat5"), None);
    }

    #[test]
    fn is_vector_constant_variants() {
        assert!(is_vector_constant("vector.zeroFloat3"));
        assert!(is_vector_constant("vector.forwardFloat3"));
        // forward is undefined in 2D
        assert!(!is_vector_constant("vector.forwardFloat2"));
        // not a vector.* name
        assert!(!is_vector_constant("zeroFloat3"));
        assert!(!is_vector_constant("vector.bogus"));
    }

    #[test]
    fn constant_type_name_maps_all() {
        assert_eq!(constant_type_name("vector.zeroFloat2"), Some(FLOAT2_TYPE));
        assert_eq!(constant_type_name("vector.zeroFloat3"), Some(FLOAT3_TYPE));
        assert_eq!(constant_type_name("vector.zeroFloat4"), Some(FLOAT4_TYPE));
        assert_eq!(constant_type_name("vector.zeroFixed2"), Some(FIXED2_TYPE));
        assert_eq!(constant_type_name("vector.zeroFixed3"), Some(FIXED3_TYPE));
        assert_eq!(constant_type_name("vector.zeroFixed4"), Some(FIXED4_TYPE));
        assert_eq!(
            constant_type_name("vector.zeroInteger2"),
            Some(INTEGER2_TYPE)
        );
        assert_eq!(
            constant_type_name("vector.zeroInteger3"),
            Some(INTEGER3_TYPE)
        );
        assert_eq!(
            constant_type_name("vector.zeroInteger4"),
            Some(INTEGER4_TYPE)
        );
        assert_eq!(constant_type_name("vector.bogus"), None);
        assert_eq!(constant_type_name("notvector"), None);
    }

    #[test]
    fn constant_components_builds() {
        let (ty, comps) = constant_components("vector.upFloat3").expect("upFloat3");
        assert_eq!(ty, "Float3");
        assert_eq!(
            comps,
            vec![
                ("Float".to_string(), "0.0".to_string()),
                ("Float".to_string(), "1.0".to_string()),
                ("Float".to_string(), "0.0".to_string()),
            ]
        );
        assert_eq!(constant_components("vector.forwardFloat2"), None);
        assert_eq!(constant_components("vector.bogus"), None);
    }

    #[test]
    fn tostring_override_targets() {
        assert_eq!(
            tostring_override_target(FLOAT2_TYPE),
            Some("__vector_toString_float2")
        );
        assert_eq!(
            tostring_override_target(INTEGER4_TYPE),
            Some("__vector_toString_integer4")
        );
        assert_eq!(
            tostring_override_target(FIXED3_TYPE),
            Some("__vector_toString_fixed3")
        );
        assert_eq!(tostring_override_target("Nope"), None);
    }

    #[test]
    fn source_file_parses() {
        assert!(source_file().is_ok());
    }

    #[test]
    fn augmented_project_injects_when_imported() {
        let ast = project("IMPORT vector\nSUB main\nEND SUB\n");
        assert!(uses_package(&ast));
        let augmented = augmented_project(&ast).expect("augment");
        assert_eq!(augmented.files.len(), ast.files.len() + 1);
    }

    #[test]
    fn augmented_project_noop_without_import() {
        let ast = project("SUB main\nEND SUB\n");
        assert!(!uses_package(&ast));
        assert_eq!(
            augmented_project(&ast).expect("a").files.len(),
            ast.files.len()
        );
    }
}
