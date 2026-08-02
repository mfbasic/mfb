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

- [x] Add `pub(crate) static AUDIO: BuiltinModule` — 11 user-facing functions;
      `openInput`/`openOutput` carry two overloads (the `call_param_name_overloads`
      case); `read`/`poll` have an optional trailing `timeoutMs`; each function's
      return is fixed. The lowered-only internal names are NOT descriptor functions
      (see Corrections).
- [x] Add `BuiltinType` entries for all 5 audio types: `AudioInput`/`AudioOutput`
      (`Opaque`), `AudioDevice`/`AudioEnvelope`/`AudioNote` (`Record`, fields
      preserved).
- [x] Model the `package_source_glue!` companion (2-file concat) as
      `BuiltinSource { rule: InjectionRule::WhenImported, loader: source_file }`.
- [x] Implement `AudioResolver` for `resolve_return_type` (dual-direction /
      input-only / variadic-open validation). `implementation_name` stays static
      (`&'static` borrowed ABI) and parameter-overload selection is descriptor DATA
      (`param_name_overloads`), not a resolver hook (see Corrections).
- [x] Rewrite the metadata helpers: `is_audio_call`/`is_builtin_type`/
      `builtin_type_fields` delegate to the descriptor; `arity`/
      `call_return_type_name` delegate + explicit fallback for the internal names;
      `resolve_call` routes through `AudioResolver`. `call_param_names`/
      `call_param_name_overloads` stay static, PINNED to `AUDIO` by parity;
      `expected_arguments`/`argument_types`/`implementation_name` stay static;
      `is_audio_runtime_call`/`is_audio_internal_call`/`source_implementation_name`/
      `resource_close_function`/`consumes_argument` are audio routing (kept).
- [x] Register `AUDIO` with the `BuiltinRegistry`.
- [x] Parity test: 11 user-facing names + `audio.nope` (membership/arity/param
      names, per-overload tables for the opens, all 5 builtin types), 15 resolver
      samples covering both open overloads, input-only read, dual-direction
      poll/available/close, render, and both play forms; internal-name fallbacks.

Acceptance: `cargo test` passes and every `audio.*` fixture runs clean under
`scripts/test-accept.sh` (6 fixtures; byte-identity via the combined C/F/G/H/I
artifact-gate at finalization).
Commit: 0e22447f8

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `audio` fixtures per the overview.

## Corrections

- **The lowered-only internal names are not descriptor functions.**
  `audio.openInputDevice`/`openOutputDevice`/`readTimeout`/`pollTimeout`/
  `closeInput`/`closeOutput` are IR-lowering artifacts: they carry a return type
  (IR lowering rewrites a surface call to one, then queries `call_return_type_name`
  for the rewritten target) but no user-facing membership and (mostly) no arity.
  Modelling them as functions would break the `is_audio_call` exclusion and give
  them a derived arity that the legacy `arity` returns `None` for. So `AUDIO`
  holds only the 11 user-facing calls; `call_return_type_name` and `arity` delegate
  to the descriptor with an explicit fallback map for the internal names.
- **`AudioResolver` covers only `resolve_return_type`.** The plan named typed
  `implementation_name` and "parameter overload selection" as resolver work, but:
  (a) `implementation_name` returns `&'static` (borrowed) so it can't route through
  the owned resolver hook — it stays static and is verified by the existing
  `implementation_name_rewrites` test; (b) parameter-overload selection is
  descriptor DATA (`param_name_overloads`), derived by `DefaultResolver`, not a
  resolver hook.
- **Two `DefaultResolver` refinements (in `descriptor.rs`).** To reproduce
  `openInput`'s legacy metadata: `param_names` now returns `None` for a
  multi-overload function (its names live in `param_name_overloads`), and
  `param_name_overloads` returns `None` for a single-overload one — exactly the
  legacy `call_param_names`/`call_param_name_overloads` split. The A synthetic
  `s.pick` parity closures and the harness `builtin_type_fields` check (opaque
  types → empty fields ≡ legacy `None`) were updated to match; app/crypto parity
  `builtin_type_fields` set to `None` (their opaque/enum types have no fields
  helper; membership asserted directly).
- **`expected_arguments`/`argument_types` not descriptor-derivable** (custom
  `"AudioInput or AudioOutput[, Integer]"` / joined phrasing). Kept static.
