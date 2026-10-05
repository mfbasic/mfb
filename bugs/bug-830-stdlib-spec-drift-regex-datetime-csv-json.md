# bug-830: stdlib spec drift — regex/datetime/csv/json topics contradict the code

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (spec accuracy)

Status: Open
Regression Test: none (doc-only; verify by rendering `mfb spec stdlib <topic>` and grep-resolving citations)

Found by the spec hardening review (batch B02). Each finding below was confirmed by a
compiled program or a grep against HEAD `031f64353`. The code is correct in every case;
the spec topic is stale or wrong and must be corrected to match the code.


---

## Finding: regex spec says POSIX alnum/word/xdigit/blank/graph/print never match — they all match (bug-316 fixed it)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC bug — stale doc)

Status: Open
Regression Test: tests/rt-behavior/regex/regex-posix-classes-rt (already pins the behaviour)

The regex spec still describes the pre-bug-316 defect as current behaviour. The
code implements every one of the six "special token" POSIX classes. The doc is wrong;
the code is right. Correct behaviour of the fix: the paragraph states what each of
the six classes matches (the definitions in `__regex_propTest`).

References:

- src/docs/spec/stdlib/01_regex.md §"Character Classes": "Note `alnum`, `word`,
  `xdigit`, `blank`, `graph`, and `print` map to special tokens (`posixAlnum`, etc.)
  that `__regex_propTest` does not implement, so those POSIX classes effectively
  never match a scalar."
- bugs/completed/bug-316-regex-posix-classes-never-match.md

### Failing Reproduction

```
IMPORT io
IMPORT regex

FUNC main() AS Nothing
  io::print("alnum  " & toString(regex::match("a1", "[[:alnum:]]")))
  io::print("blank  " & toString(regex::match(" ", "[[:blank:]]")))
  io::print("xdigit " & toString(regex::match("F", "[[:xdigit:]]")))
  io::print("word   " & toString(regex::match("_", "[[:word:]]")))
  io::print("graph  " & toString(regex::match("!", "[[:graph:]]")))
  io::print("print  " & toString(regex::match(" ", "[[:print:]]")))
END FUNC
```
`/tmp/hardening/mfb build /tmp/hardening/B02/min4 && /tmp/hardening/B02/min4/build/min4.out`

- Observed: all six print `TRUE`.
- Spec predicts: all six `FALSE`.

### Root Cause

`src/codegen/builtins/regex/helper_prop_test.rs:BODY` (`__regex_propTest`) handles
`posixAlnum` (L*/Nl/Nd), `posixWord` (`__regex_isWordCp`), `posixXdigit`
(`0-9A-Fa-f`), `posixBlank` (tab or Zs), `posixGraph` (not space, not C*),
`posixPrint` (graph or Zs). The spec paragraph predates bug-316's fix.

### Goal

- 01_regex.md documents the six classes' actual definitions; the "never match"
  sentence is removed.

#### Non-goals (must NOT change)

- Code behaviour.

### Blast Radius

- 01_regex.md §Character Classes — only site (grep "effectively never match").

---

## Finding: regex spec says class escapes are non-rangeable — literal escapes (`\x41`, `\n`, …) are rangeable; only shorthand/property escapes are not

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC bug — imprecise)

Status: Open
Regression Test: none needed (doc change); optional rt fixture pinning `[\x41-\x43]`

The spec's class-item sentence says "ranges `a-z` (low ≤ high required; escapes are
non-rangeable)". A reader takes that as "any `\…` endpoint makes `-` literal". In
the code only *item* escapes (`\d \D \w \W \s \S`, `\p{…}`/`\P{…}`) are
non-rangeable — as a low endpoint they end the item so a following `-` is literal,
and as a high endpoint they are a parse error — and the anchors `\b \B \A \z` are a
parse error inside a class. Every literal escape (`\xHH`, `\x{…}`, `\n`, `\t`,
escaped punctuation, …) is a full range endpoint. The code behaviour is reasonable
(it matches PCRE); the doc should say which escapes are which.

References:

- src/docs/spec/stdlib/01_regex.md §"Character Classes".

### Failing Reproduction

(in /tmp/hardening/B02/min4/src/main.mfb)
```
io::print("range of escapes [\\x41-\\x43] on B: " & toString(regex::match("B", "[\\x41-\\x43]")) & ", on '-': " & toString(regex::match("-", "[\\x41-\\x43]")))
io::print("range [\\n-z] on B: " & toString(regex::match("B", "[\\n-z]")))
```
plus (from /tmp/hardening/B02/rx runs): `[\d-z]+` on `"x-"` matches only `-`
(\d, `-`, `z` items); `[a-\d]` raises `77050003`.

- Observed: `[\x41-\x43]` on B: TRUE, on '-': FALSE; `[\n-z]` on B: TRUE.
- Spec predicts (non-rangeable escapes): `[\x41-\x43]` = {A, -, C} → B FALSE, '-' TRUE.

### Root Cause

`src/codegen/builtins/regex/helper_parse_class_endpoint.rs:__regex_parseClassEndpoint`
returns kind 1 (item, not rangeable) for shorthands and `\p`/`\P`, fails on
`\b \A \z \B`, and returns kind 0 ("literal scalar (rangeable)") for every
`__regex_parseLiteralEscape` result. `helper_parse_class.rs:__regex_parseClass`
fails when the high endpoint is kind 1 and compares endpoints with `ep.ch > ep2.ch`.

### Goal

- The spec sentence reads: literal escapes are rangeable; shorthand and property
  escapes are not (a following `-` is literal after one; one after `-` is an error);
  anchor escapes are invalid in a class.

#### Non-goals (must NOT change)

- Code.

### Blast Radius

- 01_regex.md §Character Classes only (grep "non-rangeable").

---

## Finding: three stdlib spec citations name a symbol that does not exist in the cited file (one does not exist anywhere)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC bug — dangling citations)

Status: Open
Regression Test: whatever citation sweep exists (see .ai/testing-gates.md "citation sweeps"); it evidently matches the symbol as a substring anywhere in the file, which is why these pass

Three `[[file:Symbol]]` markers in the B02 topics do not resolve to a definition:

1. src/docs/spec/stdlib/02_datetime.md line 14, "The package is injected only when
   a program `IMPORT datetime`…" cites
   `[[src/codegen/builtins/datetime/mod.rs:uses_package]]`. No `uses_package` exists
   anywhere in src/, packages/, repository/, tests/ — the only hit in that file is
   the module doc comment saying "the per-package `augmented_project`/`uses_package`
   helpers this module used to own are gone". The live mechanism is
   `src/codegen/registry/mod.rs:augment_project` /
   `RegistryPackage::is_imported_by` (as that comment says).
2. 02_datetime.md §"Zone constructors" cites
   `[[src/codegen/builtins/datetime/mod.rs:__datetime_fixedOffset1]]`; in mod.rs
   the name occurs only inside a unit test's string literal. The FUNC is defined in
   `src/codegen/builtins/datetime/func_fixed_offset.rs` (`BODY`, `FUNC
   __datetime_fixedOffset1`).
3. src/docs/spec/stdlib/03_csv.md §"Stringify algorithm" cites
   `[[src/codegen/builtins/csv/func_stringify.rs:__csv_stringifyRow]]`; func_stringify.rs
   only *calls* it. The FUNC is defined in
   `src/codegen/builtins/csv/helper_stringify_row.rs`.

References:

- AGENTS.md "Cite symbol+command"; .ai/specifications.md citation rules.

### Failing Reproduction

```
cd /Users/justinzaun/Development/mfb
grep -rn "uses_package" src packages repository tests | grep -v "::uses_package\|bridge_uses_package"
grep -n "__datetime_fixedOffset1" src/codegen/builtins/datetime/mod.rs
grep -rn "FUNC __datetime_fixedOffset1" src/
grep -n "__csv_stringifyRow" src/codegen/builtins/csv/func_stringify.rs
grep -rn "FUNC __csv_stringifyRow" src/
```
- Observed: (1) only the spec line and the mod.rs "are gone" comment; (2) one hit,
  `847: Some("__datetime_fixedOffset1")` in `#[test] fn …`; definition in
  func_fixed_offset.rs:8; (3) one call at line 82; definition in
  helper_stringify_row.rs:11.
- Expected: each citation names a file that defines the symbol.

### Root Cause

Per-member file splits (planning/migrate.md) and the registry's takeover of source
injection moved the definitions; the spec markers were not updated, and a
substring-based sweep still finds the name in the old file.

### Goal

- (1) → `[[src/codegen/registry/mod.rs:augment_project]]`; (2) →
  `[[src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset1]]`;
  (3) → `[[src/codegen/builtins/csv/helper_stringify_row.rs:__csv_stringifyRow]]`.
- Optionally tighten the sweep to require a definition, not a mention.

#### Non-goals (must NOT change)

- Code.

### Blast Radius

- Only these three in spec.md/01–04; other stdlib topics not checked by B02.

---

## Finding: datetime spec misstates fixedOffset's errors and label, and parseIso's field widths

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC bug — the man page is right, the spec is stale/imprecise)

Status: Open
Regression Test: none needed (doc change)

Three claims in src/docs/spec/stdlib/02_datetime.md disagree with measured
behaviour; in each case `mfb man datetime` already documents the real behaviour,
so the code is right and the spec is wrong.

1. §"Zone constructors": "`fixedOffset` takes either total seconds or `(hours,
   minutes)`; the magnitude must be under 24h … else `ErrInvalidArgument`
   (`77050002`)." — an `hours` whose `hours*3600` does not fit raises
   **`ErrOverflow` (`77050010`)**, not `ErrInvalidArgument` (man fixedOffset: "…
   3600 does not fit an Integer raises ErrOverflow"). Observed:
   `fixedOffset(9223372036854775807, 0)` → 77050010,
   `fixedOffset(-9223372036854775808, 0)` → 77050010, `fixedOffset(-24, 0)` → 77050002.
2. Same paragraph: "The label is rendered `±HH:MM`." — an offset that is not whole
   minutes renders **`±HH:MM:SS`**. Observed: `fixedOffset(-19845).label` =
   `-05:30:45`.
3. §"Parse grammar": `parseIso` is "a dedicated, hand-rolled scanner for
   `YYYY-MM-DD(T|t| )HH:MM:SS[.frac][offset]`" and (§Format grammar) "`parseIso` still
   accepts only four-digit non-negative years". The readers are *up to* the stated
   width: 1–4 year digits, 1–2 digits for month/day/hour/minute/second, and a bare
   `.` with no digits is accepted. Observed: `parseIso("24-02-29T00:00:00Z")` →
   `0024-02-29T00:00:00.000000000Z`; `parseIso("2024-2-29T23:59:59Z")` → accepted;
   `parseIso("2024-02-29T23:59:59.Z")` → accepted, 0 ns. (man parseIso: "The date and
   time readers are greedy up to their stated width but also accept fewer digits";
   "a bare `.` means zero nanoseconds".) "Only four-digit" should read "at most
   four-digit, non-negative".

References:

- src/docs/spec/stdlib/02_datetime.md (sections above)
- `mfb man datetime fixedOffset`, `mfb man datetime parseIso`

### Failing Reproduction

Programs: /tmp/hardening/B02/dt/src/main.mfb runs (`d12`, `d13`, `e4`, `i8`, `i12`,
`f6` lines), built with `/tmp/hardening/B02/run.sh dt`. Minimal:
```
IMPORT io
IMPORT datetime
FUNC fo(h AS Integer) AS String
  RETURN datetime::fixedOffset(h, 0).label
TRAP(err)
  RETURN "ERR " & toString(err.code)
END TRAP
END FUNC
FUNC main() AS Nothing
  io::print(fo(9223372036854775807))
  io::print(datetime::fixedOffset(-19845).label)
  io::print(datetime::toIso(datetime::parseIso("24-02-29T00:00:00Z"), 0))
END FUNC
```
- Observed: `ERR 77050010` / `-05:30:45` / `0024-02-29T00:00:00Z`
- Spec predicts: `ERR 77050002` / `±HH:MM` form / rejection.

### Root Cause

`src/codegen/builtins/datetime/func_fixed_offset.rs:__datetime_fixedOffset2`
computes `absHours * 3600 + mins * 60` with checked arithmetic before delegating to
`__datetime_fixedOffset1`'s range check; `helper_offset_label.rs:__datetime_offsetLabel`
appends `:SS`; `func_parse_iso.rs:__datetime_parseIso` uses
`helper_read_num.rs:__datetime_readNum(value, pos, maxDigits)`, which reads 1..max digits.

### Goal

- The three spec sentences match the man page.

#### Non-goals (must NOT change)

- Code.

### Blast Radius

- 02_datetime.md only.

---

## Finding: json spec See Also says string escaping "iterates graphemes" — the body (correctly) says code points

Last updated: 2026-10-05
Effort: small (<1h)
Severity: LOW
Class: Other (SPEC bug — stale line contradicts its own topic)

Status: Open
Regression Test: none needed (doc change)

The last See Also bullet of src/docs/spec/stdlib/04_json.md reads
"./mfb spec unicode strings-model — the byte/scalar/grapheme layers; parse scans
UTF-8 bytes, escape iterates graphemes". The same topic's §"String escaping" says
`__json_escapeString` iterates **code points** and explains at length why a grapheme
walk was a bug (CR LF is one grapheme cluster and was emitted raw). The See Also
line is the pre-fix wording.

### Failing Reproduction

```
IMPORT io
IMPORT json
FUNC main() AS Nothing
  io::print(json::stringify(json::JsonStr["\r\n"]))
  io::print(json::stringify(json::JsonStr["e\u{301}\"\u{301}"]))
END FUNC
```
- Observed: `"\r\n"` (escaped per code point) and `"é\"́"` (the `"` inside a
  `"`+combining-mark cluster is escaped) — code-point iteration, as §String escaping says.
- See Also line predicts grapheme iteration (CR LF emitted raw; the quote cluster
  not matched).

### Root Cause

`src/codegen/builtins/json/helper_escape_string.rs:__json_escapeString` walks
`encoding::utf32Encode` code points; the See Also bullet was not updated with that fix.

### Goal

- Bullet reads "escape iterates code points".

#### Non-goals (must NOT change)

- Code.

### Blast Radius

- 04_json.md See Also only.
