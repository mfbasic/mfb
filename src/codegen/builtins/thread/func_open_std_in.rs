//! `thread::openStdIn` — subscribe a thread to standard input.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, opt, overload};

const INTRO: &str = r#"Let a thread read standard input."#;

const DESC: &str = r#"`openStdIn` subscribes a thread to standard input so it may read from it.

Standard input is a single stream, but every subscriber gets its own independent
view of it. A line read by one thread is never taken away from another: all of
them see the whole stream from the moment they subscribed. That is what makes it
safe for more than one thread to read at once.

Subscribing starts from where the stream is *now*. A thread that subscribes late
sees what arrives afterwards, never a replay of input that has already gone by.

There are two forms. With no argument it subscribes the thread that is calling —
that is what a worker calls on its own behalf. With a `Thread` handle it
subscribes the worker behind that handle, which is what a parent calls to grant
input to a thread it started.

The main thread is subscribed for you: an ordinary program that reads standard
input needs none of this. It is only worker threads that must ask, and a worker
that reads without subscribing raises `ErrInvalidContext`.

Subscribing twice is harmless. A worker that ends is unsubscribed for you, so
you only need `thread::closeStdIn` to stop reading earlier than that.

A subscriber that stops reading holds the stream up for the others, since input
is kept until everyone has seen it — so unsubscribe a thread that is done with
input rather than leaving it subscribed."#;

const EX: &str = r#"Grant a worker access to standard input, then take it away again:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET w AS Thread OF Nothing TO Integer = thread::start(workers::double, 4)
  thread::openStdIn(w)
  thread::closeStdIn(w)
  LET v AS Integer = thread::waitFor(w)
  io::print(toString(v))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // stdin broadcast: zero args (calling thread) OR one parent handle (that worker).
    pkg.add_function(function(
        "openStdIn",
        Some("() or Thread OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![opt(
                "t",
                &["thread"],
                any(false),
                "The thread to subscribe. Omit it to subscribe the thread making the call. The handle stays open.",
            )],
            ParameterType::Nothing,
            Body::abi_function(lowering::lower_open_std_in),
        )],
    ));
}
