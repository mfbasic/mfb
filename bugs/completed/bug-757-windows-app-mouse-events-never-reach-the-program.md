# bug-757: in a Windows `--app` build, no mouse event ever reaches `canvas::pollMouse` or `term::pollMouse`

Last updated: 2026-09-27
Effort: small
Severity: HIGH
Class: Correctness (a whole input device silently absent on one platform)

Status: FIXED — measured on box 206 (real Windows 11 desktop, injected input): canvas mouse on the readback and the swapchain path, and term mouse under `term::on`, all report the exact moves and clicks the driver made.
Regression Test: `src/target/win_x86_64/app/mod.rs` `canvas_reconcile_tests::the_mouse_arms_are_reachable_from_the_dispatch_chain` and `the_mouse_arms_use_only_distinct_x86_registers` (both fail against the pre-fix code); the manual box-206 run below.

A Windows `--app` program that calls `canvas::enableMouse(TRUE)` and polls
`canvas::pollMouse` in its draw loop — the swat-canvas shape, which works on macOS —
never sees an event. Keys arrive; the mouse is silent. `term::pollMouse` under
`term::on` is the same.

Found while verifying bug-688 V6 (whether the new swapchain child window blocks the
mouse). plan-94-E, which wrote the Windows arms, recorded that the path had "no
machine-driven smoke" and left runtime proof to a manual check that never happened;
its golden hashes the emitted code, which is present — just never run.

## Reproduction (box 206, `.ai/remote_systems.md`)

A canvas program presents a scene each frame, drains `canvas::pollMouse`, then drains
`io::readKey(0)`, printing every event. A PowerShell driver in the interactive
session (`schtasks /it`) moves the cursor to client (50,40) then (60,45) with
`SetCursorPos`, clicks with `mouse_event`, and types `q`, `z` with `SendKeys`. The
same injection reaches a WinForms window (control probe: `move 72,49`, `move 82,54`,
`down Left`).

| build | output |
|---|---|
| main `b0d24716f` (pre-fix) | `q`, `z` only — no mouse line |
| fixed | `Move 50,40`, `Move 60,45`, `Down Left 60,45`, `Up Left 60,45`, then `q`, `z` |

Term surface (fixed build, `term::on`, lines logged to a file because Console output
goes to the transcript): `Move row=2 column=6`, `Move row=2 column=7`,
`Down Left row=2 column=7`, `Up Left row=2 column=7` — 8×16 cells, as specified.

## Root cause — four faults, each hidden by the one before it

1. **The arms were unreachable.** `emit_wndproc` appended them after
   `wnd_reconcile_done`'s `return`, with no entry label, and the dispatch chain's last
   test (`WM_APP_RECONCILE`) sent every other message to `wnd_default`. No mouse
   message ever ran them. Fixed: they start at `WND_CHECK_MOUSE`, and that branch
   targets it when the program uses the mouse.
2. **The report writer held values in aliased registers.** On x86-64 only
   `SCRATCH[0..=9]` are distinct (`map_scratch_register` is `POOL[(n - 9) % 11]` over
   `x9`–`x18`, `x20`–`x28`, so `SCRATCH[10]` is rbx again, `SCRATCH[11]`/`[12]`
   rbx/rsi). The writer kept the terminator and the length in `SCRATCH[11]`/`[12]`
   — the digit loop's value and divisor. Once (1) was fixed the pipe carried garbage
   the key reader delivered as keys (`0`, Enter). Fixed: the formatter's seven work
   registers, the buffer, x and y fit `SCRATCH[0..=9]`; the cursor is `LOCAL[0]`
   (x19 → rbp); the button code goes straight into the formatter's `value` register;
   the terminator is patched over the last byte from its stack slot.
3. **`lParam` was parked in `LOCAL[1]`** — x20, rbx, the same register the x
   coordinate is then computed into — so y was always derived from x and came out 0
   (on the wire, 1). Fixed: `SCRATCH[2]`.
4. **The button code was built in `LOCAL[1]`** while `SCRATCH[0]` (the same rbx) held
   each modifier mask, so the code was replaced by the last mask test's result.
   Fixed: `LOCAL[0]`, which is also callee-saved across the `GetKeyState` call.

A throwaway compiler whose readers skipped mouse decoding showed the pipe itself
carried only `1`, `M` per event at step (2) — which is what located the
formatter rather than the decoder.

## Blast radius

Only the Windows app backend (`src/target/win_x86_64/app/{mod,mouse}.rs`). The shared
SGR formatter, the decoder and the other backends are unchanged; the macOS and GTK
arms emit into their own register models. Golden: the Windows `.app.ncodesum` of
every fixture with a mouse surface (`app-mouse-surface`).
