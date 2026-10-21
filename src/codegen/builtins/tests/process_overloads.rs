//! Overloaded `process::` members pick their runtime helper by argument shape
//! (`engine/value/builder_values/inline_calls.rs:lower_runtime_helper_call`): the
//! extra `timeoutMs` / `from` / environment arguments select a distinct helper
//! (`sendTimeout`, `sendBytesTimeout`, `pollFrom`, `receiveFrom`,
//! `receiveBytesFrom`, `spawnEnv`), because the base helper has no slot for them.
//! Calling the base helper with the extra argument dropped would silently ignore a
//! timeout or read the wrong stream.

use super::lowered::calls_on;
use crate::testutil::CodeTarget;

const HEADER: &str = "IMPORT process\nIMPORT io\n\n";

fn calls_of(body: &str) -> Vec<String> {
    let source = format!("{HEADER}FUNC main() AS Integer\n{body}\n  RETURN 0\nEND FUNC\n");
    calls_on(&source, "main", CodeTarget::LinuxX86_64)
}

fn helper(call: &str) -> String {
    format!("_mfb_rt_process_process_{call}")
}

fn has_call(calls: &[String], call: &str) -> bool {
    calls.iter().any(|c| *c == helper(call))
}

/// Each optional argument routes to its own helper, and its absence to the base
/// one.
#[test]
fn the_optional_arguments_select_distinct_helpers() {
    let with = calls_of(
        "  RES p = process::spawn([\"cat\"], \".\", Map OF String TO String { \"K\" := \"V\" }, FALSE)\n  \
         process::send(p, \"x\", 100)\n  \
         process::sendBytes(p, [toByte(65)], 100)\n  \
         LET ready AS Boolean = process::poll(p, 10, process::Stream.StdErr)\n  \
         LET line AS String = process::receive(p, process::Stream.StdErr)\n  \
         LET chunk AS List OF Byte = process::receiveBytes(p, process::Stream.StdErr)\n  \
         io::print(line & toString(ready) & toString(len(chunk)))",
    );
    for call in [
        "spawnEnv",
        "sendTimeout",
        "sendBytesTimeout",
        "pollFrom",
        "receiveFrom",
        "receiveBytesFrom",
    ] {
        assert!(has_call(&with, call), "{call} in {with:?}");
    }
    let without = calls_of(
        "  RES p = process::spawn([\"cat\"])\n  \
         process::send(p, \"x\")\n  \
         process::sendBytes(p, [toByte(65)])\n  \
         LET ready AS Boolean = process::poll(p, 10)\n  \
         LET line AS String = process::receive(p)\n  \
         LET chunk AS List OF Byte = process::receiveBytes(p)\n  \
         io::print(line & toString(ready) & toString(len(chunk)))",
    );
    for call in [
        "spawn",
        "send",
        "sendBytes",
        "poll",
        "receive",
        "receiveBytes",
    ] {
        assert!(has_call(&without, call), "{call} in {without:?}");
    }
    for call in [
        "spawnEnv",
        "sendTimeout",
        "sendBytesTimeout",
        "pollFrom",
        "receiveFrom",
        "receiveBytesFrom",
    ] {
        assert!(!has_call(&without, call), "{call} in {without:?}");
    }
}
