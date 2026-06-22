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
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        SQRT | POW | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN | ATAN2 => None,
        FLOOR | CEIL | ROUND => Some("Integer"),
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
        CLAMP if all_same_numeric(arg_types, 3, 3) => Cow::Borrowed(arg_types[0].as_str()),
        FLOOR | CEIL | ROUND if one_floatish(arg_types) => Cow::Borrowed("Integer"),
        SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN
            if one_float_or_fixed(arg_types) =>
        {
            Cow::Borrowed(arg_types[0].as_str())
        }
        POW | ATAN2 if two_same_float_or_fixed(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
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
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        ABS | FLOOR | CEIL | ROUND | SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS
        | ATAN => Some((1, 1)),
        MIN | MAX | POW | ATAN2 => Some((2, 2)),
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

fn two_same_float_or_fixed(arg_types: &[String]) -> bool {
    arg_types.len() == 2
        && matches!(arg_types[0].as_str(), "Float" | "Fixed")
        && arg_types[0] == arg_types[1]
}

fn is_numeric(type_name: &str) -> bool {
    matches!(type_name, "Integer" | "Float" | "Fixed")
}
