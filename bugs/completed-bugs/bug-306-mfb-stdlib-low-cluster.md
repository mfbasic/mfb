# bug-306: MFBASIC-source stdlib LOW cluster (wrong error codes, zoneless ISO, URL userinfo split, stale comment)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Error-handling / Correctness / Docs

Status: Fixed
Regression Test: tests/rt-behavior/general/stdlib-error-code-contracts-rt (S1/S2/S3); S4 is a comment

LOW-severity correctness/error-handling residuals in the hand-written
MFBASIC-source stdlib, found during goal-06 (first review pass over the `.mfb`
packages). Distinct root causes, one document per the repo's low-cluster convention.
The HIGH/MEDIUM stdlib findings are filed separately (bug-302, 303, 304, 305, 315,
316).

References:

- Found during goal-06 review of `src/builtins/{datetime,encoding,net,collections}_package.mfb`.

## Items

### S1 — datetime parsers surface `ErrIndexOutOfRange` instead of `ErrInvalidFormat` on truncated input; `parseIso` rejects zoneless timestamps
- `src/builtins/datetime_package.mfb:752` (`__datetime_readOffset`), `:731`/`:862`
  (`__datetime_monthFromName`, AM/PM `strings::mid(value, vi, 2)`), reached from
  `__datetime_parseIso` (`:919`) and `__datetime_parseFields` (`:771`).
- These call `strings::mid(value, pos, k)` with a fixed count without first checking
  `pos + k <= len`; per the `mid` man page that raises `ErrIndexOutOfRange`
  (77050001), but the module promises structural mismatches fail `ErrInvalidFormat`
  (77050003) — so malformed/truncated input leaks the wrong, internal-looking code.
  Separately, `parseIso` requires a trailing offset, so a zoneless ISO 8601 timestamp
  (`2020-01-01T00:00:00`) cannot be parsed at all.
- Repro: `datetime::parseIso("2020-01-01T00:00:00")` → 77050001;
  `datetime::parse("Ja 2020", "MMM yyyy")` → 77050001.
- Fix: guard each `mid` with a length check that fails `ErrInvalidFormat` (or a
  bounded-slice helper like `readNum`); decide whether `parseIso` should accept a
  zoneless timestamp (defaulting to UTC/local).

### S2 — `encoding::htmlUnescape` numeric-entity overflow raises `ErrOverflow` instead of `ErrInvalidFormat`
- `src/builtins/encoding_package.mfb:799` (`__encoding_parseDecimal`) / `:819`
  (`__encoding_parseHex`), reached from `__encoding_htmlUnescape` (`:839`).
- `value = value*10 + digit` uses checked Integer arithmetic; a numeric entity with
  enough digits (`&#99999999999999999999999;`) overflows i64 before the `> 1114111`
  range check runs, so it fails `ErrOverflow` (77050010) instead of the module's
  `ErrInvalidFormat` (77050003) for a bad entity. `htmlUnescape` processes untrusted
  text, so the wrong code is observable to callers filtering on 77050003.
- Fix: cap accumulation (stop once the value exceeds 1114111) so an over-large numeric
  entity is `ErrInvalidFormat`.

### S3 — `net::toUrl` splits userinfo on the first `@` instead of the last
- `src/builtins/net_package.mfb:105-116` (`__net_toUrl`).
- `atIndex = __net_indexOf(authority, "@", 0)` uses the *first* `@`; WHATWG/RFC-3986
  parsing uses the *last* `@` as the userinfo/host boundary. For an authority with
  more than one `@`, the host retains an `@` (`http://a@b@c/` → host `"b@c"`) and
  passes host parsing unvalidated. Low impact (a bare `@` is not legal unescaped in
  userinfo).
- Fix: scan for the last `@` within the authority when splitting userinfo, or reject
  an unescaped `@` in the host component.

### S4 — `collections_package.mfb` `__collections_slice` comment claims clamping the body does not do
- `src/builtins/collections_package.mfb:87-95`.
- The comment claims it "clamps start into [0,len] and stop into [start,len]" but the
  MFBASIC body does not clamp; harmless only because `slice` is natively lowered (the
  source body is effectively dead), so a future maintainer relying on the body would
  be misled. (Also: several `net_package.mfb` comments say "grapheme index/slice"
  where `strings::find`/`mid` are scalar-indexed — internally consistent, cosmetic.)
- Fix: correct the comment (or remove the dead body).

## Goal

- Error codes match the modules' documented contracts; `parseIso` handles zoneless
  input per a decided policy; URL userinfo splits correctly; the stale comment is
  fixed.

### Non-goals (must NOT change)

- Valid-input parse results.
- The native lowering of `slice`/`take`/`drop`.

## Blast Radius

Each item is a single cited site across four `.mfb` packages; land per item.

## Fix Design / Phases

- [ ] Phase 1: tests for S1/S2/S3 error codes/behavior (S4 is a comment).
- [ ] Phase 2: apply per-item fixes.
- [ ] Phase 3: full stdlib acceptance suite green.

## Validation Plan

- Regression: error-code tests for datetime/encoding; multi-`@` URL test.
- Doc sync: datetime/encoding man pages for the error-code contract; net comment.

## Summary

Four LOW `.mfb` stdlib residuals — mostly wrong-error-code and a URL-parsing edge —
each a small localized fix. Value is honest error codes and edge correctness for the
untrusted-input packages.

## Resolution

All three behavioural items were reproduced first, and each produced exactly the
reported symptom: `77050001` for a zoneless ISO timestamp, `77050010` for a long
numeric entity, and host `b@c` for `http://a@b@c/`.

**S1 — error code only; the rejection itself is correct.** The report bundles two
claims, and they resolve differently. The wrong *code* is a real defect: fixed-count
`strings::mid` calls with no length check raise `ErrIndexOutOfRange` where the module
documents structural failures as `ErrInvalidFormat`. A new `__datetime_peek` helper
returns `""` past the end instead, so each caller reaches its own mismatch path and
reports the documented code; it is applied at all four cited sites (`readOffset`'s
head and `:`, `monthFromName`'s two candidates, the AM/PM marker).

But `parseIso` **should** reject a zoneless timestamp. The man page states the offset
is mandatory "because a conforming RFC 3339 timestamp always carries its own offset",
and contrasts it with `datetime::parse`, which does take a zone. That is a documented
design decision, not an oversight, so it is unchanged — only its diagnostic improved.

**S2** — both numeric-entity parsers stop accumulating once the value passes
`1114111`, the maximum Unicode scalar. `-1` is already their "not a valid entity"
signal, so the caller needed no change and the overflow can no longer preempt the
range check.

**S3** — a new `__net_lastIndexOf` splits userinfo at the **last** `@` per RFC 3986
§3.2, so `http://a@b@c/` now yields host `c`.

**S4** — the comment attributed clamping to `__collections_slice`'s MFBASIC body. The
clamping is real but lives in the *native* lowering; the body genuinely does not
clamp. Both the caller comment and the function itself now say so, and the function
is explicitly marked dead so it is not read as a specification.

### Verification

The fixture asserts malformed and valid input side by side, which matters because
three of these fixes add guards: a test that only checked the new error codes would
not catch a guard that started rejecting input that previously worked. Valid ISO
timestamps in three offset spellings, a valid patterned parse, valid entities, and
both ordinary URL shapes all still succeed.

55 `.ir` goldens moved. Most is line-number churn — added comment lines shift the
recorded lines of every fixture importing these packages — but not all of it: S2's
range check appears as a real new `if` op in the IR, which is the intended change.
Runtime behaviour is unchanged and was verified rather than assumed, including
`crypto-kat-valid` (known-answer tests) and `json-behavior`, whose `build.log`s are
byte-identical. The only non-`.ir` change in the tree is the new fixture itself.

Full `cargo test` green; artifact gate 0 diffs; acceptance 1009/1009.
