//! `thread::receive` — take a value off the message channel.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{function, lowering, overload, receive_params};

const INTRO: &str = r#"Take the next value from a thread's message channel."#;

const DESC: &str = r#"`receive` takes the next message off the thread's message channel and returns
it, waiting up to `timeoutMs` milliseconds if none has arrived yet.

Like `thread::send` it works from both ends, and the handle decides which:
from outside the thread pass your `Thread` handle to read what the worker sent
*out*; from inside pass the `ThreadWorker` handle the entry point was given to
read what the parent sent *in*. The two directions are separate queues, so you
never read back your own message.

Messages come out in the order they were sent, and each is delivered once — two
threads reading the same channel do not both get a copy.

The value arrives as its own copy, so nothing is shared with the sender.

If no message arrives before `timeoutMs` runs out, `receive` **raises**
`ErrTimeout` — it does not return an empty or default value, so a timeout is
something to trap rather than test for. A negative `timeoutMs` raises
`ErrInvalidArgument`.

To look before committing to a wait, call `thread::poll` first: it reports
whether a message is ready without taking it, and answers `FALSE` instead of
raising when there is nothing there.

`receive` does not close the handle; the thread is still collected with
`thread::waitFor`."#;

const EX: &str = r#"Read what a worker sent, then reply:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(workers::chatter, "hello")
  LET greeting AS String = thread::receive(t, 1000)
  io::print(greeting)
  thread::send(t, "acknowledged")
  LET echoed AS String = thread::receive(t, 1000)
  io::print(echoed)
  LET n AS Integer = thread::waitFor(t)
  io::print(toString(n))
  RETURN 0
END FUNC
```

Inside the worker, reading what the parent sent through its own handle:

```
EXPORT ISOLATED FUNC chatter(worker AS ThreadWorker OF String TO Integer, greeting AS String) AS Integer
  thread::send(worker, greeting & " from the worker")
  LET reply AS String = thread::receive(worker, 1000)
  thread::send(worker, "worker heard: " & reply)
  RETURN len(reply)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let msg = || ParameterType::var("Msg");
    // receive echoes the message (two kind-split overloads). Like waitFor, no
    // `Nothing`-return overload — that would wildcard-match an `Unknown` handle.
    pkg.add_function(function(
        "receive",
        Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Integer"),
        (INTRO, DESC, EX),
        vec![
            overload(
                receive_params(false, msg()),
                msg(),
                Body::abi_function_aliased(lowering::lower_receive, &["read"]),
            ),
            overload(
                receive_params(true, msg()),
                msg(),
                Body::abi_function_aliased(lowering::lower_receive, &["read"]),
            ),
        ],
    ));
}
