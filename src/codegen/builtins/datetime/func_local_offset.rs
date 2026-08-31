//! `datetime::localOffset` — descriptor entry + authored docs, and the per-member
//! `abi_function` lowering ([`lower_local_offset`]). The wrapper finalizes it
//! (crypto/io's clean-room shape). The `localtime_r` NULL / FILETIME-range failure
//! raises `ErrInvalidArgument` (bug-42), auto-propagated by the runtime-helper call
//! site.

use super::gen_shared::{
    void_int_result, LOCALS_SIZE, TIME_T_OFFSET, TM_GMTOFF_OFFSET, TM_OFFSET,
    WIN_FILETIME_MAX_UNIX_SEC, WIN_FILETIME_OFFSET, WIN_FILETIME_UNIX_EPOCH_100NS,
    WIN_HUNDRED_NS_PER_SEC, WIN_LOCAL_FILETIME_OFFSET, WIN_LOCAL_SYSTEMTIME_OFFSET,
    WIN_UNIX_EPOCH_TO_1601_SEC, WIN_UTC_SYSTEMTIME_OFFSET,
};
use crate::codegen::engine::builder::*;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::error::constants::*;
use crate::codegen::memory::data::*;
use crate::codegen::registry::AbiCtx;
use crate::target::shared::abi;

/// `abi_function` body for `datetime::localOffset(epochSeconds)` — the host's local
/// UTC offset in seconds at the given instant. On libc it calls
/// `localtime_r(&epochSeconds, &tm)` and reads `tm.tm_gmtoff`, guarding the NULL
/// return (bug-42). On Windows it converts through FILETIME/SYSTEMTIME and takes the
/// local−UTC FILETIME delta (plan-66-A). An out-of-range instant raises
/// `ErrInvalidArgument`.
pub(crate) fn lower_local_offset(
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
    // The out-of-range failure label for `localtime_r` returning NULL (bug-42); the
    // Windows path branches here on any conversion NULL as well.
    let range_fail = format!("{symbol}_range");

    if platform.family() == PlatformFamily::Windows {
        // epochSeconds (ARG[0]) → FILETIME → UTC SYSTEMTIME → local SYSTEMTIME
        // → local FILETIME. The (local − original) FILETIME delta is the UTC
        // offset in 100 ns units; a NULL from any conversion means the instant
        // is out of the FILETIME/SYSTEMTIME range → ErrInvalidArgument (bug-42).
        //
        // First bound `epochSeconds` so the `*1e7 + epoch` FILETIME arithmetic
        // cannot wrap: a wrapped product yields a valid-looking FILETIME that
        // FileTimeToSystemTime accepts, silently returning a garbage offset
        // instead of trapping (the libc `localtime_r` NULL path traps because
        // `tm_year`'s `int` overflows). The bounds are the exact FILETIME range:
        // below -11644473600 s the FILETIME is negative; above the HIGH bound
        // `epochSeconds*1e7 + epoch` exceeds i64. The residual year>30827 edge is
        // still caught by the FileTimeToSystemTime NULL check downstream.
        let epoch = vregs.next();
        let tmp = vregs.next();
        instructions.extend([
            abi::move_register(&epoch, abi::c_arg(0)), // epochSeconds
            // HIGH: epochSeconds > 910692730085 → epochSeconds*1e7+epoch > i64max.
            abi::move_immediate(&tmp, "Integer", WIN_FILETIME_MAX_UNIX_SEC),
            abi::compare_registers(&epoch, &tmp),
            abi::branch_gt(&range_fail),
            // LOW: epochSeconds + 11644473600 < 0 → FILETIME negative (pre-1601).
            abi::move_immediate(&tmp, "Integer", WIN_UNIX_EPOCH_TO_1601_SEC),
            abi::add_registers(&tmp, &epoch, &tmp),
            abi::compare_immediate(&tmp, "0"),
            abi::branch_lt(&range_fail),
            abi::move_immediate(&tmp, "Integer", WIN_HUNDRED_NS_PER_SEC),
            abi::multiply_registers(&epoch, &epoch, &tmp), // epochSeconds*1e7
            abi::move_immediate(&tmp, "Integer", WIN_FILETIME_UNIX_EPOCH_100NS),
            abi::add_registers(&epoch, &epoch, &tmp), // FILETIME
            abi::store_u64(&epoch, abi::stack_pointer(), WIN_FILETIME_OFFSET),
            abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), WIN_FILETIME_OFFSET),
            abi::add_immediate(
                abi::c_arg(1),
                abi::stack_pointer(),
                WIN_UTC_SYSTEMTIME_OFFSET,
            ),
        ]);
        platform.emit_external_call(
            "FileTimeToSystemTime",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::compare_immediate(abi::mfb_return(0), "0"));
        instructions.push(abi::branch_eq(&range_fail));
        // SystemTimeToTzSpecificLocalTime(NULL, &utc, &local): NULL selects the
        // machine's current time zone, applying its DST rules to the instant.
        instructions.extend([
            abi::move_immediate(abi::c_arg(0), "Integer", "0"),
            abi::add_immediate(
                abi::c_arg(1),
                abi::stack_pointer(),
                WIN_UTC_SYSTEMTIME_OFFSET,
            ),
            abi::add_immediate(
                abi::c_arg(2),
                abi::stack_pointer(),
                WIN_LOCAL_SYSTEMTIME_OFFSET,
            ),
        ]);
        platform.emit_external_call(
            "SystemTimeToTzSpecificLocalTime",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::compare_immediate(abi::mfb_return(0), "0"));
        instructions.push(abi::branch_eq(&range_fail));
        // SystemTimeToFileTime(&local, &localFt).
        instructions.extend([
            abi::add_immediate(
                abi::c_arg(0),
                abi::stack_pointer(),
                WIN_LOCAL_SYSTEMTIME_OFFSET,
            ),
            abi::add_immediate(
                abi::c_arg(1),
                abi::stack_pointer(),
                WIN_LOCAL_FILETIME_OFFSET,
            ),
        ]);
        platform.emit_external_call(
            "SystemTimeToFileTime",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.push(abi::compare_immediate(abi::mfb_return(0), "0"));
        instructions.push(abi::branch_eq(&range_fail));
        // offsetSeconds = (localFt − ft) / 1e7, signed (west-of-UTC is negative).
        instructions.extend([
            abi::load_u64(&epoch, abi::stack_pointer(), WIN_LOCAL_FILETIME_OFFSET),
            abi::load_u64(&tmp, abi::stack_pointer(), WIN_FILETIME_OFFSET),
            abi::subtract_registers(&epoch, &epoch, &tmp),
            abi::move_immediate(&tmp, "Integer", WIN_HUNDRED_NS_PER_SEC),
            abi::signed_divide_registers(RESULT_VALUE_REGISTER, &epoch, &tmp),
        ]);
    } else {
        // x0 holds epochSeconds. Stash it as the `time_t` input, then call
        // `localtime_r(&time_t, &tm)` and read `tm.tm_gmtoff`.
        instructions.extend([
            abi::store_u64(abi::c_arg(0), abi::stack_pointer(), TIME_T_OFFSET),
            abi::add_immediate(abi::c_arg(0), abi::stack_pointer(), TIME_T_OFFSET),
            abi::add_immediate(abi::c_arg(1), abi::stack_pointer(), TM_OFFSET),
        ]);
        platform.emit_external_call(
            "localtime_r",
            &symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        // `localtime_r` returns NULL (and sets EOVERFLOW) when the instant's
        // year does not fit `tm_year`'s `int`; on that path it writes no field
        // of `tm`, so loading `tm_gmtoff` would return an uninitialized stack
        // qword (an ASLR info-leak). Branch on the return before touching the
        // buffer (bug-42). plan-85: `localtime_r`'s pointer return is a C
        // result (`rax`, `%retC`), not the aligned MFB result register — read
        // it from the C-return register (byte-identical `x0` on ARM/RISC-V).
        instructions.push(abi::compare_immediate(abi::c_return(0), "0"));
        instructions.push(abi::branch_eq(&range_fail));
        instructions.push(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            TM_OFFSET + TM_GMTOFF_OFFSET,
        ));
    }

    instructions.push(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    instructions.push(abi::return_());

    // Out-of-range `epochSeconds`: report `ErrInvalidArgument` rather than
    // returning `tm.tm_gmtoff` from a buffer `localtime_r` never wrote. The
    // runtime-helper call site (`emit_runtime_helper_call`) already checks the
    // tag and auto-propagates the error up through `offsetAt`/`toLocal`, so no
    // package-source change is needed (bug-42). This tail sits after the shared
    // OK return so success never falls into it.
    instructions.push(abi::label(&range_fail));
    raise_error_into(
        &symbol,
        "ErrInvalidArgument",
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::return_());

    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = LOCALS_SIZE;
    Ok(void_int_result("datetime.localOffset"))
}

const INTRO: &str = r#"The host's local UTC offset in seconds at a given epoch second."#;
const DESC: &str = r#"`datetime::localOffset` returns the signed offset from UTC, in seconds, that the
host's configured local time zone applies at the absolute instant named by
`epochSeconds` — whole seconds since `1970-01-01T00:00:00Z` on the UTC timeline
(the Unix epoch, without leap seconds). A positive result places local civil
time ahead of UTC (east of the prime meridian); a negative result places it
behind UTC (west); zero means local time coincides with UTC at that instant.


This is the OS seam through which the rest of the package learns the host's
wall-clock rules. The call lowers to a libc runtime helper that hands
`epochSeconds` to `localtime_r` and reports the resolved `tm_gmtoff` for that
moment, so the result is DST-correct: it returns the standard-time offset for
instants outside daylight saving and the shifted offset for instants within it.
Two calls with epoch seconds on opposite sides of a daylight-saving transition
can therefore return different values. The offset reflects whatever zone the host
is configured to use (for example via the `TZ` environment variable or the
system zone setting), so the same program can produce different results on
different hosts.

Only the seconds value matters; there is no sub-second component. `localOffset`
is the low-level intrinsic that backs `datetime::offsetAt` for local zones and
`datetime::toLocal`; most code should prefer those higher-level functions, which
operate on `Instant` and `Zone` values rather than a raw epoch-seconds `Integer`.

`localOffset` is **not pure**: it reads the host's time-zone configuration, so
its result depends on host state. It has no side effects and reads no other
state."#;
const EX: &str = r#"The host's local offset for the current instant:

```
IMPORT datetime

SUB main()
  LET nowSeconds AS Integer = datetime::toMillis(datetime::now()) / 1000
  LET off AS Integer = datetime::localOffset(nowSeconds)
END SUB
```

Read the local offset at a fixed point on the timeline (the Unix epoch):

```
IMPORT datetime

SUB main()
  LET off AS Integer = datetime::localOffset(0)
END SUB
```"#;

pub(crate) fn register(pkg: &mut super::RegistryPackage) {
    pkg.add_function(super::RegistryFunction {
        name: "localOffset",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![super::Implementation {
            params: vec![super::Parameter {
                name: "epochSeconds",
                desc: "The instant, in seconds since the epoch, to ask about. The offset is not constant — a zone with daylight saving gives different answers at different times of year.",
                aliases: &[],
                ty: super::ParameterType::Integer,
                default: super::DefaultValue::None,
            }],
            return_type: super::ParameterType::Integer,
            errors: vec![],
            body: super::Body::abi_function(lower_local_offset),
        }],
    });
}
