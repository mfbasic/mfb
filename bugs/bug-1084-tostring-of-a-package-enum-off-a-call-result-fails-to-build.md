# bug-1084: `toString` of a package enum read straight off a call result fails to build: "native code string literal has no data object"

Last updated: 2026-10-12
Effort: unknown
Severity: LOW
Class: Correctness

Status: Open
Regression Test: none yet

Found during `bug-staging-23` (2026-10-12) and placed in a later phase.

**The single correct behavior a fix produces:** the program below builds and prints
`renderer Software` (or the renderer in use), exactly as it does when the record is bound first.

## Reproduction (main `3745e626a`)

`bugs/repro/bug-1084/enum_off_call.mfb`, `mfb build --app <dir>`:

```
io::print("renderer " & toString(canvas::getFrameStats().renderer))
error: native code string literal 'Software' has no data object while lowering eval call io.print
```

Contrast, builds: `LET s AS canvas::FrameStats = canvas::getFrameStats()` and then
`toString(s.renderer)`.

## Not known

The cause; whether it needs a package enum, a field read off a call result, or the `&`
concatenation around it. Not narrowed.
