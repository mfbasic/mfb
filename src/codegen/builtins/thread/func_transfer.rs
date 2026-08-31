//! `thread::transfer` — hand an open resource to the other side.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{function, lowering, overload, transfer_params};

const INTRO: &str = r#"Hand an open resource across to the other end of a thread."#;

const DESC: &str = r#"`transfer` hands the open handle `res` to the thread's other side, where
`thread::accept` picks it up. This is the resource channel, entirely separate
from the message channel, so a thread can be passing data and resources at the
same time.

**The call takes the handle.** Unlike everything else here, `res` is not copied:
there is one open file or socket, and after a successful transfer the sending
binding is closed and cannot be used again — `thread::accept` produces the same
open handle at the other end. If the transfer fails, the sending binding is
still open, so a `TRAP` handler can close it or try again.

It works from both ends, decided by the handle you pass: your `Thread` handle
sends the resource *to* the worker, the worker's own `ThreadWorker` handle sends
it back *to* the parent.

The thread's type has to declare the resource channel — `Thread OF Msg RES Res
TO Out`, or `Thread OF RES Res TO Out` for a thread that carries only resources.
A thread declared without one has nothing to transfer over.

Only some resource types may cross at all: `fs::File`, `tcp::Socket` and
`udp::Socket` may; listeners and `tls::Socket` may not. Resources are also never
allowed on the message channel, so this is the only way one moves between
threads.

Like the message queue, the resource queue is bounded, and `transfer` waits up
to `timeoutMs` milliseconds for room in it. If the queue is still full when that
time runs out, `transfer` raises `ErrTimeout` and the handle is still open. A
negative `timeoutMs` raises `ErrInvalidArgument`."#;

const EX: &str = r#"Open a file and hand it to a worker to write:

```
IMPORT io
IMPORT thread
IMPORT fs
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF RES fs::File TO Integer = thread::start(workers::fileWriter, 0)
  RES f AS fs::File = fs::open("/tmp/handover.txt", "write")
  thread::transfer(t, f, 2000)
  LET done AS Integer = thread::waitFor(t)
  io::print("worker returned " & toString(done))
  io::print(fs::readText("/tmp/handover.txt"))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    // Either-kind resource-plane members → two kind-split, resource-ONLY overloads.
    // The handle's msg/out are wildcards (a resource plane rides any data plane); only
    // `res` is captured, so a data-only handle (`res: Nothing`) is rejected by strict.
    pkg.add_function(function(
        "transfer",
        Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Res, Integer"),
        (INTRO, DESC, EX),
        vec![
            overload(
                transfer_params(false),
                ParameterType::Nothing,
                Body::abi_function_aliased(
                    lowering::lower_transfer,
                    &["transferResource", "emitResource"],
                ),
            ),
            overload(
                transfer_params(true),
                ParameterType::Nothing,
                Body::abi_function_aliased(
                    lowering::lower_transfer,
                    &["transferResource", "emitResource"],
                ),
            ),
        ],
    ));
}
