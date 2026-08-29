//! `strings.repeat` — descriptor + clean-room native lowering.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Concatenate a string with itself a given number of times."#;

const DESC: &str = r#"`strings::repeat` returns a new `String` made of `times` consecutive copies of
`value`, written end to end with nothing inserted between them.

Copying works on the raw UTF-8 bytes of `value`, so every multi-byte scalar and
every grapheme cluster is reproduced intact in each copy — `repeat` never splits
a character. The byte length of the result is exactly
`strings::byteLen(value) * times`.

A `times` of `0` returns the empty string regardless of `value`, and a `times` of
`1` returns a copy equal to `value`. Repeating the empty string yields the empty
string for any valid `times`. A negative `times` is rejected with
`ErrInvalidArgument`.

The total size is computed with overflow checks. A `byteLen(value) * times`
product, or the string header added to it, that cannot be represented in 64 bits
raises the same `ErrInvalidArgument` rather than allocating short and writing
past the buffer.

`value` is not mutated; the result is a new owned `String`.

`value` may also be an `astrings::AttributedString`: it returns an
`AttributedString` whose text is transformed exactly as the `String` overload's
and whose attribute spans are remapped by the same edit."#;

const EX: &str = r#"Repeat a short string; zero copies yields the empty string:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::repeat("ab", 3))
  io::print("[" & strings::repeat("x", 0) & "]")
  RETURN 0
END FUNC
```

Build a horizontal rule:

```
IMPORT io
IMPORT strings

FUNC main() AS Integer
  io::print(strings::repeat("-", 40))
  RETURN 0
END FUNC
```"#;

pub(crate) fn lower(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    if args.len() != 2 {
        return Err("strings.repeat: no native lowering for these arguments".to_string());
    }
    let value = &args[0];
    let times = &args[1];

    let value = value.clone();
    builder.require_string("strings.repeat value", &value)?;
    let value_slot = builder.spill_to_slot("strings_repeat_value", &value.location);
    let times = times.clone();
    if times.type_ != ParameterType::Integer {
        return Err(format!(
            "strings.repeat times must be Integer, got {}",
            times.type_
        ));
    }
    let times_slot = builder.spill_to_slot("strings_repeat_times", &times.location);
    let total_slot = builder.allocate_stack_object("strings_repeat_total", 8);
    let result_slot = builder.allocate_stack_object("strings_repeat_result", 8);

    let invalid = builder.label("strings_repeat_invalid");
    let alloc_ok = builder.label("strings_repeat_alloc_ok");
    let outer = builder.label("strings_repeat_outer");
    let inner = builder.label("strings_repeat_inner");
    let inner_done = builder.label("strings_repeat_inner_done");
    let outer_done = builder.label("strings_repeat_outer_done");

    // Scratch as vregs. The arena_alloc ABI arg/result register stays
    // physical only across that call; the allocation pointer is then carried
    // in a neutral vreg across the copy loops, since a held physical result
    // register is fragile on ISAs whose result/argument registers differ
    // (x86-64).
    let val_ptr_v = builder.temporary_vreg();
    let times_rem_v = builder.temporary_vreg();
    let len_v = builder.temporary_vreg();
    let total_v = builder.temporary_vreg();
    let dst_v = builder.temporary_vreg();
    let src_base_v = builder.temporary_vreg();
    let inner_src_v = builder.temporary_vreg();
    let inner_cnt_v = builder.temporary_vreg();
    let byte_v = builder.temporary_vreg();
    let val_ptr = &val_ptr_v;
    let times_rem = &times_rem_v;
    let len = &len_v;
    let total = &total_v;
    let dst = &dst_v;
    let src_base = &src_base_v;
    let inner_src = &inner_src_v;
    let inner_cnt = &inner_cnt_v;
    let byte = &byte_v;

    builder.emit(abi::load_u64(val_ptr, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(times_rem, abi::stack_pointer(), times_slot));
    builder.emit(abi::compare_immediate(times_rem, "0"));
    builder.emit(abi::branch_lt(&invalid));
    builder.emit(abi::load_u64(len, val_ptr, 0));
    // total = len * times, rejecting products (and the +9 header below) that
    // do not fit in 64 bits: an unchecked wrap here allocated small while the
    // copy loop wrote the full len*times bytes (audit-unicode #1, heap
    // overflow). Unrepresentable sizes raise the same catchable 77050002 as
    // the other argument rejections.
    builder.emit_checked_size_multiply(total, len, times_rem, &invalid);
    builder.emit(abi::store_u64(total, abi::stack_pointer(), total_slot));
    // allocate total + 9.
    builder.emit_checked_size_add_immediate(abi::return_register(), total, 9, &invalid);
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::label(&alloc_ok));
    // Capture the allocation result while x1 is unambiguously the call result.
    let result_ptr = builder.allocate_register();
    builder.emit(abi::move_register(&result_ptr, abi::mfb_return(1)));
    builder.emit(abi::store_u64(
        &result_ptr,
        abi::stack_pointer(),
        result_slot,
    ));
    builder.emit(abi::load_u64(total, abi::stack_pointer(), total_slot));
    builder.emit(abi::store_u64(total, &result_ptr, 0));

    // Copy loop: times_rem outer counter, dst cursor, src_base, len.
    builder.emit(abi::load_u64(val_ptr, abi::stack_pointer(), value_slot));
    builder.emit(abi::load_u64(times_rem, abi::stack_pointer(), times_slot));
    builder.emit(abi::load_u64(len, val_ptr, 0));
    builder.emit(abi::add_immediate(src_base, val_ptr, 8));
    builder.emit(abi::add_immediate(dst, &result_ptr, 8));
    builder.emit(abi::label(&outer));
    builder.emit(abi::compare_immediate(times_rem, "0"));
    builder.emit(abi::branch_eq(&outer_done));
    // inner: copy len bytes from src_base to dst.
    builder.emit(abi::move_register(inner_src, src_base));
    builder.emit(abi::move_register(inner_cnt, len));
    builder.emit(abi::label(&inner));
    builder.emit(abi::compare_immediate(inner_cnt, "0"));
    builder.emit(abi::branch_eq(&inner_done));
    builder.emit(abi::load_u8(byte, inner_src, 0));
    builder.emit(abi::store_u8(byte, dst, 0));
    builder.emit(abi::add_immediate(inner_src, inner_src, 1));
    builder.emit(abi::add_immediate(dst, dst, 1));
    builder.emit(abi::subtract_immediate(inner_cnt, inner_cnt, 1));
    builder.emit(abi::branch(&inner));
    builder.emit(abi::label(&inner_done));
    builder.emit(abi::subtract_immediate(times_rem, times_rem, 1));
    builder.emit(abi::branch(&outer));
    builder.emit(abi::label(&outer_done));
    builder.emit(abi::move_immediate(byte, "Integer", "0"));
    builder.emit(abi::store_u8(byte, dst, 0));
    let result = builder.allocate_register();
    builder.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
    let after = builder.label("strings_repeat_after");
    builder.emit(abi::branch(&after));
    builder.emit(abi::label(&invalid));
    builder.raise_error("strings.repeat", "ErrInvalidArgument")?;
    builder.emit(abi::label(&after));
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::String,
        location: Operand::from(result.render()),
        text: "strings.repeat".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "repeat",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "value",
                    desc: "The string to repeat. Any `String`, including the empty one.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "times",
                    desc: "The number of copies to concatenate. Must be `0` or greater. `0` yields `\"\"` and `1` yields a copy of `value`.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::String,
            errors: vec!["ErrInvalidArgument"],
            body: Body::abi_inline(lower),
        }],
    });
}
