# bug-213: audio builtin classifier leaks internal names + optional-arg diagnostics understate valid signatures

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun / docs

Status: Open

Two diagnostic-surface defects in the builtins layer:

- `is_audio_call` (`src/builtins/audio.rs:74`, names at `:79-91`) includes the
  lowered-only internal names (`openInputDevice`, `openOutputDevice`,
  `readTimeout`, `pollTimeout`, `closeInput`, `closeOutput`), so the user-facing
  classifier treats them as real builtins — the bug-173-E pattern fixed for
  `tls`/`thread` but never applied to `audio`. Trigger: `audio::readTimeout()`
  yields `TYPE_CALL_ARGUMENT_MISMATCH ... expected supported overload` instead
  of an unknown-function diagnostic. Fix: narrow `is_audio_call` to user-facing
  names and add an `is_audio_runtime_call` for the codegen/IR-lowering sites.
- `expected_arguments` reports only the maximal-arity form for calls with an
  optional trailing arg (`src/builtins/audio.rs:302` audio READ;
  `src/builtins/fs.rs:253` OPEN_FILE), so `fs::openFile(5)` advertises
  `String, String` even though the 1-arg `String` form is valid. Fix: spell the
  optional argument, e.g. `"String[, String]"`, `"AudioInput, Integer[, Integer]"`.
