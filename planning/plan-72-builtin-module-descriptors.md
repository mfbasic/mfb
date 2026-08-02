# plan-72: builtin module descriptors

Last updated: 2026-08-01
Overall Effort: huge (> 3d)

This plan replaces every hand-written package metadata module under
`src/builtins/` with a descriptor-based compiler plugin interface: each package
exposes one `BuiltinModule` containing its functions, types, source companion
rule, and optional resolver behavior. The single behavioral outcome is that all
current calls, named arguments, source injection, implementation-name rewrites,
builtin type lookups, IR verification, runtime helper selection, and generated
artifacts remain unchanged while every builtin package is queried through the
descriptor registry instead of package-specific free-function chains.

The work is split by landing order. `A` establishes the descriptor vocabulary
and registry once. Every subsequent letter migrates exactly one builtin
package. Do not start a later letter until the previous letter is complete and
committed, unless the letter is explicitly documented as parallelizable in its
own file.

References:

- `.ai/compiler.md` — compiler/runtime completion gate and required acceptance
  commands.
- `bugs/completed/bug-340-builtins-cli-reorg.md` — current known duplication in
  builtin metadata chains; this plan finishes the cleanup for every package.
- `bugs/completed/bug-349-datetime-named-arg-misbinding.md` — datetime overload
  name binding is fragile and must be preserved by descriptor overload selection.
- `bugs/completed/bug-173-builtins-syntaxcheck-typecheck-nits.md` — named
  argument metadata is correctness-sensitive.
- `src/builtins/mod.rs` — current aggregate helper chains and package source
  glue.
- `src/syntaxcheck/builtins.rs` — existing `BuiltinPackage` table precedent.
- `src/ir/lower.rs` — builtin argument normalization, default padding, and
  implementation-name rewrites.
- `src/ir/verify/compat.rs` and `src/target/shared/code/type_utils.rs` — typed
  return validation seams.
- `src/docs/spec/architecture/09_modules.md`,
  `src/docs/spec/language/18_builtin-functions.md`, and the per-package stdlib
  spec files under `src/docs/spec/stdlib/`.

## Prerequisites

These are a precondition on the whole feature. Every letter re-runs this table
before starting.

Baselines updated after letter A landed (A added the `descriptor.rs`
infrastructure module and its test references). See Corrections.

| Must be true | Command | Status |
|---|---|---|
| Only plan-72 files listed in this overview are present | `find planning planning/completed -type f -name 'plan-72*' \| sort -u \| wc -l → 29` (was `find planning planning/completed … → matches roadmap`; that command double-counts anything under `planning/completed` because `planning` already contains it — `sort -u` de-dupes) | MET on 2026-08-01 (re-verified for B) |
| The 26 builtin **packages** still exist | `ls src/builtins/*.rs \| grep -vE 'mod.rs\|descriptor.rs' \| wc -l → 26` (excludes A's `descriptor.rs` infrastructure module, which is not a package) | MET on 2026-08-01 (re-verified for B) |
| Total builtin descriptor-owned helper population is unchanged for this plan | `grep -cE '^(pub\(crate\) )?fn (is_.*call\|call_param_names\|call_return_type_name\|arity\|resolve_call\|implementation_name\|argument_types\|expected_arguments\|uses_package\|source_file\|augmented_project\|default_argument_padding\|param_types\|builtin_type_fields\|is_builtin_type\|call_param_name_overloads\|resolve_overload_target\|is_overloaded)' src/builtins/*.rs \| awk -F: '{s+=$2} END {print s}' → 209` (descriptor.rs's `DefaultResolver` methods are indented inside `impl`, so `^fn`/`^pub(crate) fn` does not match them; count stays 209 through BB, which deletes the wrappers) | MET on 2026-08-01 (re-verified for B) |
| Total direct helper call sites across all builtins is unchanged | `rg -o 'builtins::[a-z_]+::[a-zA-Z0-9_]+' src \| wc -l → 451` (was 449; +2 are A's `crate::builtins::app::source_file` references in `descriptor.rs` tests; B adds `crate::builtins::app::APP` in the production registry — these are descriptor plumbing, not new legacy-helper dispatch) | MET on 2026-08-01 (re-verified for B, remeasured per letter) |

Everything below is written against the world where these hold. If any row
changes, update the measured-population table in this overview before editing
code.

## Goal

- Every module under `src/builtins/*.rs` (except `mod.rs` and `descriptor.rs`)
  exposes a `pub(crate) static <MODULE>: BuiltinModule`.
- Existing consumers query a `BuiltinRegistry`/descriptor API for membership,
  arity, parameter names and aliases, argument types, return types, default
  padding, implementation names, builtin types, and source injection.
- Compatibility wrappers may survive temporarily during the plan, but the final
  letter (`BB`) removes the descriptor-duplicating free functions and the direct
  dispatcher chains they fed.

### Non-goals

- No public language surface change: names, aliases, overloads, arity ranges,
  diagnostics, runtime behavior, and generated AST/IR/native artifacts stay
  unchanged.
- No golden re-baseline to hide drift. A changed golden is a bug unless the plan
  proves the old golden wrong under AGENTS.md.
- No string parsing for descriptor-owned parameter or return types. The
  descriptor is the machine-readable source of truth.
- No fallback resolver that silently accepts unsupported cases. Unsupported
  means the descriptor or resolver is incomplete.

## Current State

The 26 builtin modules span 14,860 lines (`wc -l src/builtins/*.rs → 14860
total`, includes `mod.rs`; per-package LOC in the census table below). They
export 209 public metadata helper functions across the descriptor-owned surface
(`grep -cE '^(pub\(crate\) )?fn (is_.*call|...)' src/builtins/*.rs | awk ... →
209`). Their consumers make 449 direct calls into those package helpers
(`rg -o 'builtins::[a-z_]+::[a-zA-Z0-9_]+' src | wc -l → 449`).

The central aggregate helpers in `src/builtins/mod.rs` (`resolve_call_return_type`,
`call_return_type_name`, `is_builtin_call`, `call_param_name_overloads`, and
`call_param_names`) dispatch by explicit package chains for all 26 packages.
`src/syntaxcheck/builtins.rs` already has a value-level `BuiltinPackage` table,
but its rows still point at module-specific function pointers.
`src/ir/lower.rs:builtin_argument_types`, `normalize_builtin_call_arguments`,
default argument padding, and implementation-name selection are the
highest-risk consumers because they affect runtime IR shape.
`src/ir/verify/compat.rs` and `src/target/shared/code/type_utils.rs:static_nir_value_type`
are secondary return-type oracles and must be kept in lockstep.

Source injection is triplicated in `src/resolver/mod.rs`, `src/syntaxcheck/mod.rs`,
and `src/ir/lower.rs`, each with the same package ordering.

### Measured populations (per package)

Columns:

- `LOC` — `wc -l src/builtins/<pkg>.rs`
- `helpers` — count of descriptor-owned metadata helper `fn`s per the
  Prerequisites regex
- `srcglue` — 1 if the module invokes `package_source_glue!`, else 0
- `btypes` — count of `is_builtin_type` / `builtin_type_fields` / `param_types`
  fns present
- `custom` — count of `call_param_name_overloads` / `resolve_overload_target` /
  `is_overloaded` / `implementation_name` / `default_argument_padding` fns
- `fixtures` — `find tests/{syntax,rt-behavior,byte-identity} -path
  '*/<pkg>/*/project.json' | wc -l`

| Letter | Package | LOC | helpers | srcglue | btypes | custom | fixtures |
|---|---|---:|---:|---:|---:|---:|---:|
| B | app | 178 | 8 | 1 | 1 | 0 | 6 |
| C | audio | 738 | 13 | 1 | 2 | 2 | 6 |
| D | bits | 237 | 6 | 0 | 0 | 0 | 18 |
| E | collections | 1355 | 10 | 0 | 0 | 0 | 50 |
| F | crypto | 814 | 12 | 1 | 1 | 2 | 5 |
| G | csv | 162 | 7 | 1 | 0 | 1 | 2 |
| H | datetime | 923 | 11 | 1 | 1 | 3 | 12 |
| I | encoding | 596 | 10 | 1 | 0 | 3 | 29 |
| J | errorcode | 118 | 0 | 0 | 0 | 0 | 1 |
| K | fs | 713 | 7 | 0 | 1 | 0 | 98 |
| L | general | 815 | 6 | 0 | 0 | 0 | 26 |
| M | http | 581 | 9 | 1 | 1 | 2 | 7 |
| N | io | 236 | 8 | 0 | 2 | 0 | 31 |
| O | json | 251 | 8 | 1 | 1 | 1 | 8 |
| P | math | 616 | 6 | 0 | 0 | 0 | 45 |
| Q | money | 189 | 8 | 1 | 1 | 0 | 6 |
| R | net | 725 | 11 | 1 | 2 | 2 | 46 |
| S | os | 274 | 6 | 0 | 0 | 0 | 35 |
| T | regex | 298 | 11 | 0 | 0 | 2 | 6 |
| U | resource | 364 | 0 | 0 | 0 | 0 | 0 |
| V | strings | 875 | 10 | 0 | 0 | 1 | 16 |
| W | term | 477 | 9 | 1 | 3 | 0 | 33 |
| X | testing | 175 | 1 | 0 | 0 | 0 | 8 |
| Y | thread | 840 | 7 | 0 | 1 | 0 | 0 |
| Z | tls | 427 | 10 | 0 | 1 | 1 | 13 |
| AA | vector | 770 | 7 | 1 | 1 | 1 | 23 |

### Verified properties

- The descriptor core can start behind wrappers because existing aggregate
  helpers already mediate most consumers; verified by reading
  `src/builtins/mod.rs:resolve_call_return_type`, `call_param_names`,
  `call_return_type_name`, and `is_builtin_call`.
- Datetime cannot be treated as a flat merged parameter table; verified by
  `bugs/completed/bug-349-datetime-named-arg-misbinding.md` and
  `src/builtins/datetime.rs:call_param_name_overloads`.
- Term owns both hard-coded builtin types (`TermColor`, `TermSize`) and source
  companion types (`LineStyle`, `FillStyle`); verified by reading
  `src/builtins/term.rs:is_builtin_type`, `builtin_type_fields`, and
  `package_source_glue!`.
- `errorcode` and `resource` expose zero descriptor-owned helpers today; their
  letters are documentation-scale, not behavior-scale (still land the
  `BuiltinModule` static so the registry is exhaustive).
- Every listed package already has byte-identity or runtime-behavior coverage
  except `thread` and `resource`; those two letters explicitly call out that
  their gate is `cargo test` plus targeted syntax fixtures. Verified by the
  census `fixtures` column above.

## Design Overview

Add a descriptor model in `src/builtins/descriptor.rs`:

- `BuiltinModule`: package name, functions, builtin types, optional source, and
  resolver.
- `BuiltinFunction`: public name, documentation slug, overloads, implementation,
  lowering, and flags.
- `BuiltinOverload`: min/max args, parameter slice, return type.
- `Parameter`: canonical name, aliases, parameter type, default value.
- `BuiltinType`: name and `TypeKind` (`Primitive`, `Opaque`, `Record`, `Enum`).
- `BuiltinSource`: injection rule plus parser/source loader.
- `BuiltinResolver`: optional hooks for return-type selection, implementation
  selection, default padding, overload target resolution, and custom source-use
  predicates.

Keep the first implementation intentionally conservative: descriptors use the
existing `String` type names and `&'static str` implementation names at the
boundary, but the internal enum shape must not require parsing diagnostic prose.
If a strongly typed `TypeId` exists or is introduced later, it can replace the
string payloads behind the same descriptor API. The registry is a static slice
for deterministic order, not a runtime `HashMap`, unless measurement proves
lookup cost matters.

Correctness risk concentrates where descriptors replace resolution behavior:
datetime named-argument overloads, datetime default padding and implementation
dispatch, encoding typed overloads and monomorph target resolution, audio /
crypto / http / net / json / vector / tls / regex / strings / csv custom
behavior surfaces, and source injection order. The per-package letters land in
alphabetical order so the schedule is predictable; the higher-risk letters
(H datetime, I encoding, R net, C audio) can be re-ordered if a blocker
appears, but the descriptor core must still land first.

Rejected alternatives:

- Flip all consumers directly to new data in one change. Rejected because 449
  direct helper call sites and multiple type oracles make regression
  localization poor (`rg -o 'builtins::[a-z_]+::[a-zA-Z0-9_]+' src | wc -l →
  449`).
- Store only documentation strings and keep parsing them for argument types.
  Rejected because bug-340 already identifies `expected_arguments` parsing as a
  drift surface.
- Model source companion types only in `.mfb` package source. Rejected because
  `term` already splits `TermColor`/`TermSize` and `LineStyle`/`FillStyle`; the
  descriptor must describe all builtin types uniformly.
- Bundle multiple packages into a single letter. Rejected because 209 helpers
  across 26 packages produced too much churn per letter in the previous
  five-target scoping; per-package letters keep each landing reviewable and
  give parity tests a clean per-module boundary.

## Compatibility / Format Impact

No public format, package, or ABI change is intended. The implementation is
allowed to change private Rust APIs in `src/builtins/**` and internal tests. Any
AST, IR, byte-identity, runtime, man, or spec output drift must be investigated
as a regression unless proven stale under AGENTS.md.

## Sub-plan Roadmap

- [plan-72-A](plan-72-A-descriptor-core.md): descriptor types, registry API,
  and compatibility wrappers with no behavior change.
- [plan-72-B](plan-72-B-app.md): migrate `app`.
- [plan-72-C](plan-72-C-audio.md): migrate `audio`.
- [plan-72-D](plan-72-D-bits.md): migrate `bits`.
- [plan-72-E](plan-72-E-collections.md): migrate `collections`.
- [plan-72-F](plan-72-F-crypto.md): migrate `crypto`.
- [plan-72-G](plan-72-G-csv.md): migrate `csv`.
- [plan-72-H](plan-72-H-datetime.md): migrate `datetime` (custom resolver).
- [plan-72-I](plan-72-I-encoding.md): migrate `encoding` (custom resolver).
- [plan-72-J](plan-72-J-errorcode.md): migrate `errorcode` (descriptor-only).
- [plan-72-K](plan-72-K-fs.md): migrate `fs`.
- [plan-72-L](plan-72-L-general.md): migrate `general`.
- [plan-72-M](plan-72-M-http.md): migrate `http`.
- [plan-72-N](plan-72-N-io.md): migrate `io`.
- [plan-72-O](plan-72-O-json.md): migrate `json`.
- [plan-72-P](plan-72-P-math.md): migrate `math`.
- [plan-72-Q](plan-72-Q-money.md): migrate `money`.
- [plan-72-R](plan-72-R-net.md): migrate `net`.
- [plan-72-S](plan-72-S-os.md): migrate `os`.
- [plan-72-T](plan-72-T-regex.md): migrate `regex`.
- [plan-72-U](plan-72-U-resource.md): migrate `resource` (descriptor-only).
- [plan-72-V](plan-72-V-strings.md): migrate `strings`.
- [plan-72-W](plan-72-W-term.md): migrate `term`.
- [plan-72-X](plan-72-X-testing.md): migrate `testing`.
- [plan-72-Y](plan-72-Y-thread.md): migrate `thread`.
- [plan-72-Z](plan-72-Z-tls.md): migrate `tls`.
- [plan-72-AA](plan-72-AA-vector.md): migrate `vector`.
- [plan-72-BB](plan-72-BB-aggregate-cleanup.md): delete the duplicated
  free-function surface, collapse aggregate dispatch to registry iteration, and
  update spec/man citations.

## Validation Plan

- Unit tests: descriptor/registry parity tests for every package function,
  overload, parameter alias, return type, default, implementation name,
  builtin type, and source injection rule.
- Full Rust suite: `cargo test`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- Byte identity: run the repo's artifact/byte-identity gate used for native
  codegen changes; if the exact script name has moved, locate it with
  `rg -n 'byte-identity|artifact-gate' scripts tests`.
- Runtime proof: execute existing `tests/rt-behavior/<pkg>` fixtures through
  the acceptance runner and confirm `.run` output is unchanged for every
  migrated package.
- Doc sync: update `src/docs/spec/architecture/09_modules.md` so the builtin
  metadata source of truth points at descriptors; update stdlib/spec
  references only if symbol names or citations move.

## Open Decisions

- Whether `ParameterType` should immediately use an existing compiler `TypeId`
  or preserve normalized type strings behind the descriptor API. Recommendation:
  start with normalized strings in A, because no source inspection in this plan
  found a stable global `TypeId` suitable for static builtin metadata.
- Whether packages with zero descriptor-owned helpers today (`errorcode`,
  `resource`) still deserve a `BuiltinModule` static. Recommendation: yes — the
  registry stays exhaustive so `BB` can delete the aggregate arms
  unconditionally.
- Whether the per-package letters may land out of alphabetical order when a
  reviewer or bug forces re-scheduling. Recommendation: yes, provided the
  letter's `Depends on:` header still lists only `plan-72-A`; letters do not
  depend on each other.

## Corrections

- **Prerequisites baselines shifted once letter A landed (corrected during B).**
  A added `src/builtins/descriptor.rs` (the descriptor infrastructure) and test
  references to it, which the original Prerequisites commands did not anticipate:
  - Row 1 command double-counts: `find planning planning/completed` descends into
    `planning/completed` twice (it is a subdir of `planning`), so the archived
    `plan-72-A` was counted twice → `30`. Fixed with `sort -u | wc -l → 29`.
  - Row 2 measured `27` because `descriptor.rs` is a non-package file under
    `src/builtins/`. Command now excludes it → `26` packages.
  - Row 4 measured `451` (was `449`): +2 from A's `crate::builtins::app::source_file`
    references in `descriptor.rs` tests. B adds one more
    (`crate::builtins::app::APP` in the production registry). These are descriptor
    plumbing, not legacy-helper dispatch; expected count grows per migrated letter.
  - Row 3 stayed `209` (descriptor's `DefaultResolver` methods are indented inside
    `impl`, so the `^fn` anchor does not count them). MET unchanged.
  None of these indicated a blocker — letter A (the actual dependency) is complete;
  the table was a pre-A snapshot. Re-verified all rows MET before starting B.
- **Corrected A's `DefaultResolver` zero-argument renderings (done in B).** A's
  `expected_arguments` rendered `""` and `argument_types` returned `Some([])` for a
  zero-parameter call. The shared convention (proven across `app`, `money`,
  `crypto`, `datetime`) is `"()"` and `None`. Fixed in `descriptor.rs` and pinned
  by new unit tests; this is a general vocabulary fix, not app-specific tuning
  (respects A's non-goal). Evidence: `rg -n '=> "\(\)"' src/builtins/` (4 packages).

## Summary

This is a metadata authority migration, not a language feature. The risk is not
the descriptor structs; it is keeping every existing resolver, named-argument,
source-injection, and codegen-target behavior byte-for-byte equivalent while
removing the duplicated free-function surface across all 26 builtin packages.
