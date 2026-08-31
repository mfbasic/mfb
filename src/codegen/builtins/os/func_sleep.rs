//! `os::sleep` — descriptor entry + authored docs, and the per-member
//! `Body::abi_function` lowering ([`lower_sleep`]).
//!
//! The body itself is [`lower_os_sleep_helper`], which lives beside the thread
//! runtime emitters it reuses (`codegen::runtime::thread`): a main-thread
//! `os::sleep` is the plain relative `nanosleep`/`Sleep` delay the parent
//! `thread::sleep` used, and a worker `os::sleep` is the cancellation-aware
//! condvar wait the worker `thread::sleep` used. Which one runs is decided at run
//! time from the TCB back-pointer the worker trampoline publishes at `arena+8`, so
//! the call needs no thread handle (plan-99).

use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::runtime::thread::lower_os_sleep_helper;
use crate::types::ParameterType;

/// `os::sleep(ms)` — block the calling thread for at least `ms` milliseconds.
pub(crate) fn lower_sleep(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        lower_os_sleep_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result("os.sleep"))
}

const INTRO: &str = r#"Block the calling thread for a number of milliseconds"#;
const DESC: &str = r#"`os::sleep(ms)` blocks the *calling* thread — whichever thread that is — for at
least `ms` milliseconds and returns `Nothing`. A negative `ms` raises
`ErrInvalidArgument`; `ms` of `0` returns immediately.

On the program's main thread the delay is plain and uninterruptible: it has no
wakeup path, and the sleep is re-entered if a signal cuts it short, so it never
returns early.

Inside a worker (`thread::start`) the same call is a **cancellation point**. It
wakes early and fails with `ErrInterrupted` when the parent requests cancellation
(`thread::cancel`, or dropping the parent handle), matching `thread::receive` and
`thread::send` on a worker handle. The deadline is absolute, so an inbound
`thread::send` arriving mid-sleep does not shorten it. `ErrInterrupted` is
declared on every `os::sleep` because a shared `FUNC` may be called from both the
main thread and a worker, but it can only ever be raised in a worker.

The unit is milliseconds, matching every `thread::` timeout. Sub-millisecond
delays are not expressible; use `datetime::monotonicNanos` to measure finer
intervals."#;
const EX: &str = r#"Pause the main thread for a tenth of a second:

```
IMPORT os
IMPORT io

SUB main()
  os::sleep(100)
  io::print("awake")
END SUB
```

Sleep inside a worker, treating cancellation as a normal shutdown. Cancelling a
sleeping thread makes its `os::sleep` raise, so the worker traps and returns:

```
IMPORT os
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO String = thread::start(workers::tick, 0)
  os::sleep(50)
  thread::cancel(t)
  io::print(thread::waitFor(t))
  RETURN 0
END FUNC
```

The worker's own side, in a companion package (a thread entry point must be an
exported `ISOLATED FUNC`):

```
EXPORT ISOLATED FUNC tick(w AS ThreadWorker OF Nothing TO String, seed AS Integer) AS String
  os::sleep(5000) TRAP(err)
    RETURN "cancelled"
  END TRAP
  RETURN "finished"
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "sleep",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "ms",
                desc: "milliseconds to block the calling thread",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrInvalidArgument", "ErrInterrupted"],
            body: Body::abi_function(lower_sleep),
        }],
    });
}
