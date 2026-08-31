//! `thread::start` — run an isolated function on its own thread.

use crate::codegen::registry::{Body, RegistryPackage};
use crate::types::ParameterType;

use super::{function, lowering, opt, overload, req, th};

const INTRO: &str = r#"Start an isolated function running on its own thread."#;

const DESC: &str = r#"`start` launches `f` on a new thread, hands it `data`, and returns a `Thread`
handle for talking to that thread and collecting its result. It does not wait:
the call returns while the function is still running.

`f` must be an `ISOLATED FUNC` reached through an import — an
`EXPORT ISOLATED FUNC` of a package you imported, or, inside a package project,
one of your own package's named as `self::worker`. A bare unqualified name is
rejected, and so are a `SUB`, a lambda, and a closure.

The entry point's own signature decides the handle's type. A function declared
`ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out` gives back a
`Thread OF Msg TO Out`, where `Msg` is what the two sides may send each other,
`Out` is what the function returns, and `In` is the type of `data`. A thread
that also carries open resources names them as well
(`ThreadWorker OF Msg RES Res TO Out`), giving a `Thread OF Msg RES Res TO Out`.

The new thread gets its own fresh copy of `f`'s package, including its own
top-level `MUT` state. Start the same function twice and you get two threads
that share none of it.

`data` is copied into the thread, so it must be thread-sendable, and afterwards
neither side can reach the other's copy.

`inboundLimit` and `outboundLimit` bound the two message queues, defaulting to
64 each. They are what makes a slow reader push back on a fast writer instead of
letting a queue grow without limit.

Collect the thread with `thread::waitFor`, which waits for it to finish, gives
you its result, and closes the handle."#;

const EX: &str = r#"Start a worker and wait for its answer:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF Nothing TO Integer = thread::start(workers::double, 21)
  LET answer AS Integer = thread::waitFor(t)
  io::print(toString(answer))
  RETURN 0
END FUNC
```

Bound the queues more tightly than the default 64:

```
IMPORT io
IMPORT thread
IMPORT workers

FUNC main AS Integer
  LET t AS Thread OF String TO Integer = thread::start(workers::chatter, "hello", 1, 1)
  LET greeting AS String = thread::receive(t, 1000)
  io::print(greeting)
  thread::send(t, "acknowledged")
  LET n AS Integer = thread::waitFor(t)
  io::print(toString(n))
  RETURN 0
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let out = || ParameterType::var("Out");
    let msg = || ParameterType::var("Msg");
    let res = || ParameterType::var("Res");
    let nothing = || ParameterType::Nothing;

    // start: the worker's msg/res/out are echoed onto the returned parent handle, and
    // any of msg/res can be `Nothing` (a resource-only or data-less worker). Since a
    // `Var` cannot bind `Nothing` under strict validation, each `Nothing` case is a
    // distinct overload — the msg × res `{Var, Nothing}` matrix (out is always the
    // worker's return, a real value). The all-`Var` overload is FIRST so lenient
    // return-inference binds every slot (a `Nothing` slot binds under lenient and
    // elides in `name()`); the `Nothing`-literal overloads exist for strict validation.
    let start_overload = |worker_msg: ParameterType, worker_res: ParameterType| {
        overload(
            vec![
                req(
                    "f",
                    &["entry"],
                    ParameterType::func_isolated(
                        vec![
                            th(true, worker_msg.clone(), worker_res.clone(), out()),
                            ParameterType::var("In"),
                        ],
                        out(),
                    ),
                    "The function to run. It must be an `EXPORT ISOLATED FUNC` of an imported package (or one of your own, named `self::…`), and a `FUNC` rather than a `SUB`, a lambda, or a closure.",
                ),
                req(
                    "data",
                    &[],
                    ParameterType::var("In"),
                    "The starting value, handed to the function as its second argument. It is copied into the thread, so the two sides never share it.",
                ),
                opt(
                    "inboundLimit",
                    &[],
                    ParameterType::Integer,
                    "How many messages may queue up toward the worker before a `thread::send` has to wait. Defaults to 64.",
                ),
                opt(
                    "outboundLimit",
                    &[],
                    ParameterType::Integer,
                    "How many messages may queue up back toward the parent before the worker's `thread::send` has to wait. Defaults to 64.",
                ),
            ],
            th(false, worker_msg, worker_res, out()),
            Body::abi_function(lowering::lower_start),
        )
    };

    pkg.add_function(function(
        "start",
        Some("ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, In, Integer, Integer"),
        (INTRO, DESC, EX),
        vec![
            start_overload(msg(), res()),
            start_overload(msg(), nothing()),
            start_overload(nothing(), res()),
            start_overload(nothing(), nothing()),
        ],
    ));
}
