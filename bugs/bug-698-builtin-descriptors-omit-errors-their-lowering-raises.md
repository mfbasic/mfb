# bug-698: 79 built-in members outside `fs` omit error codes their lowering raises

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: LOW
Class: Other (documentation correctness)

Status: Open — **decided 2026-09-25 (owner): every function must list every error it can
produce.** That includes `ErrOutOfMemory` on every member that can raise it, and the
`Body::Mfb` members' `FAIL` codes. Land after bug-702 and bug-701, which change raise sets.
Regression Test: extend `src/codegen/builtins/tests/fs_declared_errors.rs` from `fs` to every package (Phase 1)

bug-694 found that every `fs` descriptor declared `errors: vec![]`, so no `fs`
man page had an Errors table. Its Phase 1 audit ran the same measurement on every
other package, and the gap is not `fs`-only: **79 `abi_function` members in 9
packages emit error codes their descriptor does not declare**, and 56 of them
declare none at all, so their pages have no Errors section. A developer reading
`mfb man process spawn` or `mfb man io print` cannot tell what to `TRAP` for.

**Correct behavior.** Every built-in member's descriptor `errors` equals the set
of codes its lowering can raise, so each page's derived Errors table lists
exactly what the call can raise — the invariant bug-694 established and now
guards for `fs`.

References:

- bug-694 (`bugs/completed/bug-694-fs-man-pages-declare-no-errors.md`): the `fs`
  fix, the guard this extends, and the audit these numbers come from.
- `.ai/man-content.md` §2: "Every raisable error and the condition that raises
  it, consistent with the auto-derived Errors table."

## Failing Reproduction

```
cargo build --release
for m in "process spawn" "io print" "thread start" "big add" "os getEnv" \
         "audio xruns" "app getTitle" "canvas fontBytes"; do
  set -- $m; printf '%s::%s ' $1 $2
  target/release/mfb man $1 $2 | grep -c '^Errors'
done
target/release/mfb man tcp listen | sed -n '/^Errors/,/^Examples/p'
```

- Observed: `0` Errors sections for all eight; `tcp::listen`'s table lists
  `ErrAddressInvalid`, `ErrNetworkFailed`, `ErrOutOfMemory` but not `ErrTimeout`,
  which its lowering raises.
- Expected: each page lists every code its lowering raises.

The measurement (bug-694 Phase 1): the `fs_declared_errors` lowering sweep —
lower each `abi_function` body on all five backends and collect every errorCode
value the emitted instructions load — run over every package, own raise sites
only (no callee propagation):

| package | abi_function members | with undeclared codes | of which declare none | could not lower |
| --- | --- | --- | --- | --- |
| app | 6 | 1 | 1 | 2 |
| audio | 9 | 1 | 1 | 0 |
| big | 29 | 22 | 12 | 0 |
| canvas | 61 | 4 | 1 | 7 |
| crypto | 7 | 0 | 0 | 4 |
| datetime | 3 | 0 | 0 | 0 |
| io | 15 | 10 | 10 | 0 |
| net | 2 | 0 | 0 | 0 |
| os | 28 | 17 | 8 | 0 |
| process | 14 | 14 | 14 | 0 |
| tcp | 11 | 1 | 0 | 0 |
| term | 26 | 0 | 0 | 26 |
| thread | 12 | 9 | 9 | 0 |
| tls | 11 | 0 | 0 | 0 |
| udp | 8 | 0 | 0 | 0 |

Per member (undeclared codes):

- `app::getTitle` ErrOutOfMemory
- `audio::xruns` ErrAudioUnavailable
- `big::` fromInteger, fromBytes, toBytes, abs, negate, add, subtract, multiply,
  sum, product, shiftLeft, shiftRight, parse, toString, toRadixString, divide,
  remainder, divMod, pow, gcd, factorial, modPow — ErrOutOfMemory
- `canvas::` publishScene, publishLayers, fontBlobUnchecked, fontBytes — ErrOutOfMemory
- `process::spawn` ErrInvalidArgument, ErrOutOfMemory, ErrSpawnFailed;
  `shell` ErrOutOfMemory, ErrSpawnFailed; `waitFor` ErrOutOfMemory,
  ErrResourceBusy, ErrResourceClosed; `receive` ErrEncoding, ErrOutOfMemory,
  ErrResourceClosed; `receiveBytes` ErrOutOfMemory, ErrResourceClosed; `pid`,
  `isRunning`, `closeInput`, `send`, `sendBytes`, `poll`, `signal`,
  `didSignal`, `detach` ErrResourceClosed
- `os::getEnv` ErrNotFound, ErrOutOfMemory; `setEnv` ErrInvalidArgument,
  ErrOutOfMemory; `hostName`, `userName` ErrOutOfMemory, ErrUnsupported;
  `getEnvOr`, `environ`, `args`, `prog`, `executablePath`, `appResourcePath`,
  `appDataPath`, `appCachePath`, `userHomePath`, `userDocumentsPath`, `name`,
  `arch`, `version` ErrOutOfMemory
- `io::print`, `write`, `printError`, `writeError`, `flush` ErrWriteFailed;
  `input`, `readLine` ErrEncoding, ErrEndOfFile, ErrInputFailed,
  ErrInvalidContext, ErrOutOfMemory, ErrWriteFailed; `readChar` the same less
  ErrWriteFailed; `readByte` ErrEndOfFile, ErrInputFailed, ErrInvalidContext;
  `pollInput` ErrInputFailed, ErrInvalidArgument
- `tcp::listen` ErrTimeout
- `thread::start` ErrInterrupted, ErrInvalidArgument, ErrOutOfMemory;
  `isRunning`, `waitFor`, `cancel` ErrResourceClosed; `poll`
  ErrInvalidArgument, ErrResourceClosed; `send`, `transfer` ErrInterrupted,
  ErrInvalidArgument, ErrResourceClosed, ErrTimeout; `receive`, `accept` the
  same plus ErrNotFound

`term` (26), `crypto` (4), `canvas` (7) and `app` (2) members did not lower in
the sweep's harness (a stub-only hook, or an import the retry loop does not
supply) and are **unmeasured**, not clean.

## Root Cause

Same as bug-694: nothing ties a descriptor's `errors` to what its lowering
raises. The only guard, `every_raise_error_site_is_declared_in_its_descriptor`
(`src/codegen/error/emission/builder_error_emission/tests/mod.rs`), scans
two-string-literal `CodeBuilder::raise_error` calls textually; it cannot see
`raise_error_into(symbol, "Err…", …)` in an `abi_function` helper or a code
raised by a shared emitter. bug-694 added the lowering sweep for `fs` only
(`fs_declared_errors::every_fs_member_declares_exactly_the_errors_its_lowering_raises`).

## Goal

- The lowering-sweep guard covers every package, and passes.
- Every page's Errors table equals its member's raise set, and each page's prose
  states the condition behind every code (`.ai/man-content.md` §2).

### Non-goals (must NOT change)

- No codegen or runtime behavior. This is descriptor data plus prose.
- Tempting wrong fix: one generic list per package. Each list comes from that
  member's own lowering.
- Tempting wrong fix: excluding a package from the sweep because it does not
  lower in the harness. Make the harness lower it (supply the hook or import).

## Blast Radius

- The 79 members above — fixed by this bug.
- `term`, `crypto`, `canvas` (7), `app` (2): unmeasured — Phase 1 makes them
  lower, then measures.
- `Body::Mfb` members (MFBASIC-source bodies) raise through `FAIL`, not a native
  lowering, so the sweep does not see them. Under the owner decision they are in
  scope: Phase 1 adds a guard that derives their raise set (their own `FAIL`s plus
  what the members they call can raise) and compares it to the descriptor.
- **Ordering.** bug-702 (a moved handle may raise `ErrResourceMoved`) and bug-701
  (`fs` codes unified across hosts) change raise sets. Land them first, or this bug's
  tables are rewritten twice.
- `fs`, `tls`, `udp`, `net`, `datetime`: clean (the sweep reports no undeclared
  code; `fs` since bug-694).

## Fix Design

1. Generalize `fs_declared_errors.rs`: `lower_body` already takes any member;
   parameterize the sweep by package and keep the callee classification
   (`RAISES_NOTHING` / `PROPAGATING`) per package, failing on an unclassified
   internal callee.
2. `ErrOutOfMemory` policy — **decided 2026-09-25 (owner): declare it wherever the
   lowering can raise it.** The table must equal the raise set; no "any call may run
   out of memory" convention replaces it. One short prose clause covers it on the
   page, as bug-694 did for `fs`. (130 descriptors declare it today and 31 of their
   pages mention it in prose, measured in bug-694.)
3. Fill each descriptor from the sweep's report and align each page's prose.

## Phases

### Phase 1: failing guard + complete measurement

- [ ] Extend the sweep to every package; make `term`/`crypto`/`canvas`/`app`
      lower in the harness.
- [ ] Add the `Body::Mfb` guard (see Blast Radius).
- [ ] Record the complete per-member list here.

Acceptance: the guard fails listing exactly the measured members.
Commit: —

### Phase 2: descriptors and prose, package by package

- [ ] Fill `errors` and align prose; `scripts/man-census.sh --memory-scope`,
      `--scope`, and `scripts/man-run-examples.sh <pkg> --run` per package.

Acceptance: the guard passes.
Commit: —

### Phase 3: validation

- [ ] Full `cargo test`; `scripts/artifact-gate.sh target/release/mfb all`
      (a description change can move goldens, `.ai/testing-gates.md`).

Commit: —

## Validation Plan

- Regression test: the generalized sweep.
- Runtime proof: none needed; the table is derived output. Render the pages.
- Doc sync: the man pages of each package.
- Full suite: `cargo test`, the artifact gate.

## Open Decisions

- ~~`ErrOutOfMemory` on every allocating member vs. a documented "any call may run
  out of memory" convention?~~ **Decided 2026-09-25 (owner): every function lists
  every error it can produce**, `ErrOutOfMemory` included.

## Summary

Documentation data across nine packages, no runtime effect. The risk is a wrong
raise set, which the lowering sweep removes by deriving each list from the code.
