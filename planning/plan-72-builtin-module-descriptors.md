# plan-72: builtin module descriptors

Last updated: 2026-07-31
Overall Effort: huge (> 3d)

This plan replaces the five hand-written package metadata modules `strings`,
`datetime`, `encoding`, `money`, and `term` with a descriptor-based compiler
plugin interface: each package exposes one `BuiltinModule` containing its
functions, types, source companion rule, and optional resolver behavior. The
single behavioral outcome is that all current calls, named arguments, source
injection, implementation-name rewrites, builtin type lookups, IR verification,
runtime helper selection, and generated artifacts remain unchanged while those
five packages are queried through the descriptor registry instead of package
specific free-function chains.

The work is split by landing order. Do not start a later letter until the
previous letter is complete and committed.

References:

- `.ai/compiler.md` — compiler/runtime completion gate and required acceptance
  commands.
- `bugs/completed/bug-340-builtins-cli-reorg.md` — current known duplication in
  builtin metadata chains; this plan finishes the cleanup for the five packages.
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
  `src/docs/spec/language/18_builtin-functions.md`,
  `src/docs/spec/stdlib/02_datetime.md`,
  `src/docs/spec/stdlib/13_money.md`.

## Prerequisites

These are a precondition on the whole feature. Every letter re-runs this table
before starting.

| Must be true | Command | Status |
|---|---|---|
| No existing plan-72 files are present | `find planning planning/completed -type f -name 'plan-72*' \| sort → no output` | MET on 2026-07-31 |
| The five target builtin modules still exist | `wc -l src/builtins/{strings,datetime,encoding,money,term}.rs → 875, 923, 596, 189, 477` | MET on 2026-07-31 |
| The descriptor target still excludes already-broader builtin packages | `rg -n '^(pub\\(crate\\) )?fn (is_.*call\|call_param_names\|call_return_type_name\|arity\|resolve_call\|implementation_name\|argument_types\|expected_arguments\|uses_package\|source_file\|augmented_project\|default_argument_padding\|param_types\|builtin_type_fields\|is_builtin_type\|call_param_name_overloads\|resolve_overload_target\|is_overloaded)' src/builtins/{strings,datetime,encoding,money,term}.rs \| wc -l → 48` | MET on 2026-07-31 |

Everything below is written against the world where these hold. If any row
changes, update the measured-population table in this overview before editing
code.

## Goal

- `src/builtins/{strings,datetime,encoding,money,term}.rs` each expose a
  `pub(crate) static <MODULE>: BuiltinModule`.
- Existing consumers query a `BuiltinRegistry`/descriptor API for membership,
  arity, parameter names and aliases, argument types, return types, default
  padding, implementation names, builtin types, and source injection.
- Compatibility wrappers may survive temporarily during the plan, but the final
  letter removes the five-package descriptor-duplicating free functions and the
  direct dispatcher chains they fed.

### Non-goals

- No public language surface change: names, aliases, overloads, arity ranges,
  diagnostics, runtime behavior, and generated AST/IR/native artifacts stay
  unchanged.
- No blanket migration of all builtin packages. This plan targets exactly
  `strings`, `datetime`, `encoding`, `money`, and `term`.
- No golden re-baseline to hide drift. A changed golden is a bug unless the plan
  proves the old golden wrong under AGENTS.md.
- No string parsing for descriptor-owned parameter or return types. The descriptor
  is the machine-readable source of truth.
- No fallback resolver that silently accepts unsupported cases. Unsupported means
  the descriptor or resolver is incomplete.

## Current State

The five target modules expose 48 public metadata helper functions
(`rg ... src/builtins/{strings,datetime,encoding,money,term}.rs | wc -l → 48`)
over 3,060 lines (`wc -l src/builtins/{strings,datetime,encoding,money,term}.rs
→ 875+923+596+189+477`). Their consumers make 70 direct calls into those package
helpers (`rg -o 'builtins::(strings|datetime|encoding|money|term)::...' src |
wc -l → 70`).

The central aggregate helpers in `src/builtins/mod.rs:resolve_call_return_type`,
`call_return_type_name`, `is_builtin_call`, `call_param_name_overloads`, and
`call_param_names` dispatch by explicit package chains. `src/syntaxcheck/builtins.rs`
already has a value-level `BuiltinPackage` table, but its rows still point at
module-specific function pointers. `src/ir/lower.rs:builtin_argument_types`,
`normalize_builtin_call_arguments`, default argument padding, and
implementation-name selection are the highest-risk consumers because they affect
runtime IR shape. `src/ir/verify/compat.rs` and
`src/target/shared/code/type_utils.rs:static_nir_value_type` are secondary return
type oracles and must be kept in lockstep.

Source injection is triplicated in `src/resolver/mod.rs`, `src/syntaxcheck/mod.rs`,
and `src/ir/lower.rs`, each with the same package ordering. The five target
packages currently have source companions via `package_source_glue!` for
`datetime`, `money`, `term`, and `encoding`, while `strings` has custom
`uses_package` logic for scalar seam references.

### Measured populations

| What | Count | Command |
|---|---:|---|
| Target module LOC | 3,060 | `wc -l src/builtins/{strings,datetime,encoding,money,term}.rs → 875, 923, 596, 189, 477` |
| Public target metadata helpers | 48 | `rg -n '^(pub\\(crate\\) )?fn (is_.*call\|call_param_names\|call_return_type_name\|arity\|resolve_call\|implementation_name\|argument_types\|expected_arguments\|uses_package\|source_file\|augmented_project\|default_argument_padding\|param_types\|builtin_type_fields\|is_builtin_type\|call_param_name_overloads\|resolve_overload_target\|is_overloaded)' src/builtins/{strings,datetime,encoding,money,term}.rs \| wc -l → 48` |
| Direct calls into target package helpers | 70 | `rg -o 'builtins::(strings\|datetime\|encoding\|money\|term)::[A-Za-z0-9_]+\|crate::builtins::(strings\|datetime\|encoding\|money\|term)::[A-Za-z0-9_]+' src \| wc -l → 70` |
| Target package function-name constants and literal references | 167 | `for p in strings datetime encoding money term; do rg -o '"$p\\.[A-Za-z0-9]+"\|const [A-Z0-9_]+: &str = "$p\\.[A-Za-z0-9]+"' src/builtins/$p.rs \| wc -l; done → 41,51,40,5,30` |
| Target package man pages | 147 | `for p in strings datetime encoding money term; do find src/docs/man/builtins/$p -type f -name '*.md' \| wc -l; done → 39,46,33,4,25` |
| Target package fixtures | 101 | `find tests/syntax tests/rt-behavior tests/byte-identity -path '*/strings/*' -o -path '*/datetime/*' -o -path '*/encoding/*' -o -path '*/money/*' -o -path '*/term/*' \| grep '/project.json$' \| wc -l → 101` |
| Fixture split by package | 16/12/29/6/33 | `for p in strings datetime encoding money term; do find tests/syntax tests/rt-behavior tests/byte-identity -path "*/$p/*/project.json" \| wc -l; done → strings 16, datetime 12, encoding 29, money 6, term 33` |

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
- The target packages already have byte-identity coverage; verified by
  `find tests/byte-identity -path '*/{strings,datetime,encoding,money,term}/*/project.json'`
  returning one project per target package.

## Design Overview

Add a descriptor model in `src/builtins/mod.rs` or a new
`src/builtins/descriptor.rs`:

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
dispatch, encoding typed overloads and monomorph target resolution, term builtin
type fields, and source injection order. Those land after the core and low-risk
data-only modules.

Rejected alternatives:

- Flip all consumers directly to new data in one change. Rejected because 70
  direct helper calls and multiple type oracles make regression localization poor
  (`rg -o ... src | wc -l → 70`).
- Store only documentation strings and keep parsing them for argument types.
  Rejected because bug-340 already identifies `expected_arguments` parsing as a
  drift surface.
- Model source companion types only in `.mfb` package source. Rejected because
  `term` already splits `TermColor`/`TermSize` and `LineStyle`/`FillStyle`; the
  descriptor must describe all builtin types uniformly.

## Compatibility / Format Impact

No public format, package, or ABI change is intended. The implementation is
allowed to change private Rust APIs in `src/builtins/**` and internal tests. Any
AST, IR, byte-identity, runtime, man, or spec output drift must be investigated
as a regression unless proven stale under AGENTS.md.

## Sub-plan Roadmap

- [plan-72-A](plan-72-A-descriptor-core.md): descriptor types, registry API, and
  compatibility wrappers with no behavior change.
- [plan-72-B](plan-72-B-data-modules.md): migrate data-shaped `money`, `term`,
  and most of `strings`.
- [plan-72-C](plan-72-C-source-and-type-registry.md): source injection and builtin
  type lookup through descriptors.
- [plan-72-D](plan-72-D-custom-resolvers.md): migrate custom `datetime` and
  `encoding` resolver behavior.
- [plan-72-E](plan-72-E-delete-free-function-surface.md): switch all direct
  consumers, remove duplicated five-package helper APIs, and run full validation.

## Validation Plan

- Unit tests: descriptor/registry parity tests for every target function,
  overload, parameter alias, return type, default, implementation name, builtin
  type, and source injection rule.
- Full Rust suite: `cargo test`.
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.
- Byte identity: run the repo's artifact/byte-identity gate used for native
  codegen changes; if the exact script name has moved, locate it with
  `rg -n 'byte-identity|artifact-gate' scripts tests`.
- Runtime proof: execute existing `tests/rt-behavior/{strings,datetime,encoding,money,term}`
  through the acceptance runner and confirm `.run` output is unchanged.
- Doc sync: update `src/docs/spec/architecture/09_modules.md` so the builtin
  metadata source of truth points at descriptors; update stdlib/spec references
  only if symbol names or citations move.

## Open Decisions

- Whether `ParameterType` should immediately use an existing compiler `TypeId` or
  preserve normalized type strings behind the descriptor API. Recommendation:
  start with normalized strings in A, because no source inspection in this plan
  found a stable global `TypeId` suitable for static builtin metadata.
- Whether the registry should include all builtin packages immediately or only
  the five target modules. Recommendation: only the five target modules for
  plan-72, with an API shape that can absorb the rest later.

## Corrections

Filled during execution.

## Summary

This is a metadata authority migration, not a language feature. The risk is not
the descriptor structs; it is keeping every existing resolver, named-argument,
source-injection, and codegen-target behavior byte-for-byte equivalent while
removing the duplicated five-package free-function surface.
