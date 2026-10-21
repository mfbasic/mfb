//! `thread.send`'s call-site boundary rules on hand-built IR (the package
//! path's shape):
//!
//! - a message type that belongs on the RES plane — a resource — is refused at
//!   the call with the plane rule, not the sendability one;
//! - a call whose own type is `Unknown` has no boundary to judge, so nothing
//!   is reported for it.

use super::*;

fn send_call(type_: &str) -> IrOp {
    IrOp::Eval {
        value: IrValue::Call {
            target: "thread.send".to_string(),
            args: vec![
                IrValue::Local("t".to_string()),
                IrValue::Local("f".to_string()),
            ],
            type_: ParameterType::parse(type_),
            loc: IrSourceLoc::default(),
        },
        loc: IrSourceLoc::default(),
    }
}

/// The details of the call-site message-type diagnostics for one `send`.
fn call_site_details(call_type: &str) -> Vec<String> {
    let f = func_returns(
        "run",
        "Nothing",
        vec![
            param("t", "Thread OF fs.File TO Integer", None),
            param("f", "fs.File", None),
        ],
        vec![send_call(call_type), ret_none()],
    );
    collect_diagnostics(&project(vec![f], vec![]))
        .into_iter()
        .filter(|d| d.detail.starts_with("Call to `thread.send` message type"))
        .map(|d| format!("{} {}", d.rule, d.detail))
        .collect()
}

#[test]
fn a_resource_message_type_is_refused_at_the_call_by_the_plane_rule() {
    let got = call_site_details("Nothing");
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(
        got[0].starts_with("TYPE_THREAD_RESOURCE_PLANE_REQUIRED "),
        "{got:?}"
    );
}

#[test]
fn a_send_whose_own_type_is_unknown_is_not_judged() {
    assert_eq!(call_site_details("Unknown"), Vec::<String>::new());
}
