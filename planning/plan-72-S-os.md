# plan-72-S: os package descriptor

Last updated: 2026-08-01
Effort: medium (1h-2h)
Depends on: plan-72-A

Migrate `src/builtins/os.rs` (274 LOC, 6 metadata helpers, 0 source glue,
0 builtin types, 0 custom-resolver helpers, 35 fixtures) to a
`pub(crate) static OS: BuiltinModule`.

References: plan-72 overview, `src/builtins/os.rs`,
`tests/{syntax,rt-behavior,byte-identity}/*/os/`.

## Goal

- `os::OS` descriptor exists and mirrors every current metadata helper.
- Legacy free functions in `os.rs` become wrappers over `OS`.
- Parity tests cover every function name, arity, return type, and argument
  type.

## Non-goals

- Do not add builtin types or a source companion (os has neither today).
- Do not remove the wrapper functions; `BB` owns deletion.

## Current State

`src/builtins/os.rs` is data-shaped with 6 descriptor-owned helpers and no
source companion, builtin type, or custom resolver. Fixture load is 35
projects.

## Phases

### Phase S1 — descriptor and wrappers

- [x] Add `pub(crate) static OS: BuiltinModule` with every function,
      overload, parameter, argument types, return type, implementation
      (`Same` — runtime helper, no rewrite), and default (none).
- [x] Rewrite the 6 metadata helpers as wrappers over `OS`
      (`is_os_call`/`arity`/`call_return_type_name`/`resolve_call` delegate to
      `DefaultResolver`; `call_param_names` borrowed static pinned by parity;
      `expected_arguments` stays hand-authored for the niladic `"no arguments"`
      phrasing — opted out of parity, like io).
- [x] Register `OS` with the `BuiltinRegistry` from plan-72-A.
- [x] Parity tests for every `os.*` name and unknown-name behavior.

Acceptance: `cargo test` passes; every `os.*` fixture runs clean under
`scripts/test-accept.sh target/debug/mfb target/accept-actual`.
Commit: <S-hash>

## Validation

- `cargo test`
- `scripts/test-accept.sh target/debug/mfb target/accept-actual`
- Byte-identity/artifact gate for `os` fixtures per the overview.

## Corrections

- **`expected_arguments` stays hand-authored (opted out of parity).** os's nine
  niladic calls (`environ`/`args`/`pid`/…) render their expected arguments as
  the bespoke `"no arguments"` phrasing, which the descriptor's per-position
  rendering (`"()"`) cannot reproduce, so `expected_arguments` remains a
  hand-authored static and the parity harness opts out of that row — exactly the
  precedent set by io (plan-72-N). Everything else (membership, arity, param
  names, return type, `resolve_call`) is descriptor-derived. os has no builtin
  types, no source companion, and no resolver.
