# bug-698: 79 built-in members outside `fs` omit error codes their lowering raises

Last updated: 2026-09-25
Effort: large (3h–1d)
Severity: LOW
Class: Other (documentation correctness)

Status: Open — native-body sweep DONE (2026-09-27): every package's native lowering is guarded (`declared_errors_all.rs`, 53 guards green; full suite 251/0, artifact gate 0 diffs; siblings 641/722/724/727/731 fixed and archived). bug-645's `sum` row: DONE — already landed at this HEAD (`f6f1cb561`, an ancestor; `errors: vec!["ErrFloatOverflow", "ErrFloatNaN"]` on the `Float` overload, `src/codegen/builtins/collections/func_sum.rs:115`). Phase 1's `Body::Mfb` guard: DONE (2026-10-03) — `src/codegen/builtins/tests/mfb_body_declared_errors.rs` scans every `Body::Mfb` member's source for its own `FAIL error(<code>, …)` sites plus every call it makes (a qualified `pkg::name(` call trusts that member's own declared `errors`; a bare `__name(` call to a package-private `RegistryHelper` is inlined and scanned recursively). It runs clean of classification failures (every call across the whole registry resolves) and measures 211 `Body::Mfb` members total; it currently **fails** as Phase 1's acceptance requires, listing the 136 of those 211 whose declared `errors` differ from what the scan finds (the other 75 already declare exactly what the scan finds — mostly members with no `FAIL` and no error-bearing callee at all). The 136 are recorded below. **REMAINING: Phase 2** — fill `errors` and align prose for those 136 members across 15 packages (`astrings`, `audio`, `canvas`, `color`, `csv`, `json`, `regex`, `datetime`, `encoding`, `collections`, `crypto`, `net`, `http`, `vector`). This is its own large effort: the guard's propagation is function-level (not per-overload — the same coarseness bug-645 already flagged for the native sweep), so several of the 136 findings are an overload mismatch rather than a real gap (e.g. `collections::sort`'s reported `ErrNotFound` comes only from `collections::get`'s unrelated map overload; `sort`'s own calls are the list overload, which cannot raise it) and need the same `unreachable_via_callee`-style evidence `declared_errors_all.rs` uses before touching a descriptor, not a mechanical copy of the scan's output. Owner decision 2026-09-25 stands: every function must list every error it can
produce.** That includes `ErrOutOfMemory` on every member that can raise it, and the
`Body::Mfb` members' `FAIL` codes. Land after bug-702 and bug-701, which change raise sets (both landed; see `bugs/completed/`).
Regression Test: extend `src/codegen/builtins/tests/fs_declared_errors.rs` from `fs` to every package (Phase 1); `src/codegen/builtins/tests/mfb_body_declared_errors.rs` (Phase 1, `Body::Mfb`)

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

- [ ] Fill `errors` and align prose; `scripts/man-census.sh --memory-scope`,
      `--scope`, and `scripts/man-run-examples.sh <pkg> --run` per package.
      Also covers the native-sweep `errors:` vectors; the `Body::Mfb` half is
      the 136-member list Phase 1 recorded above, in
      `src/codegen/builtins/tests/mfb_body_declared_errors.rs`'s
      `PHASE_1_BASELINE` — remove each member's entry there as its descriptor
      and prose are fixed (never edit an entry in place; see that const's own
      doc comment on re-deriving each one from the source rather than copying
      the recorded list verbatim, since the guard's function-level, not
      per-overload, propagation overstates some of them).

Acceptance: the guard passes (`PHASE_1_BASELINE` is empty and the assertion is
tightened to the plain `is_empty()` form).
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
