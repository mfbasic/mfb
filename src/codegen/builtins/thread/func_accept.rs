//! `thread::accept` — take an open resource handed over by the other side.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{accept_params, function, lowering, overload};

const INTRO: &str = r#"Take an open resource handed over by the other end of a thread."#;

const DESC: &str = r#"`accept` takes the next open handle off the thread's resource channel — the one
the other side put there with `thread::transfer` — waiting up to `timeoutMs`
milliseconds for one to arrive.

What you get back is the same open file or socket, not a copy, and from here on
it is yours: you use it and you close it, or let it close when your scope ends.
The side that transferred it can no longer touch it.

It works from both ends, decided by the handle you pass: the worker's own
`ThreadWorker` handle takes what the parent transferred in, and your `Thread`
handle takes what the worker transferred back.

The thread's type has to declare the resource channel — `Thread OF Msg RES Res
TO Out`, or `Thread OF RES Res TO Out` for a thread that carries only resources
— and what you accept comes back as that declared `Res` type. A thread declared
without a resource channel has nothing to accept.

If the resource carries `STATE`, the channel names that too
(`Thread OF RES fs::File STATE Cursor TO Out`), because both sides have to agree
on it in advance.

`accept` waits for a resource specifically. `thread::poll` reports on the
message channel and says nothing about this one."#;

const EX: &str = r#"A worker that is handed an open file and writes through it:

```
EXPORT ISOLATED FUNC fileWriter(worker AS ThreadWorker OF RES fs::File TO Integer, unused AS Integer) AS Integer
  RES f AS fs::File = thread::accept(worker, 2000)
  fs::writeAll(f, "written by the worker\n")
  RETURN 1
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let res = || ParameterType::var("Res");
    pkg.add_function(function(
        "accept",
        Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Integer"),
        (INTRO, DESC, EX),
        vec![
            overload(
                accept_params(false),
                res(),
                Body::abi_function_aliased(
                    lowering::lower_accept,
                    &["acceptResource", "readResource"],
                ),
            ),
            overload(
                accept_params(true),
                res(),
                Body::abi_function_aliased(
                    lowering::lower_accept,
                    &["acceptResource", "readResource"],
                ),
            ),
        ],
    ));
}
