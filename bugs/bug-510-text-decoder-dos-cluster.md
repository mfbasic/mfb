# bug-510: text-decoder DoS cluster — regex recursion/backtracking, json/regex/csv collection amplification, punycode O(n²) (DEC-01/02/03/05)

Last updated: 2026-09-03
Effort: medium (3h–1d)
Severity: HIGH
Class: security (denial of service — CPU/memory exhaustion on untrusted text)

Status: Open (found in audit-3, Surface 5 DEC-01/02/03/05; DEC-03 lead-reproduced, DEC-01/02/05 agent-measured)

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
