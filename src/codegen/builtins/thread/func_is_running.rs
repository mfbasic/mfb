//! `thread::isRunning` — whether a thread is still going.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, overload, req};

const INTRO: &str = r#"Report whether a thread is still running."#;

const DESC: &str = r#"`isRunning` answers `TRUE` while the thread behind `t` is still going and
`FALSE` once its function has finished. It returns immediately and never waits.

The answer is a snapshot of a moment that has already passed: a thread reported
as running may finish before the next line, and one reported as finished stays
finished. So `isRunning` is useful for deciding whether to do something else
meanwhile, and not for deciding it is safe to skip `thread::waitFor` — every
thread still has to be collected.

`isRunning` does not close the handle and does not retrieve the result; only
`thread::waitFor` does either. After `waitFor` has closed the handle, calling
`isRunning` on it raises `ErrResourceClosed`.

A cancelled thread is still running until it actually stops — `thread::cancel`
asks, and the worker decides when to notice."#;

const EX: &str = r#"Do other work while a thread runs, then collect it:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::double, 5)
  IF thread::isRunning(t) THEN
    io::print("still working")
  END IF
  LET answer AS Integer = thread::waitFor(t)
  io::print(toString(answer))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "isRunning",
        Some("Thread OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![req(
                "t",
                &["thread"],
                any(false),
                "The thread to ask about. The handle stays open — this only reads its state.",
            )],
            ParameterType::Boolean,
            Body::abi_function(lowering::lower_is_running),
        )],
    ));
}
