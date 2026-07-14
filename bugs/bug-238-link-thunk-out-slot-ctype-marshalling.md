# bug-238: LINK OUT-slot result read skips ctype sign-extension/finiteness applied to direct returns

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: correctness

Status: Open

An `OUT`-slot result in `lower_link_thunk`
(`src/target/shared/code/link_thunk.rs:547-552`) is always read back with a bare
`load_u64` from the zero-initialized 8-byte buffer, with no ctype-based
sign-extension or width/finiteness handling — unlike the direct-return path,
which sign-extends `CInt32` (`:774-780`) and NaN/Inf-checks `CDouble`.

Trigger: a `LINK FUNC` that surfaces its result through `AS OUT ... return` of
ctype `CInt32` writing a negative value (e.g. -1) yields MFBASIC Integer
`4294967295` (zero-extended) instead of `-1`; a `CDouble` OUT bypasses the
finiteness rejection applied to a direct `CDouble` return.

Fix: apply the same ctype-driven marshalling (sign-extend `CInt32`,
finiteness-check `CDouble`, mask `CByte`, normalize `CBool`) to the OUT-result
read, or reject those ctypes for OUT in the frontend.
