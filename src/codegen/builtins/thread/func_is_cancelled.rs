//! `thread::isCancelled` — the worker's side of cancellation.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, overload, req};

const INTRO: &str = r#"Report whether this thread has been asked to stop."#;

const DESC: &str = r#"`isCancelled` is the worker's half of cancellation. Call it from inside a thread,
passing the `ThreadWorker` handle the entry point was given, and it answers
`TRUE` once someone has called `thread::cancel` on that thread.

This is the only thing that makes cancellation work. `thread::cancel` only
raises a flag; nothing stops until the worker asks about it and acts. A worker
that never calls `isCancelled` cannot be cancelled at all, so any long-running
loop should check it and return early when it goes `TRUE`.

It returns immediately, never waits, and never raises. Once the answer is
`TRUE` it stays `TRUE`.

What to do on cancellation is entirely yours: return a partial result, return a
sentinel, or fail. Whatever the function does, the parent sees it through
`thread::waitFor`. Resources the worker holds are closed on the way out as
usual."#;

const EX: &str = r#"A worker that checks for cancellation inside its loop:

```
EXPORT ISOLATED FUNC patient(worker AS ThreadWorker OF String TO Integer, unused AS String) AS Integer
  MUT spins AS Integer = 0
  WHILE spins < 2000
    IF thread::isCancelled(worker) THEN
      RETURN -1
    END IF
    spins = spins + 1
  END WHILE
  RETURN spins
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "isCancelled",
        Some("ThreadWorker OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![req(
                "t",
                &["thread"],
                any(true),
                "This thread's own `ThreadWorker` handle — the first argument the entry point was given.",
            )],
            ParameterType::Boolean,
            Body::abi_function(lowering::lower_is_cancelled),
        )],
    ));
}
