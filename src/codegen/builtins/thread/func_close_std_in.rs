//! `thread::closeStdIn` — unsubscribe a thread from standard input.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{any, function, lowering, opt, overload};

const INTRO: &str = r#"Stop a thread reading standard input."#;

const DESC: &str = r#"`closeStdIn` undoes `thread::openStdIn`: the thread stops being a reader of
standard input and gives up its place in the stream.

It does not close standard input itself, and it does not affect any other
thread. The others keep reading exactly as before.

There are two forms, matching `openStdIn`. With no argument it unsubscribes the
thread that is calling; with a `Thread` handle it unsubscribes the worker behind
that handle.

You rarely have to call it. A worker that ends is unsubscribed automatically, so
this is for stopping *earlier* than the end of the thread. The reason to bother
is that input is kept until every subscriber has seen it, so a thread that has
stopped reading but is still subscribed holds the stream up for the rest — cut
it loose as soon as it is done.

Unsubscribing a thread that is not subscribed is harmless. A thread that reads
standard input after unsubscribing raises `ErrInvalidContext`; subscribe again
with `thread::openStdIn` first, and note that it will rejoin at the current
point in the stream, not where it left off."#;

const EX: &str = r#"Let a worker read input, then cut it loose before it ends:

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
```

The no-argument form acts on the thread that calls it:

```
IMPORT io
IMPORT thread

FUNC main AS Integer
  thread::openStdIn()
  thread::closeStdIn()
  io::print("done with standard input")
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(function(
        "closeStdIn",
        Some("() or Thread OF Msg TO Out"),
        (INTRO, DESC, EX),
        vec![overload(
            vec![opt(
                "t",
                &["thread"],
                any(false),
                "The thread to unsubscribe. Omit it to unsubscribe the thread making the call. The handle stays open.",
            )],
            ParameterType::Nothing,
            Body::abi_function(lowering::lower_close_std_in),
        )],
    ));
}
