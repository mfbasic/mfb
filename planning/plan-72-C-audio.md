# plan-72-C: audio package descriptor

Last updated: 2026-08-01
Effort: large (3h-1d)
Depends on: plan-72-A

Migrate `src/builtins/audio.rs` (738 LOC, 13 metadata helpers, 1
`package_source_glue!`, 2 builtin-type helpers, 2 custom-resolver helpers,
6 fixtures) to a `pub(crate) static AUDIO: BuiltinModule` with an
`AudioResolver` that preserves parameter-name overloads and typed
implementation-name selection.

References: plan-72 overview, `src/builtins/audio.rs`,
`bugs/completed-bugs/bug-380-*.md` (async handler UAF context — do not
regress),
`tests/{syntax,rt-behavior,byte-identity}/*/audio/`.

## Goal

- `audio::AUDIO` descriptor exists with `AudioResolver` covering
  `call_param_name_overloads` and `implementation_name(name, arg_types)`.
- Legacy free functions in `audio.rs` become wrappers that consult `AUDIO`.
- Parity tests cover every function, both builtin types, both source
  companion types (if any), and every arg-type driven implementation name
  currently emitted.

## Non-goals

- Do not change the async runtime handler wiring or thread-plane split.
- Do not remove the wrapper functions; `BB` owns deletion.
- Do not merge `audio` into any other letter.

## Current State

`src/builtins/audio.rs` has 13 descriptor-owned helpers, including
`call_param_name_overloads` at line 212 and `implementation_name(name,
arg_types)` at line 366. It defines two builtin types via `is_builtin_type` /
`builtin_type_fields` (line 112 / 127) and injects a companion via
`package_source_glue!`. Fixture load is 6 projects.

## Phases

### Phase C1 — descriptor and resolver

- [ ] Add `pub(crate) static AUDIO: BuiltinModule` with every function,
      overload (including parameter-name overloads), parameter (canonical +
      aliases), argument types, return type, implementation, and default.
- [ ] Add `BuiltinType` entries for the audio builtin types with record fields
      preserved.
- [ ] Model the `package_source_glue!` companion as
      `BuiltinSource { rule: InjectionRule::WhenImported, .. }`.
- [ ] Implement `AudioResolver` for typed `implementation_name` and parameter
      overload selection.
- [ ] Rewrite `is_audio_call`, `arity`, `resolve_call`, `expected_arguments`,
      `argument_types`, `call_param_names`, `call_param_name_overloads`,
      `call_return_type_name`, `implementation_name`, and both builtin-type
      helpers as wrappers over `AUDIO`/`AudioResolver`.
- [ ] Register `AUDIO` with the `BuiltinRegistry` from plan-72-A.
- [ ] Parity tests: assert descriptor + resolver answers equal legacy
      helpers for every `audio.*` name, every parameter-name overload, and
      every typed implementation-name case.

Acceptance: `cargo test` passes and every `audio.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`, including the
existing `tests/byte-identity/audio` cohort.
Commit: —

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `audio` fixtures per the overview.

## Corrections

Filled during execution.
