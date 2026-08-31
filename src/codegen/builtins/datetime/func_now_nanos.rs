//! `datetime::nowNanos` — descriptor entry + authored docs, and the per-member
//! `abi_function` lowering ([`lower_now_nanos`]). The wrapper finalizes it
//! (crypto/io's clean-room shape).

use super::gen_shared::{
    emit_libc_clock_nanos, void_int_result, CLOCK_REALTIME, LOCALS_SIZE, WIN_FILETIME_OFFSET,
    WIN_FILETIME_UNIX_EPOCH_100NS,
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
/// Unix nanoseconds (plan-66-A). Always succeeds.
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
        let ft = vregs.next();
        let tmp = vregs.next();
        instructions.extend([
            abi::load_u64(&ft, abi::stack_pointer(), WIN_FILETIME_OFFSET),
            abi::move_immediate(&tmp, "Integer", WIN_FILETIME_UNIX_EPOCH_100NS),
            abi::subtract_registers(&ft, &ft, &tmp), // 100 ns since Unix epoch
            abi::move_immediate(&tmp, "Integer", "100"),
            abi::multiply_registers(RESULT_VALUE_REGISTER, &ft, &tmp),
        ]);
    } else {
        emit_libc_clock_nanos(
            CLOCK_REALTIME,
            &symbol,
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
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = LOCALS_SIZE;
    Ok(void_int_result("datetime.nowNanos"))
}

const INTRO: &str = r#"The current wall-clock reading as nanoseconds since the Unix epoch."#;
const DESC: &str = r#"`datetime::nowNanos` is the raw form of `datetime::now`. It reads the host's
wall clock and returns a single `Integer` giving nanoseconds elapsed since
`1970-01-01T00:00:00Z` on the UTC timeline (the Unix epoch, without leap
seconds) — one count, rather than the `seconds`/`nanos` pair an `Instant`
carries.


Most programs should call `datetime::now`, which splits this same reading into a
structured `Instant` whose `seconds` and `nanos` fields can be projected through
a zone with `datetime::toUtc`, `datetime::toLocal`, or `datetime::inZone`. Reach
for `nowNanos` directly only when a raw integer count of nanoseconds is what is
wanted — to stamp a log line, derive a millisecond count, or difference two
readings without building `Instant` values.

`nowNanos` reports nanoseconds since the epoch and is bounded by the range of an
`Integer`: a 64-bit signed nanosecond count overflows in the year 2262. This is
a limit on the intrinsic, not on the `Instant` type, whose `seconds` field spans
the full `Integer` range. On any correctly configured host the reading is
non-negative.

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
            errors: vec![],
            body: super::Body::abi_function(lower_now_nanos),
        }],
    });
}
