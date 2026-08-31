//! `thread::send` — put a value on the message channel.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{function, lowering, overload, send_params};

const INTRO: &str = r#"Send a value to the other end of a thread's message channel."#;

const DESC: &str = r#"`send` puts `data` on the thread's message channel for the other side to pick up
with `thread::receive`.

It works from both ends, and which end you are on is decided by the handle you
pass: from outside the thread pass your `Thread` handle and the value goes *to*
the worker; from inside the thread pass the `ThreadWorker` handle the entry
point was given and the value comes back *to* the parent. The two directions are
separate queues, so neither can ever read back its own message.

`data` must match the handle's message type and must be thread-sendable. It is
**copied** across, so the two sides never share it: changing your value
afterwards cannot affect the copy the other side received, and vice versa. A
resource cannot go this way at all — hand it over with `thread::transfer`
instead.

The queue has a size limit, fixed when the thread was started (64 by default).
If it is full, `send` waits up to `timeoutMs` milliseconds for room. That wait
is the point: it makes a fast producer slow down to its consumer's pace rather
than letting the queue grow without limit. If the queue is still full when the
time runs out, `send` raises `ErrTimeout`; a negative `timeoutMs` raises
`ErrInvalidArgument`.

`send` does not wait for the message to be *read* — only for room to put it. It
also does nothing to the handle, which stays open."#;

const EX: &str = r#"Send a reply back to a worker that is waiting for one:

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

The worker's side of the same conversation, sending through its own handle:

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
    // send constrains arg1 to the handle's message type (two kind-split overloads).
    pkg.add_function(function(
        "send",
        Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Msg, Integer"),
        (INTRO, DESC, EX),
        vec![
            overload(
                send_params(false, msg()),
                ParameterType::Nothing,
                Body::abi_function_aliased(lowering::lower_send, &["emit"]),
            ),
            overload(
                send_params(true, msg()),
                ParameterType::Nothing,
                Body::abi_function_aliased(lowering::lower_send, &["emit"]),
            ),
        ],
    ));
}
