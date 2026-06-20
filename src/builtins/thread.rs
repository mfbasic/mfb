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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_thread_call(name: &str) -> bool {
    matches!(
        name,
        START | IS_RUNNING | WAIT_FOR | CANCEL | SEND | POLL | RECEIVE | IS_CANCELLED
    )
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
    let (_, message, output) = thread_parts(&worker)?;
    Some(format!("Thread OF {message} TO {output}"))
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

/// Output type for `thread::waitFor`, which is only valid on a parent `Thread`
/// handle (not a `ThreadWorker`).
pub(crate) fn parent_thread_output(name: &str) -> Option<&str> {
    thread_parts(name).and_then(|(kind, _, output)| (kind == THREAD_TYPE).then_some(output))
}

pub(crate) fn thread_parts(name: &str) -> Option<(&str, &str, &str)> {
    if let Some(rest) = name.strip_prefix("Thread OF ") {
        let (message, output) = split_thread_types(rest)?;
        return Some((
            THREAD_TYPE,
            strip_type_group(message),
            strip_type_group(output),
        ));
    }
    let rest = name.strip_prefix("ThreadWorker OF ")?;
    let (message, output) = split_thread_types(rest)?;
    Some((
        THREAD_WORKER_TYPE,
        strip_type_group(message),
        strip_type_group(output),
    ))
}

fn split_thread_types(rest: &str) -> Option<(&str, &str)> {
    let message_end = type_prefix_len(rest.trim())?;
    let rest = rest.trim();
    let separator = rest.get(message_end..)?;
    let output = separator.strip_prefix(" TO ")?;
    Some((rest[..message_end].trim(), output.trim()))
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

    if matches!(base, "Map" | "MapEntry" | "Thread" | "ThreadWorker") {
        let first_len = type_prefix_len(after_of)?;
        let after_first = after_of.get(first_len..)?;
        let second_input = after_first.strip_prefix(" TO ")?;
        let second_len = type_prefix_len(second_input)?;
        return Some(base_end + 4 + first_len + 4 + second_len);
    }

    Some(base_end)
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
