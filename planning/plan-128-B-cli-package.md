# plan-128-B: `cli` source package

Last updated: 2026-09-09
Effort: medium (1h–2h)
Depends on: plan-128-A-cli-argv-contract

This sub-plan creates `packages/cli`, a distributable MFBASIC source package
for declarative command-line options. A caller describes canonical names,
short aliases, requiredness, and value kind once; `cli::parse` reads the host
arguments once, caches the raw name-to-text map, and the typed accessors return
validated values or the documented error. `cli::showUsage` renders the same
schema, preventing parser/help drift.

References:

- `planning/plan-128-A-cli-argv-contract.md` — required `os::prog` and
  malformed-argument boundary.
- `mfb spec language modules-and-packages` and `mfb spec package globals` —
  source-package exports and package-level `MUT` state.
- `packages/yaml/project.json`, `packages/yaml/src/*.mfb`, and
  `packages/yaml/README.md` — manifest, multi-file source, tests, and docs
  precedent.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| plan-128-A is complete, including its UTF-8 runtime proof and full gates | `test -f planning/completed/plan-128-A-cli-argv-contract.md` | MET |

Everything below is written against an `os` package with `args()` that returns
only validated text and `prog()` that returns argv[0]. If that prerequisite is
not complete, this plan cannot start.

## 1. Goal

- An importer can build a `List OF cli::Option`, call `cli::parse(options)` once
  to obtain canonical-name-to-raw-text values, use `getInteger`, `getString`,
  `getBool`, or `getFlag` for typed values, and render a matching usage message;
  missing required options, malformed input, unknown switches, duplicate
  switches, and wrong typed access all fail deterministically.

### Non-goals

- No built-in package, compiler feature, native runtime helper, or new command
  syntax is added by this letter.
- No positional arguments, subcommands, repeated options, short-option bundles,
  `--` terminator, default-from-environment behavior, or GNU `--name=value`
  spelling unless a later plan explicitly expands the grammar.
- No implicit numeric conversion in `parse`: its map values remain the original
  option text. Typed conversion is the accessor’s job.
- No concurrent first-use guarantee: callers must parse before starting worker
  threads. The cache is package state, not a synchronization service.

## 2. Current State

- There is no `packages/cli` directory. The current six package directories
  are listed by `find packages -mindepth 1 -maxdepth 1 -type d -print | sort`.
- Source packages can export `TYPE`, `FUNC`, `LET`, and `MUT`; top-level
  `EXPORT MUT` is package state visible to importers (`src/docs/spec/language/13_modules-and-packages.md`,
  `src/docs/spec/package/06_globals.md`). A private module cache therefore
  remains in the package binary and is shared by its importer.
- `os::args()` deliberately omits the program name and `os::executablePath()`
  is an absolute executable path (`src/codegen/builtins/os/func_args.rs`,
  `func_executable_path.rs`); plan-128-A supplies the required boundary.
- `toInt(String)` already raises `ErrInvalidFormat` for malformed integer text
  (`src/codegen/builtins/general/func_to_int.rs`); `cli` must add context, not
  quietly coerce values.

### Measured populations

| What | Count | Command |
|---|---:|---|
| Existing source-package directories | 6 | `find packages -mindepth 1 -maxdepth 1 -type d -print \| sort \| wc -l` |
| Existing package source files | 50 | `find packages -path '*/src/*.mfb' -type f \| wc -l` |
| Existing package test files | 22 | `rg -l '^TESTING$' packages -g '*.mfb' \| wc -l` |
| Existing exported package source files | 9 | `rg -l '^EXPORT (TYPE\|FUNC\|MUT\|LET)' packages -g '*.mfb' \| wc -l` |

### Verified properties

- Package globals are serialized and merged into an importer. Verified by
  reading `src/docs/spec/package/06_globals.md` and
  `src/resolver/packages.rs:package_exports` (GLOBAL_TABLE is incorporated into
  imported package visibility).
- `Option{name, alias, required}` alone cannot tell a parser whether `-f`
  takes no value or whether `-i` must take one. This follows from the proposed
  `-f -i 1` token stream: consuming the next token cannot be decided after the
  fact by `getFlag`.

## 3. Design Overview

`packages/cli/project.json` declares a normal source package named `cli`.
`src/model.mfb` exports `OptionKind` (`Flag`, `Bool`, `Integer`, `String`) and
an `Option` record with canonical `name`, optional `alias`, `required`, and
`kind`. The recommended alias representation is `String` rather than `Scalar`:
the empty string unambiguously means no short alias and validation can require
exactly one scalar when non-empty. The public API uses canonical names without
leading dashes.

`src/parse.mfb` tokenizes `os::args()` once. It accepts exactly `--name value`
and `-a value` for value kinds, and `--flag`/`-f` for flags. It validates the
schema before reading input, rejects unrecognized/duplicate options and
missing values, retains the textual value in `Map OF String TO String`, checks
required presence after scanning, and stores both the schema identity/result in
private package globals. Repeated `parse` with an equal schema returns the
cached map without reading or reparsing; a non-equal schema after initialization
raises `ErrInvalidArgument` rather than producing a cache whose schema and map
disagree.

`src/get.mfb` performs presence/default handling and verifies that the requested
accessor matches the schema kind. `getInteger` delegates text conversion to
`toInt`; `getBool` accepts only `true`, `false`, `1`, and `0` (case policy is an
open decision); `getString` returns unchanged text; `getFlag` returns true only
when present. `src/usage.mfb` produces a deterministic text layout through
`io::print`, with header and footer lines provided by the caller.

Correctness risk concentrates in cache/schema identity, token consumption, and
the error matrix. This is new behavior, so runtime package tests and an emitted
consumer executable are the acceptance gates; byte identity is not applicable.

Rejected alternative: expose only `parse(args, schema)`. It makes unit tests
easy but cannot fulfill the requested host-input cache API by itself. Keep any
explicit-argument parser private as a testable implementation helper, with
public `parse(schema)` as the one-shot host-facing entry. Rejected alternative:
infer flagness from which `get*` is later called; parsing must know whether to
consume the following token first.

## 4. Detailed Design

### 4.1 Proposed public surface

```mfb
IMPORT cli

TYPE Option
  name AS String
  alias AS String
  required AS Boolean
  kind AS cli::OptionKind
END TYPE

LET options AS List OF cli::Option = [
  cli::Option["port", "p", FALSE, cli::OptionKind.Integer],
  cli::Option["verbose", "v", FALSE, cli::OptionKind.Flag]
]
LET config AS Map OF String TO String = cli::parse(options)
LET port AS Integer = cli::getInteger("port", 3000)
LET verbose AS Boolean = cli::getFlag("verbose", FALSE)
```

The accessors intentionally use the cached schema/map rather than accepting a
caller-provided map: accepting arbitrary maps would make kind and requiredness
unverifiable. A private `parseArgs(args, schema)` supports package tests;
public `parse` calls it with `os::args()` only on cache initialization.

### 4.2 Error policy

Use `errorCode::ErrInvalidArgument` for invalid schemas, unknown/duplicate
switches, missing switch values, and getter/schema-kind mismatch;
`ErrNotFound` for an absent required option; propagate `ErrInvalidFormat` from
`toInt` for bad numeric text. `ErrEncoding` is raised by the plan-128-A `os`
boundary before package code begins. All errors name the canonical option and
the offending spelling/value where applicable.

### 4.3 Cache and usage

The private cache begins uninitialized, stores the accepted schema fingerprint
and raw map only after a complete successful parse, and is never partially
published after an error. `showUsage(header, schema, footer)` validates the
schema but does not initialize or alter the parsed-values cache. It prints the
program display name from `os::prog()`, options in supplied schema order, and
the caller’s header/footer line lists unchanged.

## Phases

### Phase 1 — Package scaffold and executable contract tests

Create the package boundary and pin the data model/error vocabulary before the
parser is written.

- [x] Add `packages/cli/project.json` (`name: cli`, `version: 0.1.0`,
  `kind: package`, source root `src`) and `packages/cli/README.md` with
  manifest-consumer instructions matching `packages/yaml/README.md`.
- [x] Add `packages/cli/src/model.mfb` with exported `OptionKind`, `Option`,
  error constants, schema validation, and exact public API documentation;
  reserve empty `alias` for no short form.
- [x] Add TESTING blocks for duplicate names/aliases, empty names, aliases that
  do not hold exactly one scalar, and every option kind.
- [x] Add a minimal imported-consumer fixture and package build/test commands
  to prove the `.mfp` exports resolve as `cli::Option` and `cli::OptionKind`.

Acceptance: `mfb build packages/cli` writes `packages/cli/cli.mfp`; `mfb test
packages/cli` passes schema-model tests; a separate consumer builds using the
documented `file:packages/cli/cli.mfp` manifest dependency.
Commit: 053f5af60, 1a7e8cbca

### Phase 2 — Parse/cache/accessor implementation and matrix

Implement behavior using a private explicit-argument worker and public
host-argument cache boundary.

- [x] Add `packages/cli/src/parse.mfb` with private `parseArgs`, canonical and
  alias lookup, strict token consumption, required checks, and only-on-success
  private `MUT` cache publication.
- [x] Add `packages/cli/src/get.mfb` with `getInteger`, `getString`, `getBool`,
  and `getFlag`; require matching `OptionKind`, return defaults only when an
  optional option is absent, and preserve raw map values for `parse`.
- [x] Add package TESTING cases for long/short forms; every value and flag
  spelling; `-i` lacking a value; bad integer text; `-b` lacking a value; all
  accepted boolean literals; absent required/optional cases; unknown,
  duplicate, and value-looking-switch cases; cache reuse and unequal-schema
  rejection.
- [x] Add a runnable consumer/smoke script that invokes the generated program
  with `-p 3000`, `--verbose`, `-b true`, `-b false`, `-b 1`, and `-b 0`, and
  checks stdout plus nonzero error exits for failures.

Acceptance: the complete documented token/error matrix runs through both the
private package tests and an actual command invocation; a second public parse
uses the cached result and never changes it after host arguments are consumed.
Commit: 0dc1eaa96, d55c8d886, f318ea7b5, bd20ece43

### Phase 3 — Usage renderer, docs, and final package proof

Finish the user-facing output and verify the distributable artifact.

- [x] Add `packages/cli/src/usage.mfb` implementing deterministic
  `showUsage(header, schema, footer)` with program name, short/long spellings,
  required marker, kind placeholder, and caller line ordering.
- [x] Extend README examples to use the final signatures, show typed/default
  behavior, explain one-shot cache and thread constraint, list supported
  syntax, and show how callers `TRAP` each error class.
- [x] Add exact-output tests for usage with no alias, required and optional
  options, all kinds, header, and footer; add the usage consumer to the smoke
  script.
- [x] Run package build/test/doc commands and rebuild the documented consumer
  from a clean temporary project without relying on in-tree imports.

Acceptance: usage output is stable and derives every option row from the same
schema parser accepts; README commands build, test, document, and consume the
generated `.mfp` successfully.
Commit: 0dc1eaa96, 1a929c8c1, 1aab6fc6c

## Validation Plan

- Tests: `mfb test packages/cli` covers model, parser, cache, typed getter,
  and usage rows; the external consumer smoke test covers actual `os::args()`
  invocation and output/error exits.
- Coverage check: list every exported `cli::` member and match it to package
  test and smoke names with `rg -n '^EXPORT (TYPE|FUNC)' packages/cli/src` and
  `rg -n 'cli::' packages/cli` before declaring coverage complete.
- Runtime proof: build an importer against `packages/cli/cli.mfp`, invoke it
  with the documented switches, and verify values, usage, and traps.
- Doc sync: `packages/cli/README.md`, package description in `project.json`,
  generated `mfb pkg doc packages/cli/cli.mfp`; no embedded stdlib spec changes
  belong to the package.
- Acceptance: `mfb build packages/cli`, `mfb test packages/cli`, package docs,
  the smoke script, and the repository’s full relevant suite after the
  plan-128-A prerequisite has landed.

## Open Decisions

- API shape — recommend `parse(schema)` plus cached getters that take only
  `(name, default)`, with private `parseArgs(args, schema)` for tests. This
  resolves the proposal’s incompatible `parse` and `get*` signatures while
  preserving the requested cache.
- Schema kind — recommend adding `OptionKind`; without it `parse` cannot know
  whether a switch consumes a value. Alternative: separate flag/value record
  variants, which is more verbose but equally sound.
- Boolean truth table — recommend `1`/`true` → true and `0`/`false` → false.
  The supplied examples say `0` and `false` are true, which conflicts with
  their stated intent and must not be guessed during implementation.
- Strict grammar — recommend only `--name value` and `-a value` initially,
  rejecting unknown/duplicate switches and all unlisted GNU conveniences. This
  keeps the first public contract small and makes later expansion additive.
- Required error — recommend `ErrNotFound` for an absent required named option
  and `ErrInvalidArgument` for malformed command syntax/schema. Alternative:
  use one code for both, but it removes a useful caller distinction.

## Corrections

- A `file:` package source is resolved relative to the importing project's
  `packages/` directory. The clean importer proof therefore links
  `packages/cli.mfp`, rather than using an absolute source path.

## Summary

The package is pure MFBASIC but relies on plan-128-A for raw-host correctness.
Its central design choice is an explicit option kind and a cache that is
published only after a complete successful parse; all parsing, getters, and
usage output then share one validated schema.
