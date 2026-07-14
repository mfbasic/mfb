# bug-221: thread::transfer/accept expose no parameter-name table → named arguments silently fail to bind

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun

Status: Open

`thread::transfer`/`thread::accept` expose no parameter-name table, so named
arguments silently fail to bind even though their mirror calls `send`/`receive`
support them and the man pages document `t`/`res`/`timeoutMs` as nameable.

Trigger: `thread::transfer(t, res, timeoutMs := 100)` or
`thread::accept(t, timeoutMs := 100)` — `call_param_names`
(`src/builtins/thread.rs:75-88`) returns `None` and thread declares no
`call_param_name_overloads`, so `timeoutMs` cannot resolve, while the
structurally identical `thread::send(t, v, timeoutMs := 100)` works.

Fix: add `TRANSFER`/`ACCEPT` (and `OPEN_STD_IN`/`CLOSE_STD_IN`) rows to
`thread::call_param_names`, e.g. `TRANSFER => Some(&[&["t","thread"],
&["res","resource"], &["timeoutMs"]])`, `ACCEPT => Some(&[&["t","thread"],
&["timeoutMs"]])`.
