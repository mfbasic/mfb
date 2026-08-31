//! `thread::poll` — wait a bounded time for a message to be available.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, overload, req};

const INTRO: &str = r#"Wait up to a time limit for a message to be ready to read."#;

const DESC: &str = r#"`poll` waits up to `ms` milliseconds for a message to be waiting on the thread's
message channel, and answers `TRUE` if one is there and `FALSE` if the time ran
out first. It returns as soon as a message arrives, so a generous limit costs
nothing when the message comes quickly.

It only looks — it does not take the message. A `TRUE` means the next
`thread::receive` has something to return; you still have to call it.

`poll` is how you stay responsive without committing to a wait: check for work,
and if there is none, go and do something else rather than blocking in
`thread::receive`. With `ms` of `0` it is a bare "is anything there?" that
returns at once.

`poll` is about the **message** channel only. It says nothing about resources
crossing on the resource channel, and nothing about whether the thread has
finished — for that, use `thread::isRunning`.

A `FALSE` is not a failure. It means nothing had arrived yet, and the thread may
well send something a moment later."#;

const EX: &str = r#"Check for a message before committing to a wait:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(workers::chatter, "hello")
  IF thread::poll(t, 1000) THEN
    LET greeting AS String = thread::receive(t, 1000)
    io::print(greeting)
    thread::send(t, "acknowledged")
  ELSE
    io::print("nothing arrived within a second")
  END IF
  LET n AS Integer = thread::waitFor(t)
  io::print(toString(n))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "poll",
        Some("Thread OF Msg TO Out, Integer"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![
                req(
                    "t",
                    &["thread"],
                    any(false),
                    "The thread whose message channel to check. The handle stays open.",
                ),
                req(
                    "ms",
                    &[],
                    ParameterType::Integer,
                    "How long to wait, in milliseconds, for a message to appear. `0` checks and returns at once.",
                ),
            ],
            ParameterType::Boolean,
            Body::abi_function(lowering::lower_poll),
        )],
    ));
}
