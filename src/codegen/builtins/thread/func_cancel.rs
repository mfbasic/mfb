//! `thread::cancel` — ask a thread to stop.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, overload, req};

const INTRO: &str = r#"Ask a thread to stop at its next opportunity."#;

const DESC: &str = r#"`cancel` raises the cancellation flag on the thread behind `t` and returns
straight away.

**It is a request, not a kill.** Nothing is interrupted, no code is unwound, and
the thread keeps running normally until its own function chooses to stop. The
worker sees the request by calling `thread::isCancelled` and decides what to do
— usually return early. A worker that never calls `isCancelled` runs to
completion exactly as if you had never cancelled it, so a long loop with no
check in it cannot be stopped this way.

Because it is cooperative, cancellation is safe: a cancelled thread still closes
its own resources and still produces an outcome on the way out, whatever it
returns.

`cancel` does not wait and does not close the handle. A cancelled thread is
still collected with `thread::waitFor`, which is where you find out what it
returned once it noticed.

Cancelling twice is harmless — the flag is already up."#;

const EX: &str = r#"Ask a worker to stop, then collect what it did:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(workers::patient, "go")
  thread::cancel(t)
  LET outcome AS Integer = thread::waitFor(t)
  io::print("the worker returned " & toString(outcome))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "cancel",
        Some("Thread OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![req(
                "t",
                &["thread"],
                any(false),
                "The thread to ask to stop. The handle stays open — collect the thread with `thread::waitFor` afterwards.",
            )],
            ParameterType::Nothing,
            Body::abi_function_aliased(lowering::lower_cancel, &["drop"]),
        )],
    ));
}
