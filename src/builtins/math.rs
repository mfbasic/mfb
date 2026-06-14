use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_MATH_ABS, OPCODE_MATH_ACOS, OPCODE_MATH_ASIN,
    OPCODE_MATH_ATAN, OPCODE_MATH_ATAN2, OPCODE_MATH_CEIL, OPCODE_MATH_CLAMP, OPCODE_MATH_COS,
    OPCODE_MATH_DEGREES, OPCODE_MATH_E, OPCODE_MATH_EXP, OPCODE_MATH_FLOOR, OPCODE_MATH_IS_FINITE,
    OPCODE_MATH_LOG, OPCODE_MATH_LOG10, OPCODE_MATH_MAX, OPCODE_MATH_MIN, OPCODE_MATH_PI,
    OPCODE_MATH_POW, OPCODE_MATH_RADIANS, OPCODE_MATH_ROUND, OPCODE_MATH_SIGN, OPCODE_MATH_SIN,
    OPCODE_MATH_SQRT, OPCODE_MATH_TAN, OPCODE_MATH_TRUNC, TYPE_BOOLEAN, TYPE_FIXED, TYPE_FLOAT,
    TYPE_INTEGER,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "math";

const PI: &str = "math.pi";
const PI_FLOAT: &str = "math.piFloat";
const PI_FIXED: &str = "math.piFixed";
const TWO_PI: &str = "math.2pi";
const TWO_PI_FIXED: &str = "math.2piFixed";
const PI_2: &str = "math.pi2";
const PI_2_FIXED: &str = "math.pi2Fixed";
const PI_4: &str = "math.pi4";
const PI_4_FIXED: &str = "math.pi4Fixed";
const E: &str = "math.e";
const E_FLOAT: &str = "math.eFloat";
const E_FIXED: &str = "math.eFixed";
const LN_2: &str = "math.ln2";
const LN_2_FIXED: &str = "math.ln2Fixed";
const LN_10: &str = "math.ln10";
const LN_10_FIXED: &str = "math.ln10Fixed";
const ABS: &str = "math.abs";
const SIGN: &str = "math.sign";
const MIN: &str = "math.min";
const MAX: &str = "math.max";
const CLAMP: &str = "math.clamp";
const FLOOR: &str = "math.floor";
const CEIL: &str = "math.ceil";
const ROUND: &str = "math.round";
const TRUNC: &str = "math.trunc";
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
const RADIANS: &str = "math.radians";
const DEGREES: &str = "math.degrees";
const IS_FINITE: &str = "math.isFinite";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_math_call(name: &str) -> bool {
    matches!(
        name,
        PI
            | PI_FLOAT
            | PI_FIXED
            | TWO_PI
            | TWO_PI_FIXED
            | PI_2
            | PI_2_FIXED
            | PI_4
            | PI_4_FIXED
            | E
            | E_FLOAT
            | E_FIXED
            | LN_2
            | LN_2_FIXED
            | LN_10
            | LN_10_FIXED
            | ABS
            | SIGN
            | MIN
            | MAX
            | CLAMP
            | FLOOR
            | CEIL
            | ROUND
            | TRUNC
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
            | RADIANS
            | DEGREES
            | IS_FINITE
    )
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        PI | PI_FLOAT | TWO_PI | PI_2 | PI_4 | E | E_FLOAT | LN_2 | LN_10 => Some("Float"),
        PI_FIXED | TWO_PI_FIXED | PI_2_FIXED | PI_4_FIXED | E_FIXED | LN_2_FIXED
        | LN_10_FIXED => Some("Fixed"),
        SQRT | POW | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN | ATAN2 => None,
        RADIANS | DEGREES => Some("Float"),
        FLOOR | CEIL | ROUND | TRUNC | SIGN => Some("Integer"),
        IS_FINITE => Some("Boolean"),
        _ => None,
    }
}

pub(crate) fn is_math_constant(name: &str) -> bool {
    matches!(
        name,
        PI
            | PI_FLOAT
            | PI_FIXED
            | TWO_PI
            | TWO_PI_FIXED
            | PI_2
            | PI_2_FIXED
            | PI_4
            | PI_4_FIXED
            | E
            | E_FLOAT
            | E_FIXED
            | LN_2
            | LN_2_FIXED
            | LN_10
            | LN_10_FIXED
    )
}

pub(crate) fn constant_type_name(name: &str) -> Option<&'static str> {
    match name {
        PI | PI_FLOAT | TWO_PI | PI_2 | PI_4 | E | E_FLOAT | LN_2 | LN_10 => Some("Float"),
        PI_FIXED | TWO_PI_FIXED | PI_2_FIXED | PI_4_FIXED | E_FIXED | LN_2_FIXED
        | LN_10_FIXED => Some("Fixed"),
        _ => None,
    }
}

pub(crate) fn constant_value(name: &str) -> Option<&'static str> {
    match name {
        PI | PI_FLOAT => Some("3.141592653589793"),
        PI_FIXED => Some("3.141592653589793"),
        TWO_PI => Some("0.6366197723675814"),
        TWO_PI_FIXED => Some("0.6366197723675814"),
        PI_2 => Some("1.5707963267948966"),
        PI_2_FIXED => Some("1.5707963267948966"),
        PI_4 => Some("0.7853981633974483"),
        PI_4_FIXED => Some("0.7853981633974483"),
        E | E_FLOAT => Some("2.718281828459045"),
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
        PI | PI_FLOAT | TWO_PI | PI_2 | PI_4 | E | E_FLOAT | LN_2 | LN_10
            if arg_types.is_empty() =>
        {
            Cow::Borrowed("Float")
        }
        PI_FIXED | TWO_PI_FIXED | PI_2_FIXED | PI_4_FIXED | E_FIXED | LN_2_FIXED | LN_10_FIXED
            if arg_types.is_empty() =>
        {
            Cow::Borrowed("Fixed")
        }
        ABS | MIN | MAX if all_same_numeric(arg_types, 1, 2) => {
            Cow::Borrowed(arg_types[0].as_str())
        }
        CLAMP if all_same_numeric(arg_types, 3, 3) => Cow::Borrowed(arg_types[0].as_str()),
        SIGN if one_numeric(arg_types) => Cow::Borrowed("Integer"),
        FLOOR | CEIL | ROUND | TRUNC if one_floatish(arg_types) => Cow::Borrowed("Integer"),
        SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN
            if one_float_or_fixed(arg_types) =>
        {
            Cow::Borrowed(arg_types[0].as_str())
        }
        RADIANS | DEGREES if one_numeric(arg_types) => Cow::Borrowed("Float"),
        POW | ATAN2 if two_same_float_or_fixed(arg_types) => Cow::Borrowed(arg_types[0].as_str()),
        IS_FINITE if one_numeric(arg_types) => Cow::Borrowed("Boolean"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        PI | PI_FLOAT | PI_FIXED | TWO_PI | TWO_PI_FIXED | PI_2 | PI_2_FIXED | PI_4
        | PI_4_FIXED | E | E_FLOAT | E_FIXED | LN_2 | LN_2_FIXED | LN_10 | LN_10_FIXED => {
            Some("no arguments")
        }
        ABS | SIGN | IS_FINITE => Some("Integer | Float | Fixed"),
        FLOOR | CEIL | ROUND | TRUNC => Some("Float | Fixed"),
        SQRT | EXP | LOG | LOG10 | SIN | COS | TAN | ASIN | ACOS | ATAN => Some("Float | Fixed"),
        RADIANS | DEGREES => Some("Integer | Float | Fixed"),
        MIN | MAX => Some("same numeric type, same numeric type"),
        POW | ATAN2 => Some("Float | Fixed, same type"),
        CLAMP => Some("numeric value, numeric low, numeric high of the same type"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        PI | PI_FLOAT | PI_FIXED | TWO_PI | TWO_PI_FIXED | PI_2 | PI_2_FIXED | PI_4
        | PI_4_FIXED | E | E_FLOAT | E_FIXED | LN_2 | LN_2_FIXED | LN_10 | LN_10_FIXED => {
            Some((0, 0))
        }
        ABS | SIGN | FLOOR | CEIL | ROUND | TRUNC | SQRT | EXP | LOG | LOG10 | SIN | COS | TAN
        | ASIN | ACOS | ATAN | RADIANS | DEGREES | IS_FINITE => Some((1, 1)),
        MIN | MAX | POW | ATAN2 => Some((2, 2)),
        CLAMP => Some((3, 3)),
        _ => None,
    }
}

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    name: &str,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    let lowered = args
        .iter()
        .map(|arg| lowerer.lower_value(arg, locals))
        .collect::<Result<Vec<_>, _>>()?;
    let arg_types = lowered
        .iter()
        .map(|slot| slot.type_name.clone())
        .collect::<Vec<_>>();
    let resolved = resolve_call(name, &arg_types).ok_or_else(|| {
        format!(
            "built-in `{name}` does not accept ({})",
            arg_types.join(", ")
        )
    })?;

    let dst_type_id = primitive_type_id(&resolved.return_type)
        .unwrap_or_else(|| lowerer.type_id(&resolved.return_type));
    let dst = lowerer.add_register(dst_type_id, 0);
    let mut operands = vec![dst];
    operands.extend(lowered.iter().map(|slot| slot.register));
    lowerer.push(opcode_for(name)?, operands);
    Ok(ValueSlot {
        register: dst,
        type_name: resolved.return_type.into_owned(),
    })
}

fn opcode_for(name: &str) -> Result<u16, String> {
    match name {
        PI | PI_FLOAT | PI_FIXED => Ok(OPCODE_MATH_PI),
        E | E_FLOAT | E_FIXED => Ok(OPCODE_MATH_E),
        ABS => Ok(OPCODE_MATH_ABS),
        SIGN => Ok(OPCODE_MATH_SIGN),
        MIN => Ok(OPCODE_MATH_MIN),
        MAX => Ok(OPCODE_MATH_MAX),
        CLAMP => Ok(OPCODE_MATH_CLAMP),
        FLOOR => Ok(OPCODE_MATH_FLOOR),
        CEIL => Ok(OPCODE_MATH_CEIL),
        ROUND => Ok(OPCODE_MATH_ROUND),
        TRUNC => Ok(OPCODE_MATH_TRUNC),
        SQRT => Ok(OPCODE_MATH_SQRT),
        POW => Ok(OPCODE_MATH_POW),
        EXP => Ok(OPCODE_MATH_EXP),
        LOG => Ok(OPCODE_MATH_LOG),
        LOG10 => Ok(OPCODE_MATH_LOG10),
        SIN => Ok(OPCODE_MATH_SIN),
        COS => Ok(OPCODE_MATH_COS),
        TAN => Ok(OPCODE_MATH_TAN),
        ASIN => Ok(OPCODE_MATH_ASIN),
        ACOS => Ok(OPCODE_MATH_ACOS),
        ATAN => Ok(OPCODE_MATH_ATAN),
        ATAN2 => Ok(OPCODE_MATH_ATAN2),
        RADIANS => Ok(OPCODE_MATH_RADIANS),
        DEGREES => Ok(OPCODE_MATH_DEGREES),
        IS_FINITE => Ok(OPCODE_MATH_IS_FINITE),
        _ => Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    }
}

fn all_same_numeric(arg_types: &[String], min: usize, max: usize) -> bool {
    (min..=max).contains(&arg_types.len())
        && arg_types.first().is_some_and(|first| is_numeric(first))
        && arg_types.iter().all(|type_| type_ == &arg_types[0])
}

fn one_numeric(arg_types: &[String]) -> bool {
    arg_types.len() == 1 && is_numeric(&arg_types[0])
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

fn primitive_type_id(type_name: &str) -> Option<u32> {
    match type_name {
        "Boolean" => Some(TYPE_BOOLEAN),
        "Integer" => Some(TYPE_INTEGER),
        "Float" => Some(TYPE_FLOAT),
        "Fixed" => Some(TYPE_FIXED),
        _ => None,
    }
}
