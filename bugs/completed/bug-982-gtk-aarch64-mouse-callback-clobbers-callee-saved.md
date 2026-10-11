# bug-982: Linux aarch64 GTK mouse callbacks clobber callee-saved `x19`–`x22`, so any mouse event SIGSEGVs an app that enabled the mouse

Last updated: 2026-10-07
Effort: small (<1h)
Severity: HIGH
Class: Memory-safety

Status: Fixed (2026-10-07, bs18-agent-i)
Regression Test: `tests/runtime/rt_foreign_entry_callee_saved.rs` (every OS-entered function, every app target); `target::linux_gtk::mouse::tests`

On linux-aarch64 `--app` builds, an app that calls `term::enableMouse(TRUE)` dies with
SIGSEGV (exit 139) on the first mouse event: a click, a motion, or a scroll. This includes
`Mode.Console` default programs. Without a mouse action, the same program exits 0.

**Correct behavior:** a GTK signal callback is called through the C ABI, so it must preserve
the AAPCS64 callee-saved registers (`x19`–`x28`). The mouse handler must save every
callee-saved register it writes and restore it before it returns. A mouse event is then
queued for `term::pollMouse` and the program keeps running.

## Root cause

`emit_mouse_handler` (`src/target/linux_gtk/mouse.rs`) builds the four GTK mouse callbacks
(press, release, motion, scroll). Its prologue saves only `lr` (plus the controller and the
d0/d1 coordinates). The body then writes `abi::LOCAL[0]` (`x19`) and `abi::LOCAL[1]` (`x20`)
in the gate, in `emit_translate_and_convert` and in `emit_modifiers`. The SGR formatting
writes `SCRATCH[10..12]`, which realize to `x20`–`x22`. None of them are restored. GLib's
closure marshaller holds a pointer in `x19` across the `blr` into our callback, and faults
dereferencing it after the return (`x19 = 0x1`, the `_mfb_rt_mouse_mode` value the handler
loaded).

`grep -n 'LOCAL\[0\]' src/target/linux_gtk/mouse.rs` shows the writes. The prologue at the
top of `emit_mouse_handler` stores only `link_register`, `c_arg(0)` and `FP_SCRATCH[0..1]`.

x86-64 may be unaffected, because callbacks there are bracketed by `wrap_x86_instructions`.
This is a guess: box 2228 was down, so x86-64 GTK was not run.

## Failing Reproduction

Measured by the coordinator on 2026-10-05, with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), on box 2226 (Debian 12 aarch64 glibc, GTK).

```
mfb init p962mc   # src/main.mfb:
IMPORT term
IMPORT io
IMPORT os
IMPORT fs
FUNC main() AS Nothing
  os::sleep(500)
  term::on()
  term::enableMouse(TRUE)
  io::print("mouse")
  term::sync()
  MUT i AS Integer = 0
  WHILE i < 40
    LET ev AS term::MouseEvent = term::pollMouse()
    IF ev.kind <> term::MouseKind.None THEN
      fs::appendText("/tmp/p962m.log", "event row=" & toString(ev.row) & " col=" & toString(ev.column) & "\n")
    END IF
    os::sleep(100)
    i = i + 1
  END WHILE
  term::off()
  fs::appendText("/tmp/p962m.log", "done\n")
END FUNC

mfb build -q --app --target linux-aarch64 p962mc
# ship build/p962mc-glibc.AppImage to 2226, then there:
Xvfb :93 -screen 0 1024x768x24 &
export DISPLAY=:93 GSK_RENDERER=cairo GDK_BACKEND=x11
timeout 20 ./p962mc-glibc.AppImage & sleep 1.2; xdotool click 1; wait $!; echo rc=$?
```

Observed, one run per action:

| action | rc |
|---|---|
| none | 0 |
| `xdotool click 1` | 139 |
| `xdotool mousemove 120 90; xdotool mousemove 300 200` | 139 |
| `xdotool click 4` (scroll) | 139 |

gdb on 2226 (from the agent that found it, bs16-c): the fault is in thread 1 (the GTK main
thread) at `ldr x0, [x19]`, right after `blr x26` (the GLib closure marshal's call into our
callback), with `x19 = 0x1` and `x20 = 0xaaaaaabff300` (the term area our handler loaded).

Expected: rc 0, with one `event` line logged per mouse action, and a final `done`.

## Related

- Found while proving bug-962 (`term::` in an `app::setMode` program has no drawing area).
- bug-985 (reproduced): the window built by
  `emit_reconcile_build_helper` never attaches the mouse controllers, so a `setMode` program
  gets no mouse events at all. It can only be reproduced once bug-962 has landed, and it must
  be fixed after this bug, because attaching the controllers first would turn "no events"
  into this crash.

## Resolution

**Fix.** `emit_mouse_handler` (`src/target/linux_gtk/mouse.rs`) parks `LOCAL[0..4]` in a
new frame area (`OFF_CALLEE_SAVED`, 32 bytes; the report buffer moved up by 32) in the
prologue and reloads them at `mouse_done`, the one exit every path reaches. On AArch64
that is `x19`–`x22` (`SCRATCH[10..12]`, used by the SGR formatter, are `x20`–`x22` too);
on x86-64 `LOCAL[0]` is the physical `rbp` that `finalize_x86_app_function`'s bracket
does not save, and the other three are vregs inside the bracket.

**Second defect on the same path, found while proving this one.** The cells conversion
loaded the integer cell metrics (`ST_TERM_CELL_W/H`, stored from Pango's
`logical.width/height`) with `fmov` (a bit move), so a 9 px cell divided as the denormal
4.4e-323, every quotient was infinite and the range check dropped every click and
motion on the term surface. It now converts with `scvtf`. Only the wheel (which skips
the conversion) ever reported before. Pinned by
`linux_gtk::mouse::tests::cell_metrics_are_converted_not_reinterpreted`.

**The audit the coordinator asked for**: `tests/runtime/rt_foreign_entry_callee_saved.rs`
checks every address-taken function and every `_mfb_{macapp,gtkapp,winapp}_*` body in the
`-ncode` of an app canvas+term mouse program and a console mouse program (macOS, both
Linux arches, Windows), the thread fixture (all five targets) and the TLS fixture (three
Linux targets), with a value-tracking dataflow and interprocedural escape sets. Calls into
compiled MFB code are held to the convention: any callee-saved register outside the
allocator's preserved set may change. RED at f6b4099c9 for exactly these sites, all fixed:

| Site | Target(s) | Register(s) | Fix |
| --- | --- | --- | --- |
| `_mfb_gtkapp_mouse_{pressed,released,motion,scroll}` | linux-aarch64 / linux-x86_64 | `x19`–`x22` / `rbp` | above |
| macOS mouse IMPs, `_mfb_macapp_canvas_blit` | macos-aarch64 | `x19`, `x22`/`x23` | bug-993 |
| `_mfb_rt_canvas_graphics_entry` (graphics thread start routine) | linux-aarch64, linux-x86_64, windows-x86_64 (macOS by convention) | `x20`/`x28`; `rbx`/`r13`; + `rdi`/`rsi` | `graphics_trampoline_saved` lists every non-allocator callee-saved register (arena, thread, closure, `mfb_arg(7)`, Win64 `mfb_arg(4/5)`); frame 56 SysV / 104 Win64 / 48 AArch64 |
| `_mfb_rt_thread_trampoline` | linux-x86_64, windows-x86_64 | `rbp`; `rbp`/`rdi`/`rsi` | x86 saves `mfb_arg(7)` (+ Win64 `mfb_arg(4/5)`) from offset 72; SysV frame stays 88, Win64 grows to 104. AArch64/riscv64 bytes unchanged |
| `_mfb_tls_verify_cb` (OpenSSL callback) | linux-x86_64 | `rbx` (`SCRATCH[0]`) | uses `c_arg(2)`, caller-saved everywhere |

Verdicts for the rest (the test audits them and they pass): every other `_mfb_gtkapp_*`
callback (key, draw, resize, idle, window-sync, fullscreen-notify, canvas
draw/resize/commit, display rate) and the macOS key/term/metal/window IMPs; the Win32
`_mfb_winapp_wndproc` and `_mfb_winapp_key_inject`; `_main`, `_mfb_gtkapp_main`,
`_mfb_macapp_program`, the finish helpers (no reachable `ret`). The Win64 `xmm6`/`xmm7`
part of the same audit is bug-994.

**Spec.** `mfb spec app term-backend` (callback ABI paragraph + the integer cell
metrics), `mfb spec threading os-integration` (trampoline `rbp`, Win64 `rdi`/`rsi`).
`.ai/arch-abi.md` § "Every OS-entered callback is a C-ABI callee".

### Proof

RED test: `cargo test --release --test rt_foreign_entry_callee_saved` at 2383ab9c5 (test
only) failed all 6 tests naming the sites above; after the fix, 6 passed.

Runtime, box 2226 (Debian 12 aarch64 glibc GTK, Xvfb `:93` + xdotool, `GDK_BACKEND=x11
GSK_RENDERER=cairo`), the doc's repro built `--app --target linux-aarch64`, plus a variant
that calls `io::readKey(0)` once per loop (see Note):

| build | none | click | move | scroll |
| --- | --- | --- | --- | --- |
| main f6b4099c9 | rc 0 | rc 139 | rc 139 | rc 139 |
| fix, doc repro | rc 0 | rc 0 | rc 0 | rc 0 |
| fix, readKey variant | rc 0, 0 events | rc 0, 2 events (`row=6 col=23` press+release) | rc 0, 2 events | rc 0, 3 events (motion + 2 wheel) |

linux-x86_64 GTK was not run at runtime: 2228 is down, and 2227 has no Xvfb/xdotool (a
user-space Xvfb extracted there could not compile its keymap). The x86 handler change is
the one `rbp` save/restore, checked by the audit test.

**Note — `pollMouse` alone never sees an event.** The decoder runs on the stdin read path
(`mfb spec app term-backend`, "The mouse decoder and the read path"), and `pollMouse`
only drains the ring, so a program that polls the mouse without ever reading stdin (this
doc's repro, `/tmp/cdscroll` in bug-993) decodes nothing: the fix makes it survive, but it
logs no events. bug-758's repro reads `io::readKey(0)` for the same reason. Reported to the
coordinator as a separate question (a contract/man-page decision, not part of this fix).

## Golden impact

`scripts/artifact-gate.sh target/release/mfb all`: 16 diffs, each localized by a
function-level `-ncode` diff against main:

- `syntax/app/app-mouse-surface` `.{macos-aarch64,linux-aarch64,linux-x86_64,windows-x86_64}.app.ncodesum`:
  the mouse IMPs/handlers, `_mfb_macapp_canvas_blit`, `_mfb_rt_canvas_graphics_entry`.
- `byte-identity/thread` `.linux-x86_64`/`.windows-x86_64.ncodesum`: `_mfb_rt_thread_trampoline` only.
- `byte-identity/tls`, `byte-identity/http` `.linux-{aarch64,x86_64,riscv64}.ncodesum`: `_mfb_tls_verify_cb` only.
- `byte-identity/resource-xfer-slots` `.linux-{aarch64,x86_64,riscv64}` (verify callback) and
  `.windows-x86_64` (trampoline) `.ncodesum`.
