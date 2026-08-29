//! `thread` registry-migration tests: membership, arity, argument-typed return
//! resolution (the `ThreadHandle` two-overload model), the opaque handle types, and
//! the bespoke argument phrasings — the descriptor half the legacy `ThreadResolver`
//! used to own.

use crate::codegen::registry::{self, registry};

fn rt(call: &str, args: &[&str]) -> Option<String> {
    let types: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    registry::resolve_call(call, &types, false)
}

const ALL_CALLS: &[&str] = &[
    "thread.start",
    "thread.isRunning",
    "thread.waitFor",
    "thread.cancel",
    "thread.send",
    "thread.poll",
    "thread.receive",
    "thread.isCancelled",
    "thread.transfer",
    "thread.accept",
    "thread.openStdIn",
    "thread.closeStdIn",
];

#[test]
fn registered_on_the_clean_room_registry() {
    let pkg = registry()
        .resolve_package("thread")
        .expect("thread package");
    assert_eq!(pkg.functions().len(), 12);
}

#[test]
fn membership_via_generic_registry() {
    for n in ALL_CALLS {
        assert_eq!(registry().owning_package(n), Some("thread"), "{n}");
    }
    assert!(registry().owning_package("thread.nope").is_none());
    // The internal resource-plane / direction-split names are lowered-only: not members.
    for n in [
        "thread.emit",
        "thread.read",
        "thread.transferResource",
        "thread.acceptResource",
        "thread.emitResource",
        "thread.readResource",
        "thread.drop",
    ] {
        assert!(registry().owning_package(n).is_none(), "{n}");
    }
}

#[test]
fn opaque_handle_types_recognized() {
    assert!(registry().is_builtin_type("Thread"));
    assert!(registry().is_builtin_type("ThreadWorker"));
    // Parametric spellings recognized via the head-token extension.
    assert!(registry().is_builtin_type("Thread OF Integer TO String"));
    assert!(registry().is_builtin_type("ThreadWorker OF Integer RES fs.File TO String"));
    // Non-thread containers are NOT thread types.
    assert!(!registry().is_builtin_type("List OF Integer"));
    assert!(!registry().is_builtin_type("Integer"));
    // Qualified form resolves too.
    assert_eq!(
        registry().qualified_builtin_type("thread.Thread"),
        Some("Thread".to_string())
    );
}

#[test]
fn arity_ranges() {
    assert_eq!(registry().arity("thread.start"), Some((2, 4)));
    assert_eq!(registry().arity("thread.isRunning"), Some((1, 1)));
    assert_eq!(registry().arity("thread.send"), Some((2, 3)));
    assert_eq!(registry().arity("thread.poll"), Some((2, 2)));
    assert_eq!(registry().arity("thread.receive"), Some((1, 2)));
    assert_eq!(registry().arity("thread.transfer"), Some((2, 3)));
    assert_eq!(registry().arity("thread.accept"), Some((1, 2)));
    assert_eq!(registry().arity("thread.openStdIn"), Some((0, 1)));
}

#[test]
fn start_resolves_data_and_resource_workers() {
    let data = "ISOLATED FUNC(ThreadWorker OF Integer TO String, Integer) AS String";
    assert_eq!(
        rt("thread.start", &[data, "Integer"]),
        Some("Thread OF Integer TO String".into())
    );
    assert_eq!(
        rt("thread.start", &[data, "Integer", "Integer"]),
        Some("Thread OF Integer TO String".into())
    );
    assert_eq!(
        rt("thread.start", &[data, "Integer", "Integer", "Integer"]),
        Some("Thread OF Integer TO String".into())
    );
    // Resourced worker: the resource plane is echoed onto the parent handle.
    let resd = "ISOLATED FUNC(ThreadWorker OF Integer RES fs.File TO String, Integer) AS String";
    assert_eq!(
        rt("thread.start", &[resd, "Integer"]),
        Some("Thread OF Integer RES fs.File TO String".into())
    );
    // Wrong data-arg type / wrong limit / non-function / wrong arity.
    assert_eq!(rt("thread.start", &[data, "String"]), None);
    assert_eq!(rt("thread.start", &[data, "Integer", "String"]), None);
    assert_eq!(rt("thread.start", &["Integer", "Integer"]), None);
    assert_eq!(rt("thread.start", &[data]), None);
    // worker output != function output.
    let bad = "ISOLATED FUNC(ThreadWorker OF Integer TO String, Integer) AS Boolean";
    assert_eq!(rt("thread.start", &[bad, "Integer"]), None);
}

#[test]
fn parent_and_worker_queries() {
    let t = "Thread OF Integer TO String";
    let w = "ThreadWorker OF Integer TO String";
    assert_eq!(rt("thread.isRunning", &[t]), Some("Boolean".into()));
    assert_eq!(rt("thread.waitFor", &[t]), Some("String".into()));
    assert_eq!(rt("thread.cancel", &[t]), Some("Nothing".into()));
    assert_eq!(rt("thread.poll", &[t, "Integer"]), Some("Boolean".into()));
    assert_eq!(rt("thread.isCancelled", &[w]), Some("Boolean".into()));
    // Parent-only ops reject a worker; worker-only ops reject a parent.
    assert_eq!(rt("thread.isRunning", &[w]), None);
    assert_eq!(rt("thread.isCancelled", &[t]), None);
    assert_eq!(rt("thread.poll", &[t, "String"]), None);
    assert_eq!(rt("thread.isRunning", &[t, t]), None);
}

#[test]
fn send_receive_sleep_either_kind() {
    let t = "Thread OF Integer TO String";
    let w = "ThreadWorker OF Integer TO String";
    assert_eq!(rt("thread.send", &[t, "Integer"]), Some("Nothing".into()));
    assert_eq!(rt("thread.send", &[w, "Integer"]), Some("Nothing".into()));
    assert_eq!(
        rt("thread.send", &[t, "Integer", "Integer"]),
        Some("Nothing".into())
    );
    assert_eq!(rt("thread.send", &[t, "String"]), None);
    assert_eq!(rt("thread.send", &[t, "Integer", "String"]), None);
    // Unknown message accepts any arg (legacy resolve_send_unknown_message).
    let u = "Thread OF Unknown TO String";
    assert_eq!(rt("thread.send", &[u, "Integer"]), Some("Nothing".into()));

    assert_eq!(rt("thread.receive", &[t]), Some("Integer".into()));
    assert_eq!(rt("thread.receive", &[w]), Some("Integer".into()));
    assert_eq!(
        rt("thread.receive", &[t, "Integer"]),
        Some("Integer".into())
    );
    assert_eq!(rt("thread.receive", &[t, "String"]), None);

    // plan-99: `thread::sleep` is gone — the handle-free `os::sleep` replaced both
    // handle sides. It resolves as an unknown member on either side and at every
    // arity, which is what makes a stale `thread::sleep(t, ms)` a compile error.
    assert_eq!(rt("thread.sleep", &[t, "Integer"]), None);
    assert_eq!(rt("thread.sleep", &[w, "Integer"]), None);
    assert!(registry().owning_package("thread.sleep").is_none());
    assert!(registry().owning_package("thread.sleepWorker").is_none());
}

#[test]
fn transfer_accept_resource_plane() {
    let s = "Thread OF Integer RES fs.File TO String";
    assert_eq!(
        rt("thread.transfer", &[s, "fs.File"]),
        Some("Nothing".into())
    );
    assert_eq!(
        rt("thread.transfer", &[s, "fs.File", "Integer"]),
        Some("Nothing".into())
    );
    assert_eq!(rt("thread.transfer", &[s, "Socket"]), None);
    assert_eq!(rt("thread.accept", &[s]), Some("fs.File".into()));
    assert_eq!(rt("thread.accept", &[s, "Integer"]), Some("fs.File".into()));
    assert_eq!(rt("thread.accept", &[s, "String"]), None);
    // A data-only handle has no resource plane: rejected (STRICT-validated to None).
    let d = "Thread OF Integer TO String";
    assert_eq!(
        registry::resolve_call(
            "thread.accept",
            &["Thread OF Integer TO String".into()],
            true
        ),
        None
    );
    let _ = d;
    // A stateful plane element round-trips (plan-54): accept returns it verbatim.
    let stateful = "Thread OF Integer RES fs.File STATE Cursor TO String";
    assert_eq!(
        rt("thread.accept", &[stateful]),
        Some("fs.File STATE Cursor".into())
    );
    assert_eq!(
        rt("thread.transfer", &[stateful, "fs.File"]),
        Some("Nothing".into())
    );
}

#[test]
fn open_close_stdin() {
    assert_eq!(rt("thread.openStdIn", &[]), Some("Nothing".into()));
    assert_eq!(rt("thread.closeStdIn", &[]), Some("Nothing".into()));
    let t = "Thread OF Integer TO String";
    assert_eq!(rt("thread.openStdIn", &[t]), Some("Nothing".into()));
    assert_eq!(rt("thread.closeStdIn", &[t]), Some("Nothing".into()));
    // A worker handle is not a parent Thread: the one-arg form rejects it.
    let w = "ThreadWorker OF Integer TO String";
    assert_eq!(rt("thread.openStdIn", &[w]), None);
    assert_eq!(rt("thread.openStdIn", &[t, "Integer"]), None);
}

#[test]
fn expected_argument_phrasings() {
    assert!(registry::expected_arguments("thread.start")
        .unwrap()
        .starts_with("ISOLATED"));
    assert_eq!(
        registry::expected_arguments("thread.isRunning"),
        Some("Thread OF Msg TO Out")
    );
    assert_eq!(
        registry::expected_arguments("thread.isCancelled"),
        Some("ThreadWorker OF Msg TO Out")
    );
    assert!(registry::expected_arguments("thread.send")
        .unwrap()
        .contains(" or "));
    assert!(registry::expected_arguments("thread.openStdIn")
        .unwrap()
        .contains("Thread"));
}

#[test]
fn param_names_bind() {
    // The resource-plane mirrors and stdin wrappers carry parameter names (bug-221).
    assert_eq!(
        registry::call_param_names("thread.transfer").unwrap().len(),
        3
    );
    assert_eq!(
        registry::call_param_names("thread.accept").unwrap().len(),
        2
    );
    assert_eq!(
        registry::call_param_names("thread.openStdIn")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(registry::call_param_names("thread.send").unwrap().len(), 3);
}
