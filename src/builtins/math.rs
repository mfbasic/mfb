use std::borrow::Cow;

const PI: &str = "math.pi";
const PI_FIXED: &str = "math.piFixed";
const TWO_OVER_PI: &str = "math.twoOverPi";
const TWO_OVER_PI_FIXED: &str = "math.twoOverPiFixed";
const PI_2: &str = "math.pi2";
const PI_2_FIXED: &str = "math.pi2Fixed";
const PI_4: &str = "math.pi4";
const PI_4_FIXED: &str = "math.pi4Fixed";
const E: &str = "math.e";
const E_FIXED: &str = "math.eFixed";
const LN_2: &str = "math.ln2";
const LN_2_FIXED: &str = "math.ln2Fixed";
const LN_10: &str = "math.ln10";
const LN_10_FIXED: &str = "math.ln10Fixed";
const ABS: &str = "math.abs";
const MIN: &str = "math.min";
const MAX: &str = "math.max";
const CLAMP: &str = "math.clamp";
const FLOOR: &str = "math.floor";
const CEIL: &str = "math.ceil";
const ROUND: &str = "math.round";
const SQRT: &str = "math.sqrt";
const POW: &str = "math.pow";
const EXP: &str = "math.exp";
const LOG: &str = "math.log";
const LOG10: &str = "math.log10";
const SIN: &str = "math.sin";
const COS: &str = "math.cos";
const TAN: &str = "math.tan";
const ASIN: &str = "math.asin";
const ACOS: &str = "math.acos";
const ATAN: &str = "math.atan";
const ATAN2: &str = "math.atan2";
const RAND: &str = "math.rand";
const SEED: &str = "math.seed";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_math_call(name: &str) -> bool {
    matches!(
        name,
        ABS | MIN
            | MAX
            | CLAMP
            | FLOOR
            | CEIL
            | ROUND
            | SQRT
            | POW
            | EXP
            | LOG
            | LOG10
            | SIN
            | COS
            | TAN
            | ASIN
            | ACOS
            | ATAN
            | ATAN2
            | RAND
            | SEED
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        ABS | FLOOR | CEIL | ROUND | SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS
        | ATAN => Some(&[&["value"]]),
        MIN | MAX => Some(&[&["a", "left"], &["b", "right"]]),
        CLAMP => Some(&[&["value"], &["low", "minimum"], &["high", "maximum"]]),
        POW => Some(&[&["base", "value"], &["exponent", "power"]]),
        ATAN2 => Some(&[&["y"], &["x"]]),
        RAND => Some(&[&["min", "minimum"], &["max", "maximum"]]),
        SEED => Some(&[&["value", "seed"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        SQRT | POW | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN | ATAN2 => None,
        FLOOR | CEIL | ROUND => Some("Integer"),
        RAND => Some("Integer"),
        SEED => Some("Nothing"),
        _ => None,
    }
}

pub(crate) fn is_math_constant(name: &str) -> bool {
    matches!(
        name,
        PI | PI_FIXED
            | TWO_OVER_PI
            | TWO_OVER_PI_FIXED
            | PI_2
            | PI_2_FIXED
            | PI_4
            | PI_4_FIXED
            | E
            | E_FIXED
            | LN_2
            | LN_2_FIXED
            | LN_10
            | LN_10_FIXED
    )
}

pub(crate) fn constant_type_name(name: &str) -> Option<&'static str> {
    match name {
        PI | TWO_OVER_PI | PI_2 | PI_4 | E | LN_2 | LN_10 => Some("Float"),
        PI_FIXED | TWO_OVER_PI_FIXED | PI_2_FIXED | PI_4_FIXED | E_FIXED | LN_2_FIXED
        | LN_10_FIXED => Some("Fixed"),
        _ => None,
    }
}

pub(crate) fn constant_value(name: &str) -> Option<&'static str> {
    match name {
        PI => Some("3.141592653589793"),
        PI_FIXED => Some("3.141592653589793"),
        TWO_OVER_PI => Some("0.6366197723675814"),
        TWO_OVER_PI_FIXED => Some("0.6366197723675814"),
        PI_2 => Some("1.5707963267948966"),
        PI_2_FIXED => Some("1.5707963267948966"),
        PI_4 => Some("0.7853981633974483"),
        PI_4_FIXED => Some("0.7853981633974483"),
        E => Some("2.718281828459045"),
        E_FIXED => Some("2.718281828459045"),
        LN_2 => Some("0.6931471805599453"),
        LN_2_FIXED => Some("0.6931471805599453"),
        LN_10 => Some("2.302585092994046"),
        LN_10_FIXED => Some("2.302585092994046"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        ABS | MIN | MAX if all_same_numeric(arg_types, 1, 2) => {
            Cow::Borrowed(arg_types[0].as_str())
        }
        // Array (SIMD) overloads — plan-01-simd §4.2. The result list type equals
        // the (single, or two matching) argument list type.
        ABS if any_numeric_list(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        // sqrt over Float[] (NEON fsqrt) or Fixed[] (vectorized Q32.32 restoring
        // sqrt). log/log10 over Fixed[] are per-lane scalar Q32.32 (plan §4.5);
        // the Float transcendentals arrive in Phase 5.
        SQRT if one_numeric_list(arg_types, "Float") || one_numeric_list(arg_types, "Fixed") => {
            Cow::Borrowed(arg_types[0].as_str())
        }
        // log/log10 over Fixed[] (per-lane scalar Q32.32) or Float[] (NEON kernel).
        LOG | LOG10
            if one_numeric_list(arg_types, "Fixed") || one_numeric_list(arg_types, "Float") =>
        {
            Cow::Borrowed(arg_types[0].as_str())
        }
        // Float transcendental array kernels (plan-01-simd §4.6).
        EXP | SIN | COS | TAN | ATAN | ASIN | ACOS if one_numeric_list(arg_types, "Float") => {
            Cow::Borrowed(arg_types[0].as_str())
        }
        // Binary Float kernels: two same-length List OF Float.
        POW | ATAN2 if two_float_lists(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        FLOOR | CEIL | ROUND if one_floatish_list(arg_types) => Cow::Borrowed("List OF Integer"),
        MIN | MAX if two_same_numeric_lists(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        CLAMP if clamp_list(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        CLAMP if all_same_numeric(arg_types, 3, 3) => Cow::Borrowed(arg_types[0].as_str()),
        FLOOR | CEIL | ROUND if one_floatish(arg_types) => Cow::Borrowed("Integer"),
        SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN
            if one_float_or_fixed(arg_types) =>
        {
            Cow::Borrowed(arg_types[0].as_str())
        }
        POW | ATAN2 if two_same_float_or_fixed(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        RAND if two_integers(arg_types) => Cow::Borrowed("Integer"),
        SEED if one_integer(arg_types) => Cow::Borrowed("Nothing"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        ABS => Some("Integer | Float | Fixed"),
        FLOOR | CEIL | ROUND => Some("Float | Fixed"),
        SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN => Some("Float | Fixed"),
        MIN | MAX => Some("same numeric type, same numeric type"),
        POW | ATAN2 => Some("Float | Fixed, same type"),
        CLAMP => Some("numeric value, numeric low, numeric high of the same type"),
        RAND => Some("Integer min, Integer max"),
        SEED => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        ABS | FLOOR | CEIL | ROUND | SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS
        | ATAN | SEED => Some((1, 1)),
        MIN | MAX | POW | ATAN2 | RAND => Some((2, 2)),
        CLAMP => Some((3, 3)),
        _ => None,
    }
}

fn all_same_numeric(arg_types: &[String], min: usize, max: usize) -> bool {
    (min..=max).contains(&arg_types.len())
        && arg_types.first().is_some_and(|first| is_numeric(first))
        && arg_types.iter().all(|type_| type_ == &arg_types[0])
}

fn one_floatish(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && matches!(arg_types[0].as_str(), "Float" | "Fixed")
}

fn one_float_or_fixed(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && matches!(arg_types[0].as_str(), "Float" | "Fixed")
}

fn one_integer(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && arg_types[0] == "Integer"
}

fn two_integers(arg_types: &[String]) -> bool {
    arg_types.len() == 2 && arg_types[0] == "Integer" && arg_types[1] == "Integer"
}

fn two_same_float_or_fixed(arg_types: &[String]) -> bool {
    arg_types.len() == 2
        && matches!(arg_types[0].as_str(), "Float" | "Fixed")
        && arg_types[0] == arg_types[1]
}

fn is_numeric(type_name: &str) -> bool {
    matches!(type_name, "Integer" | "Float" | "Fixed")
}

/// A single `List OF <element>` argument (the unary SIMD array overloads).
/// `element` is one of `Integer`/`Float`/`Fixed`.
fn one_numeric_list(arg_types: &[String], element: &str) -> bool {
    arg_types.len() == 1 && arg_types[0] == format!("List OF {element}")
}

/// A single `List OF Float` or `List OF Fixed` (the array rounding overloads).
fn one_floatish_list(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && matches!(arg_types[0].as_str(), "List OF Float" | "List OF Fixed")
}

/// A single homogeneous numeric list argument of any numeric element type.
fn any_numeric_list(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && is_numeric_list(&arg_types[0])
}

fn is_numeric_list(type_: &str) -> bool {
    matches!(type_, "List OF Integer" | "List OF Float" | "List OF Fixed")
}

/// Two `List OF Float` arguments (the binary Float kernels `pow`/`atan2`).
fn two_float_lists(arg_types: &[String]) -> bool {
    arg_types.len() == 2 && arg_types[0] == "List OF Float" && arg_types[1] == "List OF Float"
}

/// Two arguments that are the same numeric list type (two-array `min`/`max`).
fn two_same_numeric_lists(arg_types: &[String]) -> bool {
    arg_types.len() == 2 && is_numeric_list(&arg_types[0]) && arg_types[0] == arg_types[1]
}

/// `(List OF T, T, T)` for a numeric `T` (the array `clamp` overload): a numeric
/// list followed by two scalar bounds of the matching element type.
fn clamp_list(arg_types: &[String]) -> bool {
    arg_types.len() == 3
        && is_numeric_list(&arg_types[0])
        && arg_types[0]
            .strip_prefix("List OF ")
            .is_some_and(|element| arg_types[1] == element && arg_types[2] == element)
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

    #[test]
    fn is_math_call_flags() {
        for f in [
            ABS, MIN, MAX, CLAMP, FLOOR, CEIL, ROUND, SQRT, POW, EXP, LOG, LOG10, SIN, COS, TAN,
            ASIN, ACOS, ATAN, ATAN2, RAND, SEED,
        ] {
            assert!(is_math_call(f), "{f}");
        }
        assert!(!is_math_call(PI));
        assert!(!is_math_call("math.bogus"));
    }

    #[test]
    fn is_math_constant_flags() {
        for c in [
            PI,
            PI_FIXED,
            TWO_OVER_PI,
            TWO_OVER_PI_FIXED,
            PI_2,
            PI_2_FIXED,
            PI_4,
            PI_4_FIXED,
            E,
            E_FIXED,
            LN_2,
            LN_2_FIXED,
            LN_10,
            LN_10_FIXED,
        ] {
            assert!(is_math_constant(c), "{c}");
        }
        assert!(!is_math_constant(ABS));
        assert!(!is_math_constant("math.bogus"));
    }

    #[test]
    fn constant_type_names() {
        assert_eq!(constant_type_name(PI), Some("Float"));
        assert_eq!(constant_type_name(E), Some("Float"));
        assert_eq!(constant_type_name(LN_10), Some("Float"));
        assert_eq!(constant_type_name(PI_FIXED), Some("Fixed"));
        assert_eq!(constant_type_name(E_FIXED), Some("Fixed"));
        assert_eq!(constant_type_name(LN_10_FIXED), Some("Fixed"));
        assert_eq!(constant_type_name(ABS), None);
    }

    #[test]
    fn constant_values() {
        assert_eq!(constant_value(PI), Some("3.141592653589793"));
        assert_eq!(constant_value(PI_FIXED), Some("3.141592653589793"));
        assert_eq!(constant_value(TWO_OVER_PI), Some("0.6366197723675814"));
        assert_eq!(
            constant_value(TWO_OVER_PI_FIXED),
            Some("0.6366197723675814")
        );
        assert_eq!(constant_value(PI_2), Some("1.5707963267948966"));
        assert_eq!(constant_value(PI_2_FIXED), Some("1.5707963267948966"));
        assert_eq!(constant_value(PI_4), Some("0.7853981633974483"));
        assert_eq!(constant_value(PI_4_FIXED), Some("0.7853981633974483"));
        assert_eq!(constant_value(E), Some("2.718281828459045"));
        assert_eq!(constant_value(E_FIXED), Some("2.718281828459045"));
        assert_eq!(constant_value(LN_2), Some("0.6931471805599453"));
        assert_eq!(constant_value(LN_2_FIXED), Some("0.6931471805599453"));
        assert_eq!(constant_value(LN_10), Some("2.302585092994046"));
        assert_eq!(constant_value(LN_10_FIXED), Some("2.302585092994046"));
        assert_eq!(constant_value(ABS), None);
    }

    #[test]
    fn call_param_names_shapes() {
        assert_eq!(call_param_names(ABS), Some(&[&["value"][..]][..]));
        assert!(call_param_names(MIN).is_some());
        assert!(call_param_names(CLAMP).is_some());
        assert!(call_param_names(POW).is_some());
        assert!(call_param_names(ATAN2).is_some());
        assert!(call_param_names(RAND).is_some());
        assert!(call_param_names(SEED).is_some());
        assert_eq!(call_param_names("math.bogus"), None);
    }

    #[test]
    fn call_return_type_names() {
        assert_eq!(call_return_type_name(FLOOR), Some("Integer"));
        assert_eq!(call_return_type_name(CEIL), Some("Integer"));
        assert_eq!(call_return_type_name(ROUND), Some("Integer"));
        assert_eq!(call_return_type_name(RAND), Some("Integer"));
        assert_eq!(call_return_type_name(SEED), Some("Nothing"));
        // scalar-carrying transcendentals depend on arg type -> None nominal
        assert_eq!(call_return_type_name(SQRT), None);
        assert_eq!(call_return_type_name(POW), None);
        assert_eq!(call_return_type_name(ABS), None);
        assert_eq!(call_return_type_name("math.bogus"), None);
    }

    #[test]
    fn resolve_abs_min_max_scalar() {
        assert_eq!(ret(ABS, &["Integer"]), Some("Integer".to_string()));
        assert_eq!(ret(ABS, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(ABS, &["Fixed"]), Some("Fixed".to_string()));
        assert_eq!(
            ret(MIN, &["Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(MAX, &["Float", "Float"]), Some("Float".to_string()));
        // mismatched numeric types
        assert_eq!(ret(MIN, &["Integer", "Float"]), None);
        // non-numeric
        assert_eq!(ret(ABS, &["String"]), None);
        assert_eq!(ret(ABS, &[]), None);
    }

    #[test]
    fn resolve_abs_array() {
        assert_eq!(
            ret(ABS, &["List OF Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            ret(ABS, &["List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(
            ret(ABS, &["List OF Fixed"]),
            Some("List OF Fixed".to_string())
        );
    }

    #[test]
    fn resolve_sqrt_and_transcendentals_scalar() {
        assert_eq!(ret(SQRT, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(SQRT, &["Fixed"]), Some("Fixed".to_string()));
        assert_eq!(ret(SIN, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(LOG, &["Fixed"]), Some("Fixed".to_string()));
        assert_eq!(ret(LOG10, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(EXP, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(ATAN, &["Float"]), Some("Float".to_string()));
        assert_eq!(ret(ACOS, &["Fixed"]), Some("Fixed".to_string()));
        // Integer not allowed for transcendentals
        assert_eq!(ret(SQRT, &["Integer"]), None);
    }

    #[test]
    fn resolve_sqrt_and_log_arrays() {
        assert_eq!(
            ret(SQRT, &["List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(
            ret(SQRT, &["List OF Fixed"]),
            Some("List OF Fixed".to_string())
        );
        assert_eq!(
            ret(LOG, &["List OF Fixed"]),
            Some("List OF Fixed".to_string())
        );
        assert_eq!(
            ret(LOG10, &["List OF Float"]),
            Some("List OF Float".to_string())
        );
        // exp/sin/etc array only over Float
        assert_eq!(
            ret(EXP, &["List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(
            ret(SIN, &["List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(ret(EXP, &["List OF Fixed"]), None);
    }

    #[test]
    fn resolve_binary_float_kernels() {
        assert_eq!(
            ret(POW, &["List OF Float", "List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(
            ret(ATAN2, &["List OF Float", "List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(ret(POW, &["Float", "Float"]), Some("Float".to_string()));
        assert_eq!(ret(ATAN2, &["Fixed", "Fixed"]), Some("Fixed".to_string()));
        assert_eq!(ret(POW, &["Float", "Fixed"]), None);
        assert_eq!(ret(POW, &["Integer", "Integer"]), None);
    }

    #[test]
    fn resolve_rounding() {
        assert_eq!(ret(FLOOR, &["Float"]), Some("Integer".to_string()));
        assert_eq!(ret(CEIL, &["Fixed"]), Some("Integer".to_string()));
        assert_eq!(ret(ROUND, &["Float"]), Some("Integer".to_string()));
        assert_eq!(
            ret(FLOOR, &["List OF Float"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(
            ret(ROUND, &["List OF Fixed"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(ret(FLOOR, &["Integer"]), None);
    }

    #[test]
    fn resolve_min_max_arrays() {
        assert_eq!(
            ret(MIN, &["List OF Float", "List OF Float"]),
            Some("List OF Float".to_string())
        );
        assert_eq!(
            ret(MAX, &["List OF Integer", "List OF Integer"]),
            Some("List OF Integer".to_string())
        );
        assert_eq!(ret(MIN, &["List OF Float", "List OF Integer"]), None);
    }

    #[test]
    fn resolve_clamp() {
        assert_eq!(
            ret(CLAMP, &["Float", "Float", "Float"]),
            Some("Float".to_string())
        );
        assert_eq!(
            ret(CLAMP, &["Integer", "Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            ret(CLAMP, &["List OF Float", "Float", "Float"]),
            Some("List OF Float".to_string())
        );
        // mismatched bounds
        assert_eq!(ret(CLAMP, &["List OF Float", "Float", "Integer"]), None);
        assert_eq!(ret(CLAMP, &["Float", "Float", "Integer"]), None);
    }

    #[test]
    fn resolve_rand_seed() {
        assert_eq!(
            ret(RAND, &["Integer", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(RAND, &["Float", "Float"]), None);
        assert_eq!(ret(SEED, &["Integer"]), Some("Nothing".to_string()));
        assert_eq!(ret(SEED, &["Float"]), None);
        assert_eq!(ret("math.bogus", &["Integer"]), None);
    }

    #[test]
    fn expected_arguments_present() {
        assert!(expected_arguments(ABS).unwrap().contains("Integer"));
        assert!(expected_arguments(FLOOR).unwrap().contains("Float"));
        assert!(expected_arguments(SQRT).is_some());
        assert!(expected_arguments(MIN).is_some());
        assert!(expected_arguments(POW).is_some());
        assert!(expected_arguments(CLAMP).is_some());
        assert!(expected_arguments(RAND).is_some());
        assert_eq!(expected_arguments(SEED), Some("Integer"));
        assert_eq!(expected_arguments("math.bogus"), None);
    }

    #[test]
    fn arity_spans() {
        assert_eq!(arity(ABS), Some((1, 1)));
        assert_eq!(arity(SEED), Some((1, 1)));
        assert_eq!(arity(MIN), Some((2, 2)));
        assert_eq!(arity(POW), Some((2, 2)));
        assert_eq!(arity(RAND), Some((2, 2)));
        assert_eq!(arity(CLAMP), Some((3, 3)));
        assert_eq!(arity("math.bogus"), None);
    }

    #[test]
    fn numeric_helpers() {
        assert!(is_numeric("Integer"));
        assert!(is_numeric("Float"));
        assert!(is_numeric("Fixed"));
        assert!(!is_numeric("String"));
        assert!(is_numeric_list("List OF Integer"));
        assert!(!is_numeric_list("List OF String"));
        assert!(one_floatish(&strings(&["Float"])));
        assert!(!one_floatish(&strings(&["Integer"])));
        assert!(one_integer(&strings(&["Integer"])));
        assert!(!one_integer(&strings(&["Float"])));
        assert!(two_integers(&strings(&["Integer", "Integer"])));
        assert!(!two_integers(&strings(&["Integer", "Float"])));
        assert!(one_numeric_list(&strings(&["List OF Float"]), "Float"));
        assert!(!one_numeric_list(&strings(&["List OF Float"]), "Fixed"));
    }
}
