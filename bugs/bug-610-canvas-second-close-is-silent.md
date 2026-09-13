# bug-610: `canvas::destroyImage` / `canvas::destroyFont` silently accept a second close; the spec and every other close raise `ErrResourceClosed`

Last updated: 2026-09-13
Effort: small–medium
Severity: LOW
Class: Correctness (resource contract divergence)

Status: Open
Regression Test: none yet — see Phase 1

`mfb spec language resource-management` §15 states the one resource contract:
"a second close is a defined no-op reported as `ErrResourceClosed` rather than an
operation on a dead handle". Every other built-in explicit close conforms:
`mfb man tcp close` ("An already-closed handle is an error rather than a no-op"),
`mfb man fs close`, `mfb man audio close`, `mfb man udp close`, `mfb man tls close`.

`canvas::destroyImage` and `canvas::destroyFont` do not. Their lowering stores the
closed flag unconditionally, so a second call returns normally with no error
(`grep -n -i 'double-close\|unconditional store' src/codegen/builtins/canvas/func_destroy_image.rs src/codegen/builtins/canvas/func_destroy_font.rs`
prints the deliberate "Double-close must be a no-op" comment). Until plan-125-B
Phase 4 their pages called that "the same contract every resource has".

**The single correct behavior a fix produces:** an explicit second
`destroyImage`/`destroyFont` on a handle it already closed raises
`ErrResourceClosed`, exactly as `fs::close` does; the implicit close at scope end
after an explicit one stays silent (§15 "Re-closing an already-closed handle
during a drop … is a benign no-op"). **Or**, if the silent explicit re-close is
the intended design for canvas, §15 gains the exception, and the decision is
recorded here. Which one is the owner's call. Either way, spec and code must agree.

Found by plan-125-B Phase 4's cross-package consistency review
(`planning/plan-125-findings/B-phase4/man-consistency-handles.md` finding 1;
`man-consistency-guide-package.md` finding 1). **Filed, not fixed**, by user
instruction during a documentation-only plan ("file all bugs, make no fixes"). The
canvas pages now document the current behavior as an explicit exception.

## Reproduction

Needs an `--app` build, and `canvas::createImage` needs `app::Mode.Canvas`, which
opens a window. Per the desktop-disturbance rule, the runtime repro is run only
with the owner's go-ahead:

```basic
IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  RES img AS canvas::Image = canvas::createImage(1, 1, [0, 0, 0, 255])
  canvas::destroyImage(img)
  canvas::destroyImage(img) TRAP(e)
    io::print("second close raised " & toString(e.code))
    RECOVER
  END TRAP
  io::print("done")
END SUB
```

Expected per §15: `second close raised 77030004`. Observed per the lowering: no
handler output. (A compile-time use-after-close check may reject the direct
re-use; if so, route the handle through a helper `SUB` taking `RES img`.)

## Root cause

`src/codegen/builtins/canvas/func_destroy_image.rs` and `func_destroy_font.rs`
lower to an unconditional closed-flag store, skipping the closed guard that the
other close ops emit before closing.

## Non-goals

- Changing the drop-time (implicit) re-close, which §15 defines as silent.
- Changing `ErrResourceClosed` for any other canvas use of a closed handle.

## Blast-radius audit

- Every built-in explicit close op: `fs::close`, `tcp::close`, `udp::close`,
  `tls::close`, `audio::close`, `process` (no explicit close), `thread::waitFor`.
  All but canvas's two already raise; audit each in Phase 1 by lowering.

## Fix

Phase 1 — decide spec-vs-code with the owner; RED test for the chosen contract
(codegen-inspection test that the second explicit close branches to the
`ErrResourceClosed` raise, or a spec text test). Commit:

Phase 2 — conform (emit the closed guard, or amend §15 and the canvas pages).
Full suite. Commit:
