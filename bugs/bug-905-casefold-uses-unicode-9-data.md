# bug-905: `strings::caseFold` (and regex `(?i)`) fold with Unicode 9.0 data — 184 scalars never fold

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/rt-behavior/strings/casefold-modern-unicode-rt (proposed)

`strings::caseFold` is documented as "Unicode full case folding" and as "the intended
basis for caseless matching". Both the compile-time fold and the runtime casefold tables
are generated from the `unicode-casefold` 0.2.0 crate, whose data is **Unicode 9.0.0**
(`~/.cargo/registry/src/*/unicode-casefold-0.2.0/src/tables.rs: UNICODE_VERSION = (9, 0, 0)`).
Every capital letter added to CaseFolding.txt after Unicode 9 is copied through unfolded:
Georgian Mtavruli U+1C90..U+1CBF (Unicode 11), Cyrillic U+1C89 (16), Glagolitic U+2C2F (14),
Latin U+A7B8..U+A7DC / U+A7F5, Vithkuqi U+10570..U+10595 (14), Garay U+10D50..U+10D65 (16),
Medefaidrin U+16E40..U+16E5F (11), Beria Erfe U+16EA0.. (17) — 184 scalars in total.
So `caseFold("Ა") <> caseFold("ა")` even though `lower("Ა") = "ა"` (lower/upper use Rust
`char` mappings, Unicode 17): caseless comparison silently fails for these scripts, and
the package's own advice ("use caseFold rather than upper or lower") gives a worse answer
than `lower`. The regex `(?i)` flag reads the same casefold data, so it fails identically
although `mfb man regex` says all Unicode-dependent regex behavior "resolves against a
single pinned Unicode version" (the gencat/Script tables are 16.0.0).

Correct behavior: `caseFold` applies the C+F mappings of a current CaseFolding.txt (at
least the version the other case tables use), so for every scalar
`caseFold(c) = CaseFolding(c)`.

References:

- `src/docs/spec/unicode/01_tables-and-algorithms.md` — "Mapping tables (NFD, upper, lower, casefold)": casefold table = `case_fold()` over every codepoint; "All three maps are full (sequence-valued) ... exactly as the compile-time crate path would produce".
- `mfb man strings caseFold` — "applying Unicode full case folding ... intended basis for caseless matching".
- `mfb man regex` / `src/codegen/builtins/regex/mod.rs` DESC — "(?i) case folding resolves against a single pinned Unicode version".
- Found by batch B10 (unicode spec hardening).

## Failing Reproduction

```
mkdir -p /tmp/cf && cd /tmp/cf && mfb init . >/dev/null
cat > src/main.mfb <<'EOF'
IMPORT io
IMPORT strings
IMPORT regex
FUNC d(s AS String) AS String
  RETURN s & ""
END FUNC
FUNC main() AS Integer
  io::print("caseFold(U+1C90)=U+10D0? " & toString(strings::caseFold(d("\u{1C90}")) = "\u{10D0}"))
  io::print("lower(U+1C90)=U+10D0? " & toString(strings::lower(d("\u{1C90}")) = "\u{10D0}"))
  io::print("caseFold(U+A7C0) = caseFold(U+A7C1)? " & toString(strings::caseFold(d("\u{A7C0}")) = strings::caseFold(d("\u{A7C1}"))))
  io::print("caseFold(U+10570) = caseFold(U+10597)? " & toString(strings::caseFold(d("\u{10570}")) = strings::caseFold(d("\u{10597}"))))
  io::print("folded caseFold(U+1C90)=U+10D0? " & toString(strings::caseFold("\u{1C90}") = "\u{10D0}"))
  io::print("regex (?i): " & toString(regex::match(d("\u{1C90}"), "(?i)^\u{10D0}$")))
  RETURN 0
END FUNC
EOF
mfb build && ./build/*.out
```

- Observed:
```
caseFold(U+1C90)=U+10D0? FALSE
lower(U+1C90)=U+10D0? TRUE
caseFold(U+A7C0) = caseFold(U+A7C1)? FALSE
caseFold(U+10570) = caseFold(U+10597)? FALSE
folded caseFold(U+1C90)=U+10D0? FALSE
regex (?i): FALSE
```
- Expected: all `TRUE` (CaseFolding.txt 11.0+: `1C90; C; 10D0`, `A7C0; C; A7C1`, `10570; C; 10597`).

Full sweep (measured): every scalar U+0001..U+10FFFF (minus surrogates, LF) was run through
the runtime `caseFold` and compared with CaseFolding.txt 17.0.0 C+F
(`/tmp/hardening/B10/ctest17.py`, driver `/tmp/hardening/B10/uni`): 184 mismatches, every one
an unfolded identity; `upper`/`lower` had 0 mismatches against UnicodeData+SpecialCasing 17.0.0.

Contrast: `ß→ss`, `İ→i̇`, `ﬃ→ffi`, Cherokee, Deseret (`U+10400→U+10428`) fold correctly
(all present in Unicode 9).

## Root Cause

`src/unicode/runtime_tables.rs:parse_tables` builds the casefold entries with
`build_mapping_tables(|value| value.to_string().case_fold()...)` and
`src/unicode/backend.rs:case_fold` folds statically with the same
`unicode_casefold::UnicodeCaseFold` trait. `Cargo.toml` pins `unicode-casefold = "0.2"`,
whose tables are Unicode 9.0.0 and which has had no newer release. The regex `(?i)` path
calls `strings::caseFold` on both sides (`src/codegen/builtins/regex/helper_char_eq.rs`:
`IF lit.fold THEN RETURN strings::caseFold(lit.ch) = strings::caseFold(__regex_chr(cp))`),
so it inherits the stale data.

## Goal

- `caseFold(c)` equals CaseFolding.txt (C+F) for every scalar, at runtime and folded.
- The casefold data version is stated in the unicode spec next to the 16.0.0 gencat pin.

### Non-goals (must NOT change)

- `upper`/`lower` (correct today against 17.0.0), NFC, and the table layout/ABI
  (`NfdEntry` 16-byte records, binary search).

## Blast Radius

- `src/unicode/runtime_tables.rs:parse_tables` (runtime casefold tables) — fixed by this bug
- `src/unicode/backend.rs:case_fold` (compile-time fold) — fixed by this bug
- regex `(?i)` matching (`helper_char_eq.rs`) — calls `strings::caseFold`, fixed by this bug
