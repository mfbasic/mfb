use std::borrow::Cow;

pub(crate) const THREAD_TYPE: &str = "Thread";
pub(crate) const THREAD_WORKER_TYPE: &str = "ThreadWorker";

const START: &str = "thread.start";
const IS_RUNNING: &str = "thread.isRunning";
const WAIT_FOR: &str = "thread.waitFor";
const CANCEL: &str = "thread.cancel";
const SEND: &str = "thread.send";
const POLL: &str = "thread.poll";
const RECEIVE: &str = "thread.receive";
const IS_CANCELLED: &str = "thread.isCancelled";
/// Resource plane: move a resource across a thread boundary. `transfer` mirrors
/// `send` and `accept` mirrors `receive`, but they carry a resource message
/// rather than data, keeping the data channel resource-free.
pub(crate) const TRANSFER: &str = "thread.transfer";
pub(crate) const ACCEPT: &str = "thread.accept";
/// Internal lowered targets for the resource plane. `thread::transfer`/`accept`
/// lower to these so codegen routes them to the dedicated resource queues (they
/// never appear in source). The plane is split by direction like the data plane:
/// `transfer` on a parent handle is `transferResource` (inbound queue) and on a
/// worker handle is `emitResource` (outbound queue); `accept` on a worker handle
/// is `acceptResource` (inbound queue) and on a parent handle is `readResource`
/// (outbound queue). The worker-direction split is applied in `builder_values`.
pub(crate) const TRANSFER_RESOURCE: &str = "thread.transferResource";
pub(crate) const ACCEPT_RESOURCE: &str = "thread.acceptResource";
pub(crate) const EMIT_RESOURCE: &str = "thread.emitResource";
pub(crate) const READ_RESOURCE: &str = "thread.readResource";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_thread_call(name: &str) -> bool {
    matches!(
        name,
        START | IS_RUNNING
            | WAIT_FOR
            | CANCEL
            | SEND
            | POLL
            | RECEIVE
            | IS_CANCELLED
            | TRANSFER
            | ACCEPT
            | TRANSFER_RESOURCE
            | ACCEPT_RESOURCE
            | EMIT_RESOURCE
            | READ_RESOURCE
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        START => Some(&[
            &["f", "entry"],
            &["data"],
            &["inboundLimit"],
            &["outboundLimit"],
        ]),
        IS_RUNNING | WAIT_FOR | CANCEL | IS_CANCELLED => Some(&[&["t", "thread"]]),
        SEND => Some(&[&["t", "thread"], &["data", "value"], &["timeoutMs"]]),
        POLL => Some(&[&["t", "thread"], &["ms"]]),
        RECEIVE => Some(&[&["t", "thread"], &["timeoutMs"]]),
        _ => None,
    }
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == THREAD_TYPE
        || name == THREAD_WORKER_TYPE
        || name.starts_with("Thread OF ")
        || name.starts_with("ThreadWorker OF ")
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        START if matches_start(arg_types) => Cow::Owned(start_thread_type(&arg_types[0])?),
        IS_RUNNING if arg_types.len() == 1 && is_parent_thread_type(&arg_types[0]) => {
            Cow::Borrowed("Boolean")
        }
        WAIT_FOR if arg_types.len() == 1 && is_parent_thread_type(&arg_types[0]) => {
            thread_output(&arg_types[0]).map(Cow::Borrowed)?
        }
        CANCEL if arg_types.len() == 1 && is_parent_thread_type(&arg_types[0]) => {
            Cow::Borrowed("Nothing")
        }
        SEND if (arg_types.len() == 2 || arg_types.len() == 3)
            && is_thread_type(&arg_types[0])
            && thread_message(&arg_types[0])
                .is_some_and(|message| message == "Unknown" || message == arg_types[1])
            && arg_types.get(2).is_none_or(|timeout| timeout == "Integer") =>
        {
            Cow::Borrowed("Nothing")
        }
        POLL if arg_types.len() == 2
            && is_parent_thread_type(&arg_types[0])
            && arg_types[1] == "Integer" =>
        {
            Cow::Borrowed("Boolean")
        }
        RECEIVE
            if (arg_types.len() == 1 || arg_types.len() == 2)
                && is_thread_type(&arg_types[0])
                && arg_types.get(1).is_none_or(|timeout| timeout == "Integer") =>
        {
            thread_message(&arg_types[0]).map(Cow::Borrowed)?
        }
        TRANSFER
            if (arg_types.len() == 2 || arg_types.len() == 3)
                && is_thread_type(&arg_types[0])
                && thread_resource(&arg_types[0])
                    .is_some_and(|resource| resource == "Unknown" || resource == arg_types[1])
                && arg_types.get(2).is_none_or(|timeout| timeout == "Integer") =>
        {
            Cow::Borrowed("Nothing")
        }
        ACCEPT
            if (arg_types.len() == 1 || arg_types.len() == 2)
                && is_thread_type(&arg_types[0])
                && arg_types.get(1).is_none_or(|timeout| timeout == "Integer") =>
        {
            thread_resource(&arg_types[0]).map(Cow::Borrowed)?
        }
        IS_CANCELLED if arg_types.len() == 1 && is_worker_thread_type(&arg_types[0]) => {
            Cow::Borrowed("Boolean")
        }
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        START => Some("ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out, In, Integer, Integer"),
        IS_RUNNING | WAIT_FOR | CANCEL => Some("Thread OF Msg TO Out"),
        SEND => Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Msg, Integer"),
        POLL => Some("Thread OF Msg TO Out, Integer"),
        RECEIVE => Some("Thread OF Msg TO Out or ThreadWorker OF Msg TO Out, Integer"),
        IS_CANCELLED => Some("ThreadWorker OF Msg TO Out"),
        TRANSFER => {
            Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Res, Integer")
        }
        ACCEPT => {
            Some("Thread OF Msg RES Res TO Out or ThreadWorker OF Msg RES Res TO Out, Integer")
        }
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        START => Some((2, 4)),
        IS_RUNNING | WAIT_FOR | CANCEL | IS_CANCELLED => Some((1, 1)),
        SEND => Some((2, 3)),
        POLL => Some((2, 2)),
        RECEIVE => Some((1, 2)),
        TRANSFER => Some((2, 3)),
        ACCEPT => Some((1, 2)),
        _ => None,
    }
}

fn matches_start(arg_types: &[String]) -> bool {
    if !(2..=4).contains(&arg_types.len()) {
        return false;
    }
    let Some(params) = function_params(&arg_types[0]) else {
        return false;
    };
    params.len() == 2
        && is_worker_thread_type(&params[0])
        && thread_output(&params[0]).is_some_and(|output| {
            function_output(&arg_types[0]).is_some_and(|function_output| output == function_output)
        })
        && params[1] == arg_types[1]
        && arg_types.get(2).is_none_or(|limit| limit == "Integer")
        && arg_types.get(3).is_none_or(|limit| limit == "Integer")
}

fn start_thread_type(name: &str) -> Option<String> {
    let worker = function_params(name)?.first()?.clone();
    let (_, message, resource, output) = thread_parts_full(&worker)?;
    Some(format_thread_type(THREAD_TYPE, message, resource, output))
}

/// Render a thread type string from its parts, emitting the optional `RES Res`
/// clause and the resource-only spelling (`message == "Nothing"`) symmetrically
/// with `split_thread_types`.
pub(crate) fn format_thread_type(
    kind: &str,
    message: &str,
    resource: Option<&str>,
    output: &str,
) -> String {
    match resource {
        Some(resource) if message == "Nothing" => {
            format!("{kind} OF RES {resource} TO {output}")
        }
        Some(resource) => format!("{kind} OF {message} RES {resource} TO {output}"),
        None => format!("{kind} OF {message} TO {output}"),
    }
}

fn function_params(name: &str) -> Option<Vec<String>> {
    let rest = name.strip_prefix("ISOLATED FUNC(")?;
    let (params, _) = rest.split_once(") AS ")?;
    Some(split_top_level_types(params))
}

fn function_output(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("ISOLATED FUNC(")?;
    rest.split_once(") AS ").map(|(_, output)| output)
}

pub(crate) fn is_thread_type(name: &str) -> bool {
    thread_parts(name).is_some()
}

pub(crate) fn is_parent_thread_type(name: &str) -> bool {
    thread_parts(name).is_some_and(|(kind, _, _)| kind == THREAD_TYPE)
}

pub(crate) fn is_worker_thread_type(name: &str) -> bool {
    thread_parts(name).is_some_and(|(kind, _, _)| kind == THREAD_WORKER_TYPE)
}

pub(crate) fn thread_message(name: &str) -> Option<&str> {
    thread_parts(name).map(|(_, message, _)| message)
}

pub(crate) fn thread_output(name: &str) -> Option<&str> {
    thread_parts(name).map(|(_, _, output)| output)
}

/// The resource type carried on the thread's resource plane
/// (`thread::transfer`/`thread::accept`), or `None` for a data-only thread. A
/// data-only thread is spelled `Thread OF Msg TO Out`; the resource plane is the
/// optional `RES Res` clause: `Thread OF Msg RES Res TO Out` (or `Thread OF RES
/// Res TO Out` when there is no data channel).
pub(crate) fn thread_resource(name: &str) -> Option<&str> {
    thread_parts_full(name).and_then(|(_, _, resource, _)| resource)
}

/// Output type for `thread::waitFor`, which is only valid on a parent `Thread`
/// handle (not a `ThreadWorker`).
pub(crate) fn parent_thread_output(name: &str) -> Option<&str> {
    thread_parts(name).and_then(|(kind, _, output)| (kind == THREAD_TYPE).then_some(output))
}

pub(crate) fn thread_parts(name: &str) -> Option<(&str, &str, &str)> {
    thread_parts_full(name).map(|(kind, message, _, output)| (kind, message, output))
}

/// Full structural view of a thread type: `(kind, message, resource, output)`.
/// `message` is the data-plane message type (`"Nothing"` for a resource-only
/// thread); `resource` is the resource-plane type, present only when the type
/// carries a `RES Res` clause.
pub(crate) fn thread_parts_full(name: &str) -> Option<(&'static str, &str, Option<&str>, &str)> {
    let (kind, rest) = if let Some(rest) = name.strip_prefix("Thread OF ") {
        (THREAD_TYPE, rest)
    } else if let Some(rest) = name.strip_prefix("ThreadWorker OF ") {
        (THREAD_WORKER_TYPE, rest)
    } else {
        return None;
    };
    let (message, resource, output) = split_thread_types(rest)?;
    Some((
        kind,
        strip_type_group(message),
        resource.map(strip_type_group),
        strip_type_group(output),
    ))
}

/// Parse the body after `Thread OF ` / `ThreadWorker OF ` into
/// `(message, resource, output)`. Accepts three shapes:
///   `<msg> TO <out>`              (data-only)
///   `<msg> RES <res> TO <out>`    (data + resource planes)
///   `RES <res> TO <out>`          (resource-only; message defaults to Nothing)
fn split_thread_types(rest: &str) -> Option<(&str, Option<&str>, &str)> {
    let rest = rest.trim();

    // Resource-only: no data message before the `RES` clause.
    if let Some(after_res) = rest.strip_prefix("RES ") {
        let res_end = type_prefix_len(after_res)?;
        let resource = after_res[..res_end].trim();
        let output = after_res.get(res_end..)?.strip_prefix(" TO ")?.trim();
        return Some(("Nothing", Some(resource), output));
    }

    let message_end = type_prefix_len(rest)?;
    let message = rest[..message_end].trim();
    let tail = rest.get(message_end..)?;

    // Optional ` RES <res>` clause between the message and ` TO `.
    if let Some(after_res) = tail.strip_prefix(" RES ") {
        let res_end = type_prefix_len(after_res)?;
        let resource = after_res[..res_end].trim();
        let output = after_res.get(res_end..)?.strip_prefix(" TO ")?.trim();
        return Some((message, Some(resource), output));
    }

    let output = tail.strip_prefix(" TO ")?.trim();
    Some((message, None, output))
}

pub(crate) fn strip_type_group(type_: &str) -> &str {
    let trimmed = type_.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        return trimmed;
    }
    let mut depth = 0usize;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + ch.len_utf8() != trimmed.len() {
                    return trimmed;
                }
            }
            _ => {}
        }
    }
    &trimmed[1..trimmed.len() - 1]
}

fn type_prefix_len(input: &str) -> Option<usize> {
    let input = input.trim_start();
    if input.starts_with('(') {
        let mut depth = 0usize;
        for (index, ch) in input.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(index + ch.len_utf8());
                    }
                }
                _ => {}
            }
        }
        return None;
    }

    let base_end = input
        .char_indices()
        .find_map(|(index, ch)| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' || ch == '.' {
                None
            } else {
                Some(index)
            }
        })
        .unwrap_or(input.len());
    if base_end == 0 {
        return None;
    }
    let base = &input[..base_end];
    let Some(after_of) = input[base_end..].strip_prefix(" OF ") else {
        return Some(base_end);
    };

    if matches!(base, "List" | "Result") {
        return type_prefix_len(after_of).map(|len| base_end + 4 + len);
    }

    if matches!(base, "Map" | "MapEntry") {
        let first_len = type_prefix_len(after_of)?;
        let after_first = after_of.get(first_len..)?;
        let second_input = after_first.strip_prefix(" TO ")?;
        let second_len = type_prefix_len(second_input)?;
        return Some(base_end + 4 + first_len + 4 + second_len);
    }

    if matches!(base, "Thread" | "ThreadWorker") {
        // `[msg] [RES res] TO out` — mirror split_thread_types' three shapes.
        return thread_body_len(after_of).map(|len| base_end + 4 + len);
    }

    Some(base_end)
}

/// Length consumed by a thread type body (`[msg] [RES res] TO out`) starting at
/// `rest`. Used by `type_prefix_len` to measure a nested thread type.
fn thread_body_len(rest: &str) -> Option<usize> {
    if let Some(after_res) = rest.strip_prefix("RES ") {
        let res_len = type_prefix_len(after_res)?;
        let to = after_res.get(res_len..)?.strip_prefix(" TO ")?;
        let out_len = type_prefix_len(to)?;
        // "RES " (4) + res + " TO " (4) + out
        return Some(4 + res_len + 4 + out_len);
    }

    let msg_len = type_prefix_len(rest)?;
    let tail = rest.get(msg_len..)?;

    if let Some(after_res) = tail.strip_prefix(" RES ") {
        let res_len = type_prefix_len(after_res)?;
        let to = after_res.get(res_len..)?.strip_prefix(" TO ")?;
        let out_len = type_prefix_len(to)?;
        // msg + " RES " (5) + res + " TO " (4) + out
        return Some(msg_len + 5 + res_len + 4 + out_len);
    }

    let to = tail.strip_prefix(" TO ")?;
    let out_len = type_prefix_len(to)?;
    // msg + " TO " (4) + out
    Some(msg_len + 4 + out_len)
}

fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(params[start..index].trim().to_string());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(params[start..].trim().to_string());
    parts
}
