# bug-116 — macOS TLS configure-block leaks the copied sec_protocol_options per connection

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** LOW — steady small leak per `tls::connect` on macOS.
**Class:** memory-safety (leak).

## Finding

`src/target/macos_aarch64/tls.rs:110-142` (`cfg_invoke_function`). The
configure block calls the captured `nw_tls_copy_sec_protocol_options` (a
copy-rule function returning +1) and, after
`sec_protocol_options_set_tls_server_name`, discards the object without
releasing it — one `sec_protocol_options` object leaks per `tls::connect`. The
TLS module resolves `nw_release` (tls/macos.rs) but this trampoline neither
captures nor calls a release for the object.

## Trigger

Long-running macOS client making many `tls::connect` calls → steady small leak
per connection.

## Fix

Call `nw_release` on the `sec_protocol_options` object at the end of the
configure block (after the server-name setter), matching the copy's +1
retain.
