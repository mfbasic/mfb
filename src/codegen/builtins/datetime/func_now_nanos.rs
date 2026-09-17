//! `datetime::nowNanos` — descriptor entry + authored docs, and the per-member
//! `abi_function` lowering ([`lower_now_nanos`]). The wrapper finalizes it
//! (crypto/io's clean-room shape).

use super::gen_shared::{
    emit_clock_overflow_tail, emit_libc_clock_nanos, void_int_result, CLOCK_REALTIME,
    LOCALS_SIZE, WIN_FILETIME_OFFSET, WIN_FILETIME_UNIX_EPOCH_100NS,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `abi_function` body for `datetime::nowNanos` — the current wall-clock reading in
/// nanoseconds since the Unix epoch. On libc platforms it rides the shared
/// [`emit_libc_clock_nanos`] with `CLOCK_REALTIME`; on Windows it reads
/// `GetSystemTimePreciseAsFileTime` (100 ns intervals since 1601) and rebases to
/// Unix nanoseconds (plan-66-A). A reading whose nanosecond count does not fit an
/// `Integer` (past 2262 or before 1678) raises `ErrOverflow` (bug-640).
pub(crate) fn lower_now_nanos(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let platform = ctx.platform;
    let platform_imports = ctx.platform_imports;
    let mut instructions: Vec<CodeInstruction> = Vec::new();
    let mut relocations = Vec::new();
    let mut vregs = Vregs::new();
    let overflow = format!("{symbol}_overflow");

    if platform.family() == PlatformFamily::Windows {
        // GetSystemTimePreciseAsFileTime(&ft): 100 ns intervals since 1601.
        instructions.push(abi::add_immediate(
            abi::c_arg(0),
            abi::stack_pointer(),
            WIN_FILETIME_OFFSET,
        ));
        platform.emit_external_call(
            "GetSystemTimePreciseAsFileTime",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // nanos = (FILETIME - epoch) * 100, raising `ErrOverflow` when it does not
        // fit an `Integer` (bug-640). A FILETIME is unsigned: one at or above 2^63
        // reads negative here, and its Unix count is then at least 2^63 - epoch
        // intervals, far past the range once scaled, so it overflows outright.
        // Below 2^63 the subtract cannot wrap, and the `* 100` is checked by
        // comparing the signed high word of the product with the low word's sign.
        let ft = vregs.next();
        let tmp = vregs.next();
        let high = vregs.next();
        let sign = vregs.next();
        instructions.extend([
            abi::load_u64(&ft, abi::stack_pointer(), WIN_FILETIME_OFFSET),
            abi::compare_immediate(&ft, "0"),
            abi::branch_lt(&overflow),
            abi::move_immediate(&tmp, "Integer", WIN_FILETIME_UNIX_EPOCH_100NS),
            abi::subtract_registers(&ft, &ft, &tmp), // 100 ns since Unix epoch
            abi::move_immediate(&tmp, "Integer", "100"),
            abi::signed_multiply_high_registers(&high, &ft, &tmp),
            abi::multiply_registers(&ft, &ft, &tmp),
            abi::arithmetic_shift_right_immediate(&sign, &ft, 63),
            abi::compare_registers(&high, &sign),
            abi::branch_ne(&overflow),
            abi::move_register(RESULT_VALUE_REGISTER, &ft),
        ]);
    } else {
        emit_libc_clock_nanos(
            CLOCK_REALTIME,
            &symbol,
            &overflow,
            platform,
            platform_imports,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        )?;
    }

    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());
    emit_clock_overflow_tail(&symbol, &overflow, &mut instructions, &mut relocations);
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = LOCALS_SIZE;
    Ok(void_int_result("datetime.nowNanos"))
}

const INTRO: &str = r#"The current wall-clock reading as nanoseconds since the Unix epoch."#;
const DESC: &str = r#"`datetime::nowNanos` is the raw form of `datetime::now`. It reads the host's
wall clock and returns a single `Integer` giving nanoseconds elapsed since
`1970-01-01T00:00:00Z` on the UTC timeline (the Unix epoch, without leap
seconds) — one count, rather than the `seconds`/`nanos` pair a `datetime::Instant`
carries.


Most programs should call `datetime::now`, which splits this same reading into a
structured `datetime::Instant` whose `seconds` and `nanos` fields can be projected through
a zone with `datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone`. Reach
for `nowNanos` directly only when a raw integer count of nanoseconds is what is
wanted — to stamp a log line, derive a millisecond count, or difference two
readings without building `datetime::Instant` values.

`nowNanos` reports nanoseconds since the epoch and is bounded by the range of an
`Integer`: a 64-bit signed nanosecond count covers
`1677-09-21T00:12:43.145224192Z` through `2262-04-11T23:47:16.854775807Z`. A
host clock reading outside that range raises `ErrOverflow` rather than returning
a wrapped count. This is a limit on `nowNanos`, not on the `datetime::Instant`
type, whose `seconds` field spans the full `Integer` range. On a correctly
configured host the reading is non-negative. `datetime::now` is built from this
same reading, so it shares the limit and raises the same error.

`nowNanos` is **not pure**: two calls may return different values, and a
program's output depends on the host clock. For reproducible logic, capture one
reading and derive everything else from it. It takes no arguments, reads host
clock state only, and has no side effects."#;
const EX: &str = r#"Read the current time as a raw nanosecond count:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
END SUB
```

Derive a millisecond timestamp from the nanosecond reading:

```
IMPORT datetime

SUB main()
  LET ns AS Integer = datetime::nowNanos()
  LET ms AS Integer = ns / 1000000
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "nowNanos",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("()"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![],
            return_type: super::ParameterType::Integer,
            errors: vec!["ErrOverflow"],
            body: super::Body::abi_function(lower_now_nanos),
        }],
    });
}
