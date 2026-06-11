use crate::bytecode::{BuiltinCallLowerer, ValueSlot, OPCODE_WRITE_STDOUT, TYPE_NOTHING};
use crate::ir::IrValue;
use std::collections::HashMap;

pub(crate) const NAME: &str = "io.print";
pub(crate) const PARAM_TYPE: &str = "String";
pub(crate) const RETURN_TYPE: &str = "Nothing";

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    if args.len() != 1 {
        return Err(format!(
            "built-in `{NAME}` has {} argument(s), expected 1",
            args.len()
        ));
    }

    let arg = lowerer.lower_value(&args[0], locals)?;
    if arg.type_name != PARAM_TYPE {
        return Err(format!(
            "built-in `{NAME}` argument has type {}, expected {PARAM_TYPE}",
            arg.type_name
        ));
    }

    lowerer.push(OPCODE_WRITE_STDOUT, vec![arg.register, 1]);

    let result = lowerer.add_register(TYPE_NOTHING, 0);
    lowerer.push_default(result, TYPE_NOTHING);
    Ok(ValueSlot {
        register: result,
        type_name: RETURN_TYPE.to_string(),
    })
}
