//! `datetime::localOffset` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). The OS-seam body is the shared
//! `abi_function` lowering [`super::gen_os_seam::lower_datetime_os_seam`]; the
//! wrapper finalizes it (crypto/io's clean-room shape). The `localtime_r` NULL /
//! FILETIME-range failure raises `ErrInvalidArgument` through the tag the shared
//! body sets (bug-42), auto-propagated by the runtime-helper call site.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;

/// `abi_function` body for `datetime::localOffset` — the shared OS-seam lowering,
/// selected by call name.
pub(crate) fn lower_local_offset(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    super::gen_os_seam::lower_datetime_os_seam(builder, ctx, "datetime.localOffset")
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
                desc: "",
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
