# bug-213: audio builtin classifier leaks internal names + optional-arg diagnostics understate valid signatures

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: footgun / docs

Status: Fixed (2026-07-15) — (1) `is_audio_call` narrowed to the user-facing surface and a new `is_audio_runtime_call` (user-facing + the lowered-only internal names) now serves the codegen/plan/runtime sites (4 plan.rs + runtime/mod.rs), mirroring the tls/thread bug-173-E split. `audio::call_return_type_name` had to be narrowed too: `builtins::is_builtin_call` falls back to it, so leaving the internal names there re-admitted `audio::readTimeout()` as a user-callable builtin (it silently compiled to a SIGSEGV). (2) `expected_arguments` now spells the optional trailing argument: audio READ -> `AudioInput, Integer[, Integer]`, audio POLL -> `AudioInput or AudioOutput[, Integer]`, fs OPEN_FILE/NO_FOLLOW -> `String[, String]`.
Regression Test: `audio::readTimeout()` / `closeInput()` / `openInputDevice()` now report SYMBOL_UNKNOWN_IDENTIFIER (was a call-argument mismatch, then briefly a silent miscompile); unit test `is_call_accepts_only_the_user_facing_surface` pins the split and the return-type-fallback hole. HW-verified on Ubuntu x86_64 (VM 2228): audio::devices() and audio::openOutput() (incl. scope-drop close, which routes through the internal closeOutput name) still work.

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
