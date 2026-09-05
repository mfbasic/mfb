# bug-510: text-decoder DoS cluster — regex recursion/backtracking, json/regex/csv collection amplification, punycode O(n²) (DEC-01/02/03/05)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (denial of service — CPU/memory exhaustion on untrusted text)

Status: In progress on `worktree-B-510` — see the Fix and Corrections sections at the end (found in audit-3, Surface 5 DEC-01/02/03/05; DEC-03 lead-reproduced, DEC-01/02/05 agent-measured; all four re-reproduced this session)

Regression Test: fixtures asserting a bounded time/memory for a hostile regex pattern+subject, a deeply/widely structured JSON, and a long punycode label.

## Summary

The text decoders reachable from a compiled MFBASIC program have no effective
CPU/memory budget on hostile input. They are MFBASIC source (no memory
corruption), but each amplifies cheaply:

| ID | Sev | What | Location | Measured |
|---|---|---|---|---|
| DEC-01 | HIGH | matcher charges recursion depth per *matched atom*: `^([a-z0-9-]+\.)+[a-z]{2,}$` raises at 54 labels (162 B), and caps any pattern at 300 sequential atoms | `regex/helper_match_cont.rs:20`, `helper_match_alt.rs:19`, `helper_match_node.rs:25` | 162 B fails |
| DEC-02 | HIGH | backtracking budget resets per match → `findAll`/`replace` cost is `matches × budget` | `regex/helper_search_from.rs:14`, `helper_match_results.rs:21` | 368 B → 16.9 s CPU |
| DEC-03 | HIGH | json/regex/csv materialize the whole input as a per-element collection | `json/func_parse.rs:211`, `regex/helper_make_ctx.rs:15`, `csv/func_parse.rs:72` | 1.2 MB → ~1.05 GB RSS (lead-reproduced, ~875×) |
| DEC-05 | MED | `punycodeDecode` is O(n²) in label length and enforces no 63-octet cap | `encoding/helper_puny_decode_label.rs:82-90` | 32 KB → 32.75 s |

Correctness-adjacent gaps in the same decoders, filed here for context: DEC-04
(json tokenizes by grapheme cluster → rejects CRLF-formatted JSON), DEC-06 (no
RFC 3492 overflow check, wrong error code), DEC-07 (hex escapes accept a sign:
`"\u+041"` → `"A"`), DEC-08 (uleb128 malleability + `i64::MIN` from unsigned),
DEC-09 (base64/base32 accept non-zero trailing bits), DEC-10 (embedded NUL in
decoded strings), DEC-11 (two Unicode versions in one binary), DEC-12 (regex
compile size-unbounded).

## Reproduction

DEC-03 lead-reproduced: `spikes/audit-3/DEC-03` — a 1.2 MB JSON array → ~1.05 GB
RSS. DEC-01/02/05 agent-measured (repros under `/tmp/dec-audit/`). UTF-8/16/32
decoding was verified strict (overlongs, surrogates, >U+10FFFF rejected) and JSON
depth, regex parser/matcher caps, CSV quote machine, and LEB128 shift bound are
present — the gap is uniformly the *cost* budget, not correctness of the accept set.

## Best fix

- Regex: charge recursion depth by input position consumed, not by matched atom
  (DEC-01); make the backtracking budget global to a `findAll`/`replace`, not
  per-match (DEC-02); cap pattern compile size (DEC-12). Ideally move the matcher
  to an automata/step-budgeted model.
- json/csv/regex: build results with amortized-linear collection growth and bound
  peak memory by a factor of input size (DEC-03).
- Punycode: enforce the 63-octet label cap and use the linear insertion the RFC
  describes (DEC-05); add the overflow check (DEC-06).

## Non-goals

No MFBASIC surface change; keep the accept set for well-formed inputs identical
(this is a cost fix, not a semantics change); DEC-04's grapheme-tokenization fix
must not change which *valid* JSON is accepted beyond allowing CRLF.

## Prior art

None — the `encoding`/`json`/`csv`/`regex` packages had no prior DoS audit
(searched `regex`, `backtrack`, `punycode`, `json parse`, `amplification`, `cap`).

## Corrections (this session)

- **DEC-03's mechanism is not what the table says.** The collections the decoders
  build are linear in the input and the arena reuses their growth garbage (a second
  identical 400k-element list costs 8.5 MB, not 27 MB). What made 1.2 MB cost ~1 GB
  is three codegen leaks, now filed as `bugs/bug-536-…`: values of recursive types
  (`json::Json`, `__regex_Cont`) are never freed, `RETURN <record constructor>`
  abandons its block (`__json_Node[…]`, `__regex_Result[…]` per value/step), and a
  `String` call result consumed by `&` is never freed (`__csv_decodeRange` per
  scalar). Measured: `json::parse` of the same 2 MB document twice adds 194 MB the
  second time; `regex::findAll` over 100k characters leaks ~200 MB per call;
  `csv::parse` of 1.2 MB leaks ~83 MB per call. The decoder-level share — the
  tokenisation lists — is real but secondary (json's grapheme list is ~42 bytes per
  character; regex held the subject twice at ~750 bytes per character).
- **DEC-01's suggested fix is unsound as stated.** "Charge recursion depth by input
  position consumed" would let a progressing pattern recurse without limit, and a
  native frame is consumed whether or not the step consumed a character — the
  matcher's frames are ~300 stack slots and a group repetition costs ten of them,
  so `^(ab)*$` failed on a 200-character input and the hostname pattern at 54
  labels. The fix removes the native recursion instead (below).
- **csv is left as filed.** `CsvReader.chars` is an exported `List OF Integer`
  field, so the stream path must keep scalars; its dominant cost is bug-536's
  String-temp leak and the result list's one-time growth garbage, neither of which a
  tokenizer change touches. Measured 1.2 MB of empty fields → 524 MB, of which ~83 MB
  per call is the leak.
- **DEC-05's cap is 1024 octets, not the 63 the brief named.** A 63-octet cap was
  implemented first and refused RFC 3492's own Korean sample string (74 octets),
  which decoded before the fix — the positive test caught it. The encoder also emits
  labels far past 63 (2000 `ü` → 2006 octets) and `decode(encode(x))` round-trips
  them today. The cost the cap bounds is the quadratic in-place insertion; at 1024
  octets that is at most ~8 MB of shifting, so the bound holds while every RFC
  sample, every DNS label (≤ 63) and every ordinary round trip stays decodable. The
  32 KB audit label is refused.

## Fix

| ID | Change | Where |
|---|---|---|
| DEC-01 | The matcher is an explicit-stack backtracker: `__regex_run` loops over a node-or-continuation task with a backtrack stack of `__regex_Choice` records, each holding the choice below it (a linked chain, the shape the continuations already use). Same exploration order as the CPS recursion — the 85-case corpus is byte-identical. `__REGEX_DEPTH_LIMIT` is retired; `__REGEX_PENDING_LIMIT` (500 000 pending choices) bounds the matcher's memory. The first version kept the choice points in a flat `List OF Integer` with append-only side tables for continuations, capture snapshots and `Repeat` records; it died of bug-538 (`collections::get` of a recursive-type element aliases the list's storage and the next growing `append` frees it — `(a|b)+?c` on `abc` raised "Allocation failed"), which is filed separately and re-reproduced on main. | `regex/helper_run.rs` (replaces `helper_match_{node,alt,cont,rep}.rs`, `helper_depth_limit.rs`) |
| DEC-02 | One backtracking budget per public call: `__regex_makeCtx` arms `__regex_callBudget = 2 000 000 + 100 × len(subject)`; every node visit is charged to it and to the unchanged per-search budget. | `regex/helper_make_ctx.rs`, `helper_steps.rs`, `helper_run.rs` |
| DEC-03 (regex) | The context holds the subject once, as scalars; `__regex_Lit` carries its code point; text is rebuilt from the scalar (`__regex_chr`) only where folding or a non-ASCII class needs it. | `regex/helper_make_ctx.rs`, `helper_char_eq.rs`, `helper_class_match.rs`, `helper_anchor_match.rs`, `helper_simple_match_at.rs`, `mod.rs` |
| DEC-03 (json) + DEC-04 | `json::parse` scans `strings::toBytes(value)` (tight, one byte per byte) instead of a grapheme list; string bodies accumulate bytes and decode once. CR LF between tokens is two whitespace bytes and now parses; nothing else in the accept set moves (63-document corpus). | `json/**` |
| DEC-05 / DEC-06 | `punycodeDecode` refuses an encoded label past 1024 octets before decoding, inserts in place (`collections::insert` on the `MUT` output), and applies RFC 3492 §6.4's overflow checks so an overflowing integer is `ErrInvalidFormat`, not `ErrOverflow`. | `encoding/helper_puny_decode_label.rs` |

Docs synced: `spec/stdlib/01_regex.md` (matcher section rewritten, citations
repointed, `Lit` fields), `spec/stdlib/08_encoding.md`, `punycodeDecode` man
text + `ErrInvalidFormat` in its errors list, `.ai/codegen-invariants.md` (the
depth-guard note), json `parse` man text + `spec/stdlib/04_json.md`.

Tests: `tests/rt_regex_bounds.rs`, `tests/rt_json_bounds.rs`,
`tests/rt_encoding_punycode_bounds.rs` (RED evidence in the phase-1 commit
message; positives pinned from the pre-fix compiler).
