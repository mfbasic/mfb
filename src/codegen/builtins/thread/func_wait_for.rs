//! `thread::waitFor` — wait for a thread to finish and take its result.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{function, lowering, overload, req, th};

const INTRO: &str = r#"Wait for a thread to finish and take its result."#;

const DESC: &str = r#"`waitFor` blocks until the thread behind `t` finishes, then gives you what its
function returned. There is no timeout — it waits as long as the thread takes.

If the thread's function failed instead of returning, `waitFor` fails the same
way in your code, carrying that same error with its original code and message.
So a failure inside a thread reaches you exactly like a failure from any other
call: it routes to your `TRAP`, or propagates to your caller.

**`waitFor` also closes the handle.** It is how a thread is collected, and each
thread is collected once. Calling `waitFor` — or anything else — on the same
handle again raises `ErrResourceClosed`. Read the result into a variable if you
need it more than once.

`waitFor` does not cancel anything. On a thread you no longer want, call
`thread::cancel` first and then `waitFor` to collect it; cancelling alone does
not close the handle."#;

const EX: &str = r#"Collect a worker's return value:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::double, 21)
  LET answer AS Integer = thread::waitFor(t)
  io::print(toString(answer))
  RETURN 0
END FUNC
```

A failure inside the thread arrives as a failure here, with the worker's own
error code and message:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::failing, 3)
  LET v AS Integer = thread::waitFor(t)
  io::print("the worker succeeded with " & toString(v))
  RETURN 0
TRAP(err)
  io::print("the worker failed: " & err.message)
  RETURN 0
END TRAP
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // waitFor echoes the output. A single `Var`-output overload: a wholly-`Unknown`
    // (not-yet-inferred) handle leaves `Out` unbound so the return is `None`
    // (retryable) rather than a spurious concrete — a `Nothing`-return overload would
    // wildcard-match an `Unknown` handle and poison inference to `Nothing`.
    let out = || ParameterType::var("Out");
    pkg.add_function(function(
        "waitFor",
        Some("Thread OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![req(
                "t",
                &["thread"],
                th(
                    false,
                    ParameterType::Unknown,
                    ParameterType::Unknown,
                    out(),
                ),
                "The thread to wait for. This call closes the handle, so it cannot be used again afterwards.",
            )],
            out(),
            Body::abi_function(lowering::lower_wait_for),
        )],
    ));
}
