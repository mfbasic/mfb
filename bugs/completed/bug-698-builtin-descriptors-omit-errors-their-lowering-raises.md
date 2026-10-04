# bug-698: 79 built-in members outside `fs` omit error codes their lowering raises

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: LOW
Class: Other (documentation correctness)

Status: FIXED (2026-10-03). Native-body sweep DONE (2026-09-27): every package's native lowering is guarded (`declared_errors_all.rs`, 53 guards green). bug-645's `sum` row: DONE — landed at an ancestor HEAD (`f6f1cb561`). `Body::Mfb` guard (`src/codegen/builtins/tests/mfb_body_declared_errors.rs`): DONE — added, made `TRAP`-aware (spec `08_error-model.md` §8.3/§8.4: a function-level or inline `TRAP` absorbs what it catches; only the handler's own raise, or the trapped set again under `PROPAGATE`, escapes), and every one of the 136 `Body::Mfb` members it originally flagged now has its descriptor `errors` filled from its real source (one genuine body bug fixed along the way — see Resolution) or its guard-visible false-positive recorded in `UNREACHABLE_VIA_CALLEE`/`DECLARED_EXTRA`, both keyed by `(package, member, code)` so a bare name can never collide across packages that share one (`datetime::parse`/`csv::parse`/`json::parse`). The guard now asserts a plain `is_empty()`, the same bar `declared_errors_all.rs` holds every native member to — no ratchet, no baseline.
Regression Test: extend `src/codegen/builtins/tests/fs_declared_errors.rs` from `fs` to every package (Phase 1, native); `src/codegen/builtins/tests/mfb_body_declared_errors.rs` (Phase 1, `Body::Mfb`)

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

- [x] Extend the sweep to every package; make `term`/`crypto`/`canvas`/`app`
      lower in the harness. (`declared_errors_all.rs`, one guard per package)
- [x] Add the `Body::Mfb` guard (see Blast Radius).
      `src/codegen/builtins/tests/mfb_body_declared_errors.rs`: a whole-registry
      source scan (not a lowering — an `Mfb` body has no native instructions to
      lower). Per member: union its own `FAIL error(<code>)` sites with, for
      every call it makes, the called member's own declared `errors` (a
      qualified `pkg::name(` call) or a recursive inline scan of a
      package-private `RegistryHelper`'s `FUNC … END FUNC` block (a bare
      `__name(` call that is not a sibling member's rewrite symbol). A
      `CASE pkg::Variant(binding)` / `CASE __pkg_Variant(binding)`
      union-pattern match parses identically to a call and is excluded
      explicitly (`preceded_by_case`). Zero classification failures: every
      call reached by every `Body::Mfb` member in the registry resolves to
      either a registered member or a registered helper block.
- [x] Record the complete per-member list here.

Acceptance: the guard fails listing exactly the measured members. **Met**: the
raw comparison (`declared == expected`, no baseline) fails, listing the 136
members below. The COMMITTED test does not assert that raw comparison directly
— `cargo test` runs on every push and AGENTS.md forbids landing with the
suite red — it asserts against a `PHASE_1_BASELINE` constant pinned to
exactly these 136 strings (a ratchet: a mismatch outside the baseline still
fails the build, catching a new regression, but the already-known 136 do
not, so Phase 1 lands green). Phase 2 shrinks `PHASE_1_BASELINE` by removing
each entry it fixes, never edits one in place, until the guard needs no
baseline at all and can tighten to the plain `is_empty()` form
`declared_errors_all.rs`'s guards use.

Commit: (this worktree, bug-698 branch)

**The complete per-member list** (`cargo test --release --bin mfb
every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise`,
2026-10-03; format `pkg::member: undeclared [...]`, union-of-implementations
granularity — see the Status line's caveat on function- vs. overload-level
precision before treating an entry as final truth for Phase 2):

```
astrings::addAttribute: undeclared ["ErrOutOfMemory"]
astrings::removeAttribute: undeclared ["ErrOutOfMemory"]
astrings::clearAttributes: undeclared ["ErrOutOfMemory"]
astrings::getAttributes: undeclared ["ErrOutOfMemory"]
astrings::toMarkdown: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
audio::render: undeclared ["ErrFloatNaN", "ErrInvalidArgument", "ErrOutOfMemory"]
audio::play: undeclared ["ErrFloatInf", "ErrFloatNaN", "ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory", "ErrOverflow"]
canvas::present: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrOverflow", "ErrResourceClosed"]
canvas::getFrameStats: undeclared ["ErrOverflow"]
canvas::waitFrame: undeclared ["ErrOutOfMemory"]
canvas::presentLayers: undeclared ["ErrInvalidArgument", "ErrOutOfMemory", "ErrOverflow"]
canvas::loadImage: undeclared ["ErrAccessDenied", "ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrInvalidPath", "ErrPathNotFound", "ErrReadFailed", "ErrTooLarge"]
canvas::loadFont: undeclared ["ErrAccessDenied", "ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrInvalidPath", "ErrReadFailed"]
canvas::listSystemFonts: undeclared ["ErrIndexOutOfRange", "ErrInvalidFormat"]
canvas::loadSystemFont: undeclared ["ErrIndexOutOfRange", "ErrInvalidFormat"]
canvas::measureText: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
color::toPacked: undeclared ["ErrInvalidArgument"]
color::fromPacked: undeclared ["ErrInvalidArgument"]
color::fromHex: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
color::toHex: undeclared ["ErrIndexOutOfRange"]
color::toHexAlpha: undeclared ["ErrIndexOutOfRange"]
color::hsl: undeclared ["ErrOverflow"]
color::hsla: undeclared ["ErrOverflow"]
color::toHsl: undeclared ["ErrInvalidArgument", "ErrOverflow"]
color::saturate: undeclared ["ErrInvalidArgument", "ErrOverflow"]
color::desaturate: undeclared ["ErrInvalidArgument", "ErrOverflow"]
color::rotateHue: undeclared ["ErrInvalidArgument", "ErrOverflow"]
color::fromName: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
color::nameOf: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
csv::parse: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
csv::stringify: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
csv::parseStream: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
csv::readRow: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
json::get: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
json::getOr: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
json::parse: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
json::stringify: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
regex::find: undeclared ["ErrInvalidArgument", "ErrOutOfMemory"]
regex::findAll: undeclared ["ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
regex::match: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
regex::replace: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
regex::findMatch: undeclared ["ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
regex::findAllMatches: undeclared ["ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
regex::count: undeclared ["ErrNotFound", "ErrOutOfMemory"]
regex::split: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
datetime::toUtc: undeclared ["ErrInvalidArgument", "ErrOverflow"]
datetime::toLocal: undeclared ["ErrOverflow"]
datetime::format: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
datetime::parse: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory", "ErrOverflow"]
datetime::toIso: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
datetime::parseIso: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrOverflow"]
datetime::formatDuration: undeclared ["ErrInvalidArgument", "ErrOutOfMemory"]
encoding::utf16Encode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::utf16Decode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
encoding::utf32Encode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::hexEncode: undeclared ["ErrIndexOutOfRange"]
encoding::hexDecode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::base32Encode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument"]
encoding::base32Decode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::base64Encode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument"]
encoding::base64Decode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::base64UrlEncode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument"]
encoding::base64UrlDecode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::percentEncode: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
encoding::percentDecode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::htmlEscape: undeclared ["ErrIndexOutOfRange"]
encoding::htmlUnescape: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::formUrlEncode: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
encoding::formUrlDecode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::punycodeEncode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::punycodeDecode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
encoding::uleb128Encode: undeclared ["ErrInvalidArgument", "ErrOutOfMemory"]
encoding::uleb128Decode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
encoding::sleb128Encode: undeclared ["ErrInvalidArgument", "ErrOutOfMemory"]
encoding::sleb128Decode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
encoding::varintEncode: undeclared ["ErrInvalidArgument", "ErrOutOfMemory"]
encoding::varintDecode: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
encoding::codepageDecode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
encoding::codepageEncode: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::sort: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::sortBy: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::take: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::drop: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::any: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
collections::all: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
collections::groupBy: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
collections::mapValues: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
collections::flatten: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::zip: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
collections::chunks: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::window: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::distinct: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
collections::merge: undeclared ["ErrIndexOutOfRange"]
collections::partition: undeclared ["ErrIndexOutOfRange", "ErrNotFound"]
collections::toSet: undeclared ["ErrOutOfMemory"]
collections::union: undeclared ["ErrOutOfMemory"]
collections::intersection: undeclared ["ErrOutOfMemory"]
collections::difference: undeclared ["ErrOutOfMemory"]
collections::symmetricDifference: undeclared ["ErrOutOfMemory"]
crypto::uuid7: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
crypto::ulid: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound"]
net::toUrl: undeclared ["ErrIndexOutOfRange", "ErrNotFound", "ErrOutOfMemory"]
net::percentDecode: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
net::parseQuery: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrNotFound", "ErrOutOfMemory"]
http::read: undeclared ["ErrAddressNotFound", "ErrConnectionClosed", "ErrInvalidArgument", "ErrNetworkFailed", "ErrOutOfMemory", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::write: undeclared ["ErrAddressNotFound", "ErrConnectionClosed", "ErrInvalidArgument", "ErrNetworkFailed", "ErrOutOfMemory", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::startRead: undeclared ["ErrAddressNotFound", "ErrConnectionClosed", "ErrInvalidArgument", "ErrNetworkFailed", "ErrOutOfMemory", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::ready: undeclared ["ErrInvalidArgument", "ErrNetworkFailed", "ErrOutOfMemory", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::pump: undeclared ["ErrConnectionClosed", "ErrInvalidArgument", "ErrNetworkFailed", "ErrOutOfMemory", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::done: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrNotFound", "ErrOutOfMemory"]
http::finish: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrNotFound", "ErrOutOfMemory"]
http::server: undeclared ["ErrAddressInvalid", "ErrNetworkFailed", "ErrOutOfMemory"]
http::serverSSL: undeclared ["ErrAddressInvalid", "ErrNetworkFailed", "ErrOutOfMemory", "ErrTlsFailed"]
http::handleRequest: undeclared ["ErrConnectionClosed", "ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidFormat", "ErrNetworkFailed", "ErrNotFound", "ErrOutOfMemory", "ErrOverflow", "ErrResourceClosed", "ErrResourceMoved", "ErrTimeout", "ErrTlsFailed"]
http::route: undeclared ["ErrIndexOutOfRange", "ErrInvalidArgument", "ErrNotFound", "ErrOutOfMemory"]
http::responseDefault: undeclared ["ErrOutOfMemory"]
http::ok: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
http::status: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
http::json: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
http::withHeader: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory"]
http::bytes: undeclared ["ErrOutOfMemory"]
http::respondFile: undeclared ["ErrIndexOutOfRange", "ErrOutOfMemory", "ErrReadFailed", "ErrResourceClosed", "ErrResourceMoved"]
http::respondPath: undeclared ["ErrAccessDenied", "ErrIndexOutOfRange", "ErrInvalidArgument", "ErrInvalidPath", "ErrNotFound", "ErrOutOfMemory", "ErrPathNotFound", "ErrReadFailed", "ErrWriteFailed"]
vector::length: undeclared ["ErrFloatDomain", "ErrInvalidArgument"]
vector::normalize: undeclared ["ErrFloatDomain", "ErrOverflow"]
vector::distance: undeclared ["ErrFloatDomain", "ErrInvalidArgument"]
vector::project: undeclared ["ErrOverflow"]
vector::angle: undeclared ["ErrFloatDomain", "ErrOverflow"]
vector::lerp: undeclared ["ErrInvalidArgument", "ErrOverflow"]
vector::lerp_unclamped: undeclared ["ErrOverflow"]
vector::slerp: undeclared ["ErrFloatDomain", "ErrFloatNaN", "ErrOverflow"]
vector::clamp_length: undeclared ["ErrFloatDomain", "ErrOverflow"]
vector::min: undeclared ["ErrInvalidArgument"]
vector::max: undeclared ["ErrInvalidArgument"]
vector::abs: undeclared ["ErrOverflow"]
vector::rotate_2d: undeclared ["ErrFloatNaN", "ErrOverflow"]
```

Every entry above is "declared but never raised []" — every `Body::Mfb` member in
the registry currently declares `errors: vec![]` (native `Mfb` bodies were entirely
outside bug-694/bug-698's native-lowering sweep until this guard), so Phase 2 starts
from zero on every one of these 136 members, not a partial list.

### Phase 2: descriptors and prose, package by package

- [x] Fill `errors` and align prose. Done per package, with the per-member
      reachability evidence in each commit (see Resolution). `PHASE_1_BASELINE`
      no longer exists — every entry was either resolved (true `errors` filled)
      or reclassified as `UNREACHABLE_VIA_CALLEE`/`DECLARED_EXTRA`, both now
      keyed by `(package, member, code)`.
      `scripts/man-census.sh --memory-scope`/`--scope` (whole surface, no
      package args): 0 unclassified hits on both. `man-run-examples.sh` was
      NOT run (no prose example text changed — only `DESC` additions
      describing new error conditions, per the house `ErrOutOfMemory` phrase
      and similar one-sentence additions for each other code).

Acceptance: the guard passes — **met**, `cargo test --release --bin mfb
every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise` is
`ok` with the plain `is_empty()` assertion (no baseline).
Commit: see the per-package commits on `bug-staging-7`/this worktree
(`astrings`, `audio`, `canvas`, `color`, `crypto`, `csv`, `datetime`,
`encoding`, `collections`, `json`, `net`, `regex`, `vector`, `http`, plus the
guard's own `TRAP`-modeling commit).

### Phase 3: validation

- [ ] Full `cargo test`; `scripts/artifact-gate.sh target/release/mfb all`
      (a description change can move goldens, `.ai/testing-gates.md`). **Not
      run here** — out of this task's scope rules (the coordinator runs the
      full gate once for the whole batch across all parallel bug worktrees).
      Expected to move: `mfb man --all`/registry-dump goldens and any
      `.audit`/help-output golden for every touched member (136 members
      across 15 packages gained/changed declared `errors`, all code-only
      descriptor + prose changes, no codegen/runtime change).

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

## Resolution

Phase 1's `Body::Mfb` guard (`src/codegen/builtins/tests/mfb_body_declared_errors.rs`)
measured 211 `Body::Mfb` members; 136 had a descriptor `errors` that did not match
what their source could raise. All 136 are now fixed, package by package (one
commit per package on this worktree), plus the guard itself gained two
capabilities the original design did not anticipate:

1. **`TRAP` modeling.** The guard originally treated a trapped call's codes as
   unconditionally reachable. Two independent findings during Phase 2 — the
   coordinator's own read of `net::percentDecode`/`net::parseQuery`, and the
   `canvas` agent's read of `loadImage`/`loadFont` — showed this double-counted
   through every function-level and inline `TRAP`/`RECOVER`/`PROPAGATE` in the
   registry. Fixed per spec `08_error-model.md` §8.3/§8.4: a function-level
   `TRAP` absorbs every error the normal body raises, escaping only what the
   handler itself raises (or the trapped set again, if the handler
   `PROPAGATE`s); an inline `TRAP` does the same for just its one expression,
   with `RECOVER` as the no-propagate case. A second bug surfaced while testing
   the fix — the first version required the `TRAP` line at column 0, but a
   function-level `TRAP` is indented like any other statement — found and fixed
   by direct instrumentation (`debug_percent_decode_and_parse_iso`, removed
   before the final commit).
2. **`(package, member, code)` keys.** `UNREACHABLE_VIA_CALLEE` was originally
   keyed by bare member name; the coordinator flagged that this collides across
   packages sharing a name (`datetime::parse`/`csv::parse`/`json::parse`), so a
   proof for one could silently hide a real gap on another. Fixed, and extended
   with a mirror-image table, `DECLARED_EXTRA`, for a code the guard's text scan
   cannot see as reachable (a dynamic `FAIL error(<field>, ...)` re-raise, or an
   implicit checked-arithmetic operator raise) but that really is.

**One real body bug found and fixed, not just declared:**
`src/codegen/builtins/datetime/func_parse_iso.rs`'s `__datetime_parseIso` read
the date/time separator with an unguarded `strings::mid(value, pos, 1)`, so a
short input (e.g. `"2026-06-26"`) leaked `ErrIndexOutOfRange` (`77050001`)
instead of the `ErrInvalidFormat` (`77050003`) its own `DESC` promises for every
malformed-input case. Fixed by adding the same `IF pos >= n THEN FAIL
error(77050003, ...) END IF` guard the sibling `__datetime_expect` helper
already uses. Repro before/after: `parseIso("2026-06-26")` raised `77050001`
before this fix, `77050003` after.

**Two cross-cutting design questions found, deliberately left open** (per the
coordinator's instruction not to silently enshrine a leaked error as "the
contract" when a body could instead be tightened, nor to silently paper over a
real gap in an unrelated package):

- `http`/`csv`/`datetime` each construct a domain value (`CsvReader`,
  `datetime::Date`/`DateTime`) as a plain `EXPORT`ed record, which user code can
  build directly with an inconsistent/out-of-range field, bypassing the
  package's own validating constructor (`csv::parseStream`, `datetime::date`).
  `csv::readRow`'s `ErrIndexOutOfRange` (a hand-built `CsvReader` with a
  mismatched `count`/`index`) and `datetime::format`'s `ErrIndexOutOfRange` (a
  hand-built `DateTime` with `month` outside `1 .. 12`) are both now declared as
  real, matching measured behavior (confirmed with compiled probes) — but
  whether these types should instead validate defensively (raise
  `ErrInvalidArgument` up front) is a wider policy question across every
  record-represented "valid by convention" type in the registry, not a
  bug-698-sized fix. Left to the owner.
- `general::toString`'s descriptor declares `errors: vec![]` ("toString never
  fails"), but its `List OF Byte` overload decodes UTF-8 and genuinely raises
  `ErrEncoding` on an ill-formed sequence (confirmed with compiled probes
  through `encoding::htmlUnescape`/`punycodeDecode`). Fixing `toString`'s own
  descriptor ripples into every other caller across the whole registry, native
  and `Mfb` alike — out of scope here; the two measured-but-guard-invisible
  `ErrEncoding` declarations are carried via `DECLARED_EXTRA` instead.
  `general::toInt` has the same shape (declares `errors: vec![]`, its own `DESC`
  says `ErrInvalidFormat`/`ErrOverflow`) — found, not touched.

**Verification:**

```
CARGO_TARGET_DIR=/tmp/b698-target cargo test --release --bin mfb declared_errors
# 23 passed; 0 failed (includes every_body_mfb_member_declares_exactly_the_errors_its_source_can_raise)

MFB=/tmp/b698-target/release/mfb ./scripts/man-census.sh --memory-scope
# unclassified memory-vocabulary hits: 0

MFB=/tmp/b698-target/release/mfb ./scripts/man-census.sh --scope
# internals-vocabulary hits: 1 (pre-existing, unrelated: errorcode/mod.rs's
# ErrStackOverflow constant description, last touched by bug-789 -- confirmed
# via git log, not part of this change, left to its own bug)
```

Not run here (coordinator's full-batch gate): `cargo test` (whole suite),
`scripts/artifact-gate.sh`. Expected to move: `mfb man`/registry-dump goldens
and any `.audit`/help-output golden for the 136 touched members' pages.

## Summary

Documentation data across nine packages, no runtime effect. The risk is a wrong
raise set, which the lowering sweep removes by deriving each list from the code.
