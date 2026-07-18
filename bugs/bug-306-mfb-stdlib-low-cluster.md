# bug-306: MFBASIC-source stdlib LOW cluster (wrong error codes, zoneless ISO, URL userinfo split, stale comment)

Last updated: 2026-07-17
Effort: small (<1h across items)
Severity: LOW
Class: Error-handling / Correctness / Docs

Status: Open
Regression Test: per-item

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
