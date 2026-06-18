# MFBASIC Audit Implementation Plan

Last updated: 2026-06-18

This document plans how to implement the `mfb audit` command required by
`specifications/mfbasic.md` and how to extend it into a useful source,
package, dependency, and runtime-risk audit surface for MFBASIC projects.

It complements:

- `specifications/mfbasic.md`
- `specifications/project.md`
- `specifications/lockfile.md`
- `specifications/package_format.md`
- `specifications/repository.md`
- `specifications/architecture.md`
- `specifications/linker.md`

## 1. Goal

Implement:

```text
mfb audit [--format text|json] [--locked] [path]
```

The command must report the audit data already required by the language
specification:

- fallible call sites
- auto-propagation paths
- `TRAP` recovery paths
- resource cleanup behavior
- native links
- package permissions
- dependency versions
- lockfile mismatches
- verifier status

`--locked` must require the resolved dependency graph to match `mfb.lock`.

The command should be useful both for humans reviewing a project locally and for
CI systems enforcing a policy. Text output is optimized for review. JSON output
is a stable machine-readable contract.

## 2. Current State

The language specification already requires `mfb audit` in
`specifications/mfbasic.md` section 22.

Related specifications already require the compiler and package format to
preserve audit-relevant metadata:

- `project.json` records project identity, source roots, dependencies, native
  metadata, and build/audit metadata.
- `mfb.lock` records exact dependency versions, package content hashes,
  signature metadata, ABI hashes, native requirements, and transparency-log
  checkpoint information.
- `.mfp` packages preserve package manifests, dependency metadata, public ABI
  metadata, native-link metadata, resource metadata, cleanup metadata, and
  verifier-relevant bytecode structure.
- package verification must reject malformed packages, invalid signatures,
  type-invalid bytecode, invalid control flow, bad resource ownership, and
  invalid native-link manifests before package code can run.
- resource cleanup metadata must preserve secondary close failures when a
  `USING` body and the close operation both fail.

The current CLI does not dispatch an `audit` command. `src/main.rs` lists
`help`, `init`, `init-pkg`, `pkg`, `build`, and `man`; it does not list
`audit`, `fmt`, `test`, or `lsp`.

### 2.1 Metadata Available Today

A survey of the implementation establishes what the first `mfb audit`
implementation can rely on and what is still missing:

- `bytecode::read_package_info` already exposes manifest identity, ABI format
  version, type/const/resource/function/global/export/import/cleanup counts,
  the export table (kind, name, sigHash), package state globals (mutability and
  visibility), imports (name, ident, version, pin, flags, used symbols), and
  resource cleanups (function, cleanup id, pc range, resource register, close
  function id, and the `records_secondary_close_failure` flag). `mfb pkg info`
  already prints `audit:` notes for exported mutable package state and for
  cleanups that record secondary close failures.
- `.mfp` headers carry container/bytecode versions, flags, and a signature type
  (`0` unsigned, `1` Ed25519). `target::package_mfp::package_content_hash`
  computes the deterministic content hash over the header prefix, zero-filled
  signature area, and bytecode.
- The resource model no longer uses a `USING` statement. Resources are released
  by lexical drop / RAII at scope exit (`specifications/mfbasic.md` §14.7, §15).
  Source-level resource bindings are therefore ordinary `LET`/`MUT` bindings
  whose initializer produces a resource handle (for example `fs::openFile`).
- There is no `mfb.lock` parser yet. The lockfile format is specified in
  `specifications/lockfile.md` but nothing reads it. The `LOCKFILE_MISMATCH`
  diagnostic exists in `rules.rs` but is unused.
- Native-link metadata (`LINK` libraries, symbols, ABI mappings) is **not**
  surfaced by the package reader. The manifest parser reads a native-link count
  and discards it. Until that metadata is exposed, native-link audit is limited
  to resource close functions that are not the built-in filesystem close.
- There is no cryptographic signature verifier. `mfb pkg verify` checks package
  name/ident/version against `project.json`, not Ed25519 signatures.

### 2.2 First Implementation Scope

The first `mfb audit` lands Phases 1–6 at the fidelity the available metadata
allows, and records every gap above as an explicit finding rather than as a
clean result:

- Full CLI contract: `--format text|json`, `--locked`, optional `path`, exit
  codes, deterministic ordering.
- Project summary, dependency graph (resolved from installed `.mfp` packages and
  compared to `project.json` requests), lockfile comparison, package verifier
  status, source control-flow (fallible call sites, auto-propagation, `TRAP`
  classification), resource cleanup behavior, host-capability permissions, and
  the native close-function inventory.
- A `projectHash` for `--locked` comparison is defined as the lowercase hex
  SHA-256 of a canonical, sorted serialization of the `project.json`
  `packages[]` request tuples (`name`, `ident`, `version`, `pin`, `source`).
- Deterministic, machine-independent output: source locations use
  project-relative paths and the report never prints absolute or canonicalized
  paths, so golden files are portable.

Native-link symbol/ABI audit, the policy file, transitive-capability graphs,
reproducibility scoring, the target matrix, online advisory checks, and the
identifier-similarity lint remain Phase 7 follow-ups.

## 3. Non-Goals

The first implementation should not create a separate parser, resolver,
typechecker, verifier, package reader, or dependency resolver. Audit must reuse
the same front-end, package, lockfile, native-target, and verifier paths used by
builds so that audit output matches real build behavior.

The first implementation should not execute user code. It may compile, verify,
lower, inspect generated metadata, and read packages. Runtime execution belongs
to `mfb test`, generated executables, or explicit future dynamic-analysis
commands.

The first implementation should not silently downgrade missing metadata to
"clean". Missing required audit metadata is itself an audit finding.

## 4. CLI Contract

### 4.1 Command Form

```text
mfb audit [--format text|json] [--locked] [path]
```

`path` defaults to the current directory. It may name a project directory or a
single source file accepted by the normal build pipeline.

`--format text` is the default. It emits a deterministic human-readable report.

`--format json` emits the JSON schema described in this document. The schema is
versioned so CI tools can consume it safely.

`--locked` requires dependency resolution to match `mfb.lock`. It must fail when
the lockfile is missing, stale, has a mismatched `projectHash`, selects a
different package version, records a mismatched content hash, records
incompatible ABI metadata, or has dependency data that disagrees with the
selected packages.

Unknown options are usage errors. Invalid `--format` values are usage errors.

### 4.2 Exit Codes

Recommended exit behavior:

| Exit code | Meaning |
| --------- | ------- |
| `0` | Audit completed and no error-severity findings were reported. |
| `1` | Audit completed and at least one error-severity finding was reported. |
| `2` | Command-line usage error. |
| `3` | Audit could not complete because project/package input was unreadable or malformed. |

Warnings alone should not fail the process unless a future policy option asks
for warning-as-error behavior.

### 4.3 Output Ordering

All output must be deterministic:

1. project summary
2. lockfile and dependency findings
3. package verifier findings
4. source control-flow findings
5. resource and cleanup findings
6. native-link findings
7. permission and capability findings
8. security lint findings
9. proposed-policy findings

Within each group, sort by package name, source path, line, column, symbol name,
and finding code as applicable.

## 5. Required Report Sections

### 5.1 Project Summary

Report:

- project name, ident, version, kind, and entry point
- project root and selected source files
- selected target platform when target-specific audit data is requested or
  needed
- package or executable build intent
- language version from `project.json`
- whether build/audit metadata is enabled in the manifest

### 5.2 Fallible Call Sites

Report every fallible call site, including calls hidden inside expressions,
argument lists, initializers, resource bindings, and condition expressions.

Each finding should include:

- source path, line, and column
- enclosing package, function, or subroutine
- callee name and package
- result/error type
- whether the call is explicitly handled or auto-propagated
- destination of the error edge: nearest `TRAP`, function return, or package
  initializer failure

### 5.3 Auto-Propagation Paths

Report each compiler-inferred propagation edge from a fallible call to the
enclosing `TRAP` or function return.

Each edge should include:

- source call site
- intermediate expression or statement context
- destination recovery or return point
- cleanup regions crossed while propagating
- whether any crossed cleanup region can itself fail

### 5.4 `TRAP` Recovery Paths

Report every `TRAP` recovery path and classify its behavior:

- returns successfully
- returns an error
- propagates the original error
- replaces the error with `FAIL`
- shadows or discards details from the original error

The report should identify dense or security-sensitive recovery blocks, such as
catch-all recovery around filesystem, network, native, process, or resource
operations.

### 5.5 Resource Cleanup Behavior

Report each lexical resource binding and cleanup region:

- resource type and close operation
- source path, line, and column of the acquisition
- whether the resource is standard or native
- whether close can fail
- the lexical drop-close edges that release the resource on each exit path
  (normal scope exit, `RETURN`, `FAIL`, `PROPAGATE`, auto-propagated `Err`, and
  `TRAP` routing)
- whether cleanup metadata records secondary close failures
- whether any cleanup path crosses thread or callback boundaries

If a scope holding a live resource can fail and the drop-close can also fail,
the report must state that the body error wins and the drop-close failure is
retained only as diagnostic/audit metadata (`mfbasic.md` §15).

Missing cleanup metadata for such a region is an error-severity finding.

### 5.6 Native Links

Report all native binding packages, linked native libraries, declared symbols,
ABI mappings, native resource close functions, and target-specific dynamic
dependency metadata used by the selected build.

Each native link entry should include:

- declaring package and source location when available
- library name, version constraint, platforms, and link mode
- symbol name and wrapper function
- ABI mapping, including `CString`, `CPtr`, `OUT`, `REF`, `SUCCESS_ON`, and
  `ERROR_ON` rules where applicable
- resource types exposed through the binding
- close functions and whether they can fail
- target linker import or dynamic dependency name

Audit must use compiled `.mfp` native-link metadata as authoritative for
imported packages. `project.json` native metadata is informational until a
matching `LINK` declaration is compiled.

### 5.7 Package Permissions And Host Capabilities

Report package and build use of host capabilities:

- filesystem
- network
- process execution or process metadata
- environment variables
- clock/timezone
- randomness
- native-library access

For standard packages, permissions should be inferred from the built-in package
and function used. For native packages, permissions should come from package
metadata and native-link declarations.

Extended audit reporting may also classify terminal/stdio use and thread
creation, synchronization, cancellation, and message passing as capability
findings. Those categories are useful for review, but they are beyond the
minimum permission categories required by `mfbasic.md` section 22.

Each permission entry should include:

- package and function requiring the permission
- source call sites where the capability is used
- transitive dependency responsible for the permission
- target platform notes when behavior differs by target
- whether the permission is direct, transitive, standard-library, or native

### 5.8 Dependency Versions

Report the full resolved dependency graph:

- import name
- ident
- selected version
- requested version
- pin state
- source locator
- content hash
- signature type
- ident fingerprint
- signing fingerprint
- signing key status
- publish timestamp
- ABI hash
- bytecode/container versions
- native requirements
- direct dependencies

For graph conflicts, report all conflicting requirers and the requested versions
or ABI hashes that caused the conflict.

### 5.9 Lockfile Mismatches

When a lockfile is present, report:

- `projectHash` match or mismatch
- package entries missing from the lockfile
- lockfile entries not used by the project
- version, hash, ABI, signature, native metadata, and dependency mismatches
- `signingKeyStatus = past` entries and whether `publishedAt` is earlier than
  `signingKeyRotatedAt`
- unsigned-local exceptions and their policy reason
- transparency-log checkpoint metadata

Under `--locked`, any mismatch that would cause resolution or verification to
use data other than the lockfile is an error.

### 5.10 Verifier Status

Report verification status for every imported `.mfp` package and any package
being emitted by the current project:

- container header validity
- signature header validity
- signature verification status under the active trust policy
- header and manifest identity match
- section bounds and required sections
- type metadata validity
- function metadata validity
- initialized register analysis
- branch target and control-flow validity
- trap and cleanup-region validity
- resource ownership validity
- native-link metadata validity
- ABI index consistency

Verifier failures must be reported as error-severity audit findings.

## 6. Proposed Additional Features

These features are not all explicitly required by the current language spec,
but they fit MFBASIC's goals: predictable builds, explicit effects, no hidden
install-time execution, and auditable native boundaries.

### 6.1 Risk Summary

Add a top-level risk summary with counts by severity and category:

- errors
- warnings
- informational findings
- dependency findings
- native findings
- permission findings
- resource findings
- verifier findings
- security lint findings

This lets CI and editors show a compact status without parsing every detail.

### 6.2 Stable Finding Codes

Define audit finding codes separate from compiler diagnostics, for example:

```text
AUDIT-LOCK-STALE
AUDIT-PKG-UNSIGNED-LOCAL
AUDIT-PKG-SIGNING-KEY-PAST
AUDIT-NATIVE-SYMBOL
AUDIT-PERM-FILESYSTEM
AUDIT-RESOURCE-SECONDARY-CLOSE
AUDIT-FLOW-AUTO-PROPAGATE
AUDIT-LINT-IDENT-SIMILARITY
```

Finding codes make `--format json` stable and allow policy files to suppress or
deny specific classes without string matching.

### 6.3 Policy File

Add a future policy file, such as `audit.policy.json`, with project-local rules:

- deny native links by default
- deny network access
- require `--locked`
- require signed packages
- deny unsigned-local exceptions outside development
- warn or deny old signing keys
- allow specific native libraries and symbols
- allow specific permissions for named packages
- deny dependency sources outside approved registries
- set warning-as-error behavior for CI

The first implementation can report policy-relevant data without enforcing a
policy file. Policy enforcement can be added after the JSON report stabilizes.

### 6.4 Transitive Capability Explanation

For every permission or native capability, show why it is present:

```text
app -> imageTool@1.2.0 -> png@0.9.1 -> zlib native link
```

This is more useful than a flat list because developers need to know which
dependency introduced a risk.

### 6.5 Public API Effect Summary

For package projects, summarize effects exposed by exported APIs:

- exported functions that may fail
- exported functions that may touch filesystem, network, process, environment,
  clock, randomness, terminal, native library, or threads
- exported resource types and close behavior
- exported `MUT` state
- native wrappers exposed as public API

This should match the package ABI metadata so importers can audit caller-visible
effects without reading source.

### 6.6 Reproducibility Report

Report whether the build is reproducible from locked inputs:

- lockfile present and current
- all packages content-addressed
- all remote registry entries backed by transparency-log proofs
- no unsigned remote packages
- no unlocked path or git dependencies
- no target-specific native dependency without a version constraint
- package writer emits deterministic metadata

This is useful for release builds and supply-chain review.

### 6.7 Target Matrix

Support a future option:

```text
mfb audit --target all
```

or:

```text
mfb audit --target macos-aarch64 --target linux-aarch64
```

The report should show permissions, native links, runtime helper requirements,
and verifier/lowering status per target. This is important because native
requirements and target capabilities can differ even when source code is the
same.

### 6.8 Source Maps And Package-Origin Mapping

When source maps or debug metadata are available, map package findings back to
the original source file and exported symbol. When they are not available, fall
back to package name, function table index, ABI name, and bytecode location.

### 6.9 Dependency Freshness And Advisory Hooks

After the base audit command is deterministic and offline-friendly, add optional
online checks:

- newer compatible versions available
- yanked, deprecated, blocked, or legal-tombstoned releases
- registry advisory records
- transparency-log consistency from the last-seen checkpoint

Online checks must be opt-in or clearly marked because ordinary audit should be
usable in hermetic CI.

### 6.10 Identifier Similarity Report

The language spec requires linting dense or security-sensitive code for
confusing identifier similarity. Audit should report:

- case-only near-collisions
- same-scope names with small edit distance in security-sensitive regions
- future Unicode normalization, case-fold, script-mixing, and confusable
  collisions if non-ASCII identifiers are ever enabled

The initial ASCII implementation can cover case-only and edit-distance checks.

### 6.11 Generated Artifact Inventory

When audit runs after or alongside a build, report generated artifacts and their
hashes:

- `.mfp` package path and content hash
- native executable path and hash
- intermediate `.ast`, `.ir`, `.bc`, `.nir`, `.nplan`, `.nobj`, and `.ncode`
  artifacts when emitted

This helps release review connect source, lockfile, package, and executable.

### 6.12 Runtime Helper Inventory

Report all runtime helper families selected by native lowering:

- helper family
- helper symbol
- source built-in call sites that require it
- backend capability that permits it
- target import or local implementation that provides it

This makes built-in behavior auditable in the same way native links are.

## 7. JSON Output Schema

`--format json` should emit one JSON object:

```json
{
  "schema": "mfb.audit.v1",
  "tool": {
    "name": "mfb",
    "version": "0.1.0"
  },
  "project": {
    "name": "example",
    "ident": "owner#example",
    "version": "0.1.0",
    "kind": "executable",
    "root": "/absolute/project/root",
    "entry": "main",
    "languageVersion": "1.0"
  },
  "summary": {
    "errors": 0,
    "warnings": 0,
    "infos": 0
  },
  "lockfile": {
    "path": "/absolute/project/root/mfb.lock",
    "present": true,
    "locked": false,
    "projectHashMatches": true,
    "logCheckpoint": "..."
  },
  "dependencies": [],
  "packages": [],
  "sourceFlow": [],
  "resources": [],
  "nativeLinks": [],
  "permissions": [],
  "runtimeHelpers": [],
  "findings": []
}
```

Every finding should have:

```json
{
  "code": "AUDIT-LOCK-STALE",
  "severity": "error",
  "category": "lockfile",
  "message": "mfb.lock projectHash does not match project.json dependency requests",
  "location": {
    "path": "/absolute/project/root/project.json",
    "line": 1,
    "column": 1
  },
  "package": "example",
  "symbol": null,
  "details": {}
}
```

The `details` object is category-specific. Consumers must ignore unknown fields
so the schema can grow without breaking older tools.

## 8. Text Output Shape

Text output should be compact, deterministic, and review-oriented:

```text
Audit: example 0.1.0 (executable)
Root: /path/to/example
Lockfile: current
Verifier: 3 packages verified

Summary:
  errors: 0
  warnings: 2
  infos: 14

Permissions:
  filesystem
    fs::openFile at src/main.mfb:12:11

Native links:
  sqlite3 >=3.45.0 from data#sqlite 3.0.0
    symbols: sqlite3_open, sqlite3_close

Warnings:
  AUDIT-PKG-SIGNING-KEY-PAST data#shape 2.3.1 verified with a past signing key
```

Do not print noisy empty sections unless a verbose option is added later.

## 9. Internal Architecture

Recommended flow:

```text
CLI args
  -> load project
  -> resolve sources and dependencies
  -> read lockfile
  -> compile/typecheck source using normal pipeline
  -> read and verify imported packages
  -> collect source flow metadata
  -> collect package/ABI/native/resource metadata
  -> collect target/runtime helper metadata when needed
  -> compare lockfile and resolver state
  -> assemble AuditReport
  -> emit text or JSON
```

Add an internal module such as:

```text
src/audit/
  mod.rs
  report.rs
  collect.rs
  text.rs
  json.rs
  policy.rs
```

The first production implementation can keep policy parsing absent if no policy
file is supported yet, but the report data model should not make policy
enforcement difficult later. `policy.rs` is omitted until a policy file is
supported.

Audit submodules live under `src/audit/` and reach the existing project loader,
front-end pipeline, package reader, and `.mfp` header helpers in `src/main.rs`
through `crate::` paths, so audit reuses build behavior without duplicating the
parser, resolver, typechecker, or package reader. To keep findings consistent
with build diagnostics, JSON is written with a hand-rolled deterministic
formatter (the project already emits AST/IR JSON this way) rather than through
`tinyjson`, whose object key ordering is not stable.

## 10. Metadata Requirements

The compiler must expose enough metadata to audit without re-deriving facts from
formatted diagnostics:

- source span for every fallible call
- callee identity and result/error type
- explicit handling versus auto-propagation
- propagation destination
- `TRAP` block classification
- cleanup region identity and resource close operation
- whether cleanup preserves secondary close-failure metadata
- permissions required by built-in and native calls
- native wrapper declarations and ABI mappings
- package dependency and ABI metadata
- verifier stage status and failures
- runtime helper families selected by native lowering

If any required metadata is unavailable, audit should emit a finding for the
missing metadata and the implementation should treat the feature as incomplete
until the compiler produces it.

## 11. Implementation Phases

### Phase 1: Command And Report Skeleton

- Add CLI dispatch for `mfb audit`.
- Parse `--format`, `--locked`, and optional `path`.
- Load `project.json` and selected source files using existing project logic.
- Emit deterministic text and JSON reports with project summary and basic
  findings.
- Add usage tests for valid and invalid CLI forms.

### Phase 2: Dependency And Lockfile Audit

- Reuse package resolution and lockfile parsing.
- Compare resolved dependencies to `mfb.lock`.
- Report package versions, hashes, signatures, ABI hashes, native metadata, and
  lock mismatches.
- Enforce `--locked`.
- Validate unsigned-local exception reporting.

### Phase 3: Package Verifier Audit

- Reuse `.mfp` reader and verifier.
- Report verifier pass/fail by package and verifier stage.
- Surface signature, identity, section, type, function, resource, ABI, and
  native verifier failures.

### Phase 4: Source Flow Audit

- Thread fallible-call, auto-propagation, and `TRAP` metadata out of typecheck
  and lowering.
- Report source locations and error destinations.
- Add coverage for fallible calls inside expressions and argument lists.

### Phase 5: Resource Cleanup Audit

- Report all lexical resource bindings and their drop-close paths.
- Validate secondary close-failure metadata for body-fails/close-fails cases.
- Report close operation identity and close-failure behavior.

### Phase 6: Native, Permission, And Runtime Helper Audit

- Report standard-package permission usage.
- Report native-link metadata from imported packages.
- Report target runtime helper requirements and backend capability coverage.
- Add target-aware reporting for current native targets.

### Phase 7: Policy And Extended Features

- Add stable audit finding codes.
- Add optional policy file support.
- Add transitive capability explanations.
- Add reproducibility reporting.
- Add optional online advisory and freshness checks.

## 12. Validation Plan

Audit implementation must have focused CLI and report tests:

- `mfb audit` succeeds on a minimal executable project.
- `mfb audit --format text` emits deterministic text.
- `mfb audit --format json` emits valid JSON with `schema = "mfb.audit.v1"`.
- invalid `--format` is a usage error.
- unknown options are usage errors.
- `--locked` fails when `mfb.lock` is missing.
- `--locked` fails when `projectHash` mismatches.
- dependency version, hash, ABI, and signature mismatches are reported.
- imported package verifier failures are reported.
- fallible calls inside expressions are reported.
- auto-propagation to function return is reported.
- auto-propagation to `TRAP` is reported.
- `TRAP` recovery behavior is classified.
- lexical resource cleanup with a fallible scope and fallible close reports the
  secondary close-failure metadata requirement.
- native link libraries, symbols, ABI mappings, and resource close functions are
  reported.
- standard package permissions are reported for filesystem, network, process,
  environment, clock, randomness, and native-library access where applicable.
- optional extended capability findings report terminal/stdio and thread use
  when implemented.

Audit reports are regression-locked with golden files. When a test directory
provides `golden/<name>.audit`, the acceptance harness runs
`mfb audit --format text <test-dir>` and `mfb audit --format json <test-dir>`,
captures both into one `<name>.audit` artifact (each preceded by its command
line and exit code), and diffs it against the golden. Because audit output uses
project-relative source paths and never prints absolute paths, the artifact is
portable across machines. Dedicated `tests/audit-*` projects exercise the
project summary, dependency/lockfile findings, source flow, resource cleanup,
permissions, and CLI usage/`--locked` error paths.

After code changes, run:

```text
scripts/test-accept.sh target/debug/mfb target/accept-actual
```

For runtime-adjacent audit features, add a validation that builds and runs a
small program when the claim depends on observable generated-program behavior.

## 13. Open Questions

1. Should `mfb audit` select a native target by default, or should target-aware
   checks require a future `--target` option?
2. Should warnings fail CI through a command-line option such as
   `--deny warnings`, through an audit policy file, or both?
3. Should package advisory and freshness checks be a separate
   `mfb audit --online` mode to keep default audit hermetic?
4. Should audit finding codes live in `specifications/error_codes.md` or in a
   dedicated audit-code specification?
5. What is the minimum debug/source-map metadata required for useful audit of
   third-party `.mfp` packages without source?

## 14. Recommended Minimum Acceptance Bar

The first complete `mfb audit` implementation is not done until it can:

- run from the CLI with the specified command shape
- produce deterministic text and JSON reports
- enforce `--locked`
- read the same project, dependency, package, and verifier data used by builds
- report every required section from `mfbasic.md` section 22
- treat missing required audit metadata as a finding rather than as success
- pass the acceptance suite

Compilation plumbing alone is not sufficient. The command must produce a real
runtime-visible audit report for representative projects.
