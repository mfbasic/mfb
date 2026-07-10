# Audit Output (mfb.audit.v1)

`mfb audit` reports a project's fallible call sites, error propagation, `TRAP`
recovery, resource cleanup, host-capability use, dependency/lockfile status, and
package verifier results — without executing user code. This topic owns the
reimplementable detail of its two output formats and its analysis model; the
command surface is summarized in `./mfb spec architecture commands`.

Audit runs the same front-end pipeline a build does (manifest validation, parse,
resolve, monomorphize, re-resolve, entry validation, syntaxcheck) and then collects
a report from the parsed AST plus the installed `.mfp` packages. All collection is
offline.[[src/audit/collect/mod.rs:collect]]

## Invocation and Exit Status

```text
mfb audit [--format text|json] [--locked] [path]
```

| Option | Effect |
|---|---|
| `--format text` | Human-readable text report (default) |
| `--format json` | The `mfb.audit.v1` JSON document |
| `--format=VALUE` | Same as `--format VALUE` |
| `--locked` | Treat a missing/stale lockfile as an error, not a warning |
| `path` | Project directory (at most one; defaults to `.`) |

Unknown `-`-prefixed options, a missing/invalid `--format` value, or a second
`path` are usage errors.[[src/audit/mod.rs:parse_options]]

Both renderers escape untrusted strings. The text renderer replaces every control
character in a manifest- or `.mfp`-derived value (names, versions, paths,
messages) with `\u{XXXX}`, so a crafted package name cannot emit ESC sequences or
embedded newlines into the operator's terminal.[[src/audit/text.rs:safe]] The JSON
renderer escapes the same characters as `\u00xx`.[[src/audit/json.rs:write_string]]

| Exit | Meaning |
|---|---|
| `0` | Report produced, no error-severity findings |
| `1` | Report produced, at least one error-severity finding |
| `2` | Usage error (bad option / `--format` value) |
| `3` | Unreadable or malformed input (manifest/parse/resolve/syntaxcheck failure) |

Exit `2` is raised by the caller in `src/main.rs` when `parse_options` returns
`Err`; exit `3` is returned when any front-end stage fails; `0`/`1` are decided by
whether any finding has `severity == "error"`.[[src/audit/mod.rs:run]]

## JSON Document Shape

The JSON renderer builds an *ordered* `Json` tree and emits it with a hand-rolled
two-space-indent formatter, because `tinyjson`'s `HashMap`-backed objects would
otherwise serialize keys non-deterministically. Object members preserve insertion
order; arrays preserve their (pre-sorted) element order. Strings escape `"`, `\`,
`\n`, `\r`, `\t`, and other control chars as `\u00xx`. The document always ends
with a trailing newline.[[src/audit/json.rs:render]]

Top-level keys are emitted in this fixed order:

```text
schema           "mfb.audit.v1"
tool             { name, version }
project          { name, ident, version, kind, root, entry, languageVersion }
summary          { errors, warnings, infos }
lockfile         { path, present, locked, lockfileVersion, projectHashMatches }
dependencies     [ DependencyEntry, ... ]
packages         [ PackageEntry, ... ]
sourceFlow       [ FlowFunction, ... ]
resources        [ ResourceEntry, ... ]
nativeLinks      [ NativeLinkEntry, ... ]
nativeResources  [ NativeResourceEntry, ... ]
permissions      [ PermissionEntry, ... ]
findings         [ Finding, ... ]
```

`schema` is the constant `mfb.audit.v1`; `tool.version` is the compiler's
`CARGO_PKG_VERSION`.[[src/audit/json.rs:SCHEMA]] The prompt-level summary fields
(`schema`/`tool`) precede `project`; the rest follow the order above. `summary`
counts are derived by tallying finding severities.[[src/audit/report.rs:counts]]

### Object schemas

`Option` fields render as JSON `null` when absent (`opt_str`/`opt_int`/`opt_bool`).

**project** — `name`, `ident` (defaults to `name`), `version`, `kind`
(`executable`/`package`), `root` (forward-slash-normalized path), `entry`
(nullable entry-point function), `languageVersion` (manifest `mfb`
field).[[src/audit/collect/project.rs:project_summary]]

**lockfile** — `path` (`mfb.lock`), `present` (bool), `locked` (the `--locked`
flag), `lockfileVersion` (nullable int), `projectHashMatches` (nullable bool:
stored `projectHash` vs. recomputed `project_hash`).[[src/audit/collect/lockfile.rs:collect_lockfile]]
`lockfileVersion` is reported only when the stored JSON number is a non-negative
integer within the exactly-representable `f64` range; a fractional, negative, or
out-of-range value is malformed and reports `null` rather than a truncated or
saturated number.[[src/audit/collect/lockfile.rs:lockfile_version]]

**DependencyEntry** — `name`, `ident`, `requestedVersion`, `resolvedVersion`
(nullable), `pin` (bool), `source`, `signature` (nullable), `contentHash`
(nullable hex), `status` (`ok` / `needs-update` / `invalid` / `missing`). Sorted
by `name`.[[src/audit/collect/dependencies.rs:collect_dependencies]]

**PackageEntry** — `name`, `version`, `path` (`packages/<name>.mfp`), `verifier`
(`ok` / `failed`), `signature`, `contentHash` (hex), `exports`, `imports`,
`cleanups` (ints). Sorted by `name`.[[src/audit/collect/dependencies.rs:collect_packages]]

**FlowFunction** — `function`, `path`, `line`, `fallible` (bool), `trap`
(nullable `{ name, line, classification }`), `calls` (array of `{ callee, line,
propagation, capability? }`). Functions sorted by `(path, line, function)`; calls
sorted by `(line, callee)`.[[src/audit/json.rs:source_flow]]

**ResourceEntry** — `function`, `name`, `resourceType`, `closeOp`, `path`,
`line`, `native` (bool), `closeMayFail` (bool). Sorted by `(path, line,
name)`.[[src/audit/json.rs:resources]]

**NativeLinkEntry** — `package`, `symbol` (the native C symbol), `closeFunction`
(the `FREE` deallocator symbol, or `""` when the wrapper owns nothing), `mayFail`
(bool: true iff the wrapper has a `SUCCESS_ON` gate). One per `LINK` block
function, sorted by `symbol`.[[src/audit/collect/project.rs:collect_native_links]]

**NativeResourceEntry** — `package`, `resourceType`, `closeOp`, `closeMayFail`
(bool), `threadSendable` (bool), `exported` (bool), `kind` (always `"native"`),
`path`, `line`. One per `RESOURCE` declaration; `closeMayFail` is true iff the
close wrapper has a `SUCCESS_ON` gate. Sorted by `(path, line,
resourceType)`.[[src/audit/collect/project.rs:collect_native_resources]]

**PermissionEntry** — `capability`, `package`, `function`, `path`, `line`,
`kind` (`"standard"` for a builtin package, `"native"` for a call through a `LINK`
alias). One per capability-bearing call site, deduplicated by `(capability, path,
line, function)`, sorted by `(capability, path, line,
function)`.[[src/audit/collect/source.rs:collect_source]]

A call discloses a capability by package — `fs` → `filesystem`, `io` →
`terminal`, `thread` → `threads`, `net` → `network`, any `LINK` alias →
`native` — except for the three packages that mix pure and host-touching
builtins, which map per builtin:[[src/audit/collect/source.rs:builtin_capability]]

| Capability | Builtins |
|---|---|
| `environment` | `os::getEnv`, `os::getEnvOr`, `os::hasEnv`, `os::setEnv`, `os::unsetEnv`, `os::environ` |
| `process` | `os::args`, `os::pid`, `os::name`, `os::arch`, `os::hostName`, `os::userName`, `os::cpuCount`, `os::executablePath` |
| `randomness` | `math::rand`, `math::seed` |
| `clock` | `datetime::now`, `datetime::nowNanos`, `datetime::monotonic`, `datetime::monotonicNanos`, `datetime::localOffset`, `datetime::local`, `datetime::toLocal` |

The rest of `math` and `datetime` is arithmetic over caller-supplied values and
discloses nothing.

**Finding** — `code`, `severity` (`error`/`warning`/`info`), `category`,
`message`, `location` (nullable `{ path, line? }`), `package` (nullable). See the
catalogue below.[[src/audit/json.rs:findings]]

## Finding Catalogue

Findings carry a stable `code`, a `category`, and a `severity`. They are sorted
by `(category_rank, code, path, line, message)` for determinism.[[src/audit/collect/findings.rs:sort_findings]]

Category rank (lower sorts first):[[src/audit/report.rs:category_rank]]

| Rank | Category |
|---|---|
| 0 | `lockfile` |
| 1 | `dependency` |
| 2 | `package` |
| 3 | `sourceFlow` |
| 4 | `resource` |
| 5 | `native` |
| 6 | `permission` |
| 7 | `lint` |
| 8 | `policy` |
| (9) | any other |

| Code | Category | Severity | When emitted |
|---|---|---|---|
| `AUDIT-LOCK-MISSING` | lockfile | error | `--locked` set but `mfb.lock` absent |
| `AUDIT-LOCK-STALE` | lockfile | error if `--locked`, else warning | lockfile present but `projectHash` mismatches `project.json` packages |
| `AUDIT-DEP-MISSING` | dependency | error | declared package not installed under `packages/` |
| `AUDIT-DEP-INVALID` | dependency | error | package invalid or unreadable |
| `AUDIT-DEP-OUTDATED` | dependency | warning | installed package does not satisfy requested version (`needs-update`) |
| `AUDIT-PKG-VERIFY-FAILED` | package | error | package header/info failed to read (`verifier == "failed"`) |
| `AUDIT-PKG-UNSIGNED` | package | info | package signature is `unsigned` |
| `AUDIT-PKG-STATE-EXPORTED-MUT` | package | warning | package exports mutable global state |
| `AUDIT-RESOURCE-SECONDARY-CLOSE` | resource | info | a package cleanup records secondary close failures |
| `AUDIT-RESOURCE-CLOSE-MAY-FAIL` | resource | info | a resource is closed by lexical drop, so a close failure is unobservable without an explicit close op |
| `AUDIT-PERM-FILESYSTEM` | permission | info | project uses the filesystem capability |
| `AUDIT-PERM-NETWORK` | permission | info | network capability |
| `AUDIT-PERM-TERMINAL` | permission | info | terminal capability |
| `AUDIT-PERM-THREADS` | permission | info | threads capability |
| `AUDIT-PERM-PROCESS` | permission | info | process capability |
| `AUDIT-PERM-ENVIRONMENT` | permission | info | environment capability |
| `AUDIT-PERM-CLOCK` | permission | info | clock capability |
| `AUDIT-PERM-RANDOMNESS` | permission | info | randomness capability |
| `AUDIT-PERM-NATIVE` | permission | info | native capability |
| `AUDIT-PERM-OTHER` | permission | info | any other capability string |

Permission findings are emitted once per distinct capability (deduplicated by
capability across all sites).[[src/audit/collect/findings.rs:permission_findings]] Lockfile
findings are mutually staged: a missing-but-required lockfile returns
`AUDIT-LOCK-MISSING` and suppresses the stale check.[[src/audit/collect/findings.rs:lockfile_findings]]
The `lint`/`policy` categories have ranks reserved but emit no codes today.

## Analysis Model

### Fallibility fixpoint

A user function is *fallible* if its errors can escape to its caller. The
collector iterates to a fixpoint: starting from an empty set, it repeatedly marks
any not-yet-fallible function name whose *relevant block* can let an error escape,
stopping when a pass marks nothing new. The relevant block is the `TRAP` handler
body when a trap exists (body errors route there first), otherwise the function
body.[[src/audit/collect/source.rs:fallible_functions]]

A block "escapes" if it contains a `FAIL` or `PROPAGATE` (recursively, through
`IF`/`MATCH`/loop bodies), or if it contains a fallible call.[[src/audit/collect/source.rs:block_escapes]]

Overloads share a name, so the verdict reported for each `FlowFunction` is
computed from that *declaration's* own body. A call site, however, carries no
argument types before monomorphization and cannot be resolved to one overload:
the call-site test unions the verdicts of every overload of the name. Calling any
overload of a name with a fallible overload therefore reports the caller as
fallible — an over-approximation that never under-reports.[[src/audit/collect/source.rs:Fallibility]]

### Trap classification

A function's `TRAP` handler is classified by inspecting its body, in priority
order:[[src/audit/collect/source.rs:classify_trap]]

| Classification | Condition |
|---|---|
| `propagates` | handler contains a `PROPAGATE` |
| `fails` | else contains a `FAIL` |
| `returns value` | else contains a `RETURN` with a value |
| `recovers` | otherwise |

### Capability inference

A call site's capability is inferred from the callee's package prefix (the
segment before the first `.`):[[src/audit/collect/source.rs:builtin_capability]]

| Package | Capability |
|---|---|
| `fs` | `filesystem` |
| `io` | `terminal` |
| `thread` | `threads` |
| (other) | none |

Each call to a capability-bearing builtin becomes a `PermissionEntry` (and, when
fallible, the call's `capability` field).

### Fallible-call table

A call is fallible if its callee's package is a known-fallible builtin namespace,
or if it names a user function already in the fallible set:[[src/audit/collect/source.rs:is_fallible_call]]

```text
fallible builtin packages: fs, io, json, net, thread
otherwise:                 callee ∈ fallible-user-function set
```

### Resource producers

`LET name = <call>` is recognized as a resource binding when the callee matches
this table (scanned recursively through `IF`/`MATCH`/loop bodies). Recognized
resources have `native = false` and `closeMayFail = true`:[[src/audit/collect/source.rs:resource_producer]]

| Callee | resourceType | closeOp |
|---|---|---|
| `fs.open`, `fs.openFile`, `fs.openFileNoFollow`, `fs.createTempFile` | `File` | `fs.close` |
| `thread.start` | `Thread` | `thread.waitFor` |
| `net.connectTcp`, `net.accept` | `Socket` | `net.close` |
| `net.listenTcp` | `Listener` | `net.close` |

### Project hash

`projectHashMatches` compares the lockfile's stored `projectHash` against a
freshly computed lowercase-hex SHA-256 over the canonical, sorted serialization
of the manifest `packages[]` request tuples (`name`, `ident`, `version`, `pin`,
`source`, NUL-separated, newline-terminated, then sorted).[[src/audit/collect/mod.rs:project_hash]]

## See Also

* ./mfb spec architecture commands — the `mfb audit` command surface and other CLI commands
* ./mfb spec package verifier-rules — the package verification this audit reports as `verifier`/signature status
* ./mfb spec package container-format — the `.mfp` header and signature-type encoding behind dependency/package fields
* ./mfb spec language tooling-and-auditability — the source-level fallibility, `TRAP`, and resource model this audit analyzes
* ./mfb spec diagnostics error-codes — runtime/build error codes (distinct from these `AUDIT-*` finding codes)
