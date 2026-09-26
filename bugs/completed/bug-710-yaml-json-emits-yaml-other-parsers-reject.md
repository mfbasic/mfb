# bug-710: `examples/yaml-json`'s emitter writes YAML other parsers reject, and doubles the trailing newline

Last updated: 2026-09-25
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: FIXED (378194435)
Regression Test: `examples/yaml-json/tests/emitter-conformance.sh` (new — see Validation Plan)

`examples/yaml-json`'s `to-yaml` command writes bytes for C1 control characters
(U+0080–U+009F) and for U+0085 (NEL) / U+2028 (LINE SEPARATOR) directly into
plain and block YAML scalars, unescaped and unquoted. YAML's own printable
character set excludes the C1 range from appearing literally in a document at
all (in either 1.1 or 1.2), and NEL/LINE SEPARATOR are line-break characters
under YAML 1.1 — exactly the kind of reader disagreement the emitter's stated
design goal ("whatever it is not certain a plain scalar would round-trip gets
quoted", `examples/yaml-json/src/emit.mfb:10-12`) says it protects against.
Today it does not: PyYAML rejects the C1 case outright and mis-parses the
NEL/LINE-SEPARATOR case as a second (malformed) mapping key. Separately,
`to-yaml`'s output always ends with two newlines instead of one, because both
`toYaml` and the `io::print` call that writes its result append their own
trailing `"\n"`.

**Correct behavior.** (a) Every codepoint the emitter cannot certify is safe in
a plain or block scalar — including the C1 controls and the YAML-1.1 line-break
characters NEL and LINE SEPARATOR — is escaped in a double-quoted scalar, the
same way `\x00`–`\x1F` and `\x7F` already are. Quoting alone is not sufficient
for the C1 case: `quoteScalar` delegates to `json::stringify`, which does not
escape C1 bytes, so the character reaches the output raw regardless of scalar
style. (b) `to-yaml` output ends with exactly one trailing newline.

References:

- `examples/yaml-json/README.md:70-80` — the emitter's stated design rule
  ("anything it is not certain a plain scalar would round-trip gets quoted")
  and the existing YAML-1.1-compatibility precedent (quoting `yes`/`no`/`on`/
  `off`/`y`/`n` even though the package's own YAML-1.2 reader would not
  misread them).
- YAML 1.1 §5.2 / YAML 1.2 §5.2 ("Character Set") — both restrict the
  printable character set; the 1.1 spec additionally lists `x85` (NEL) and
  `x2028` (LS) as line-break characters, which 1.2 does not.

## Failing Reproduction

Built in a scratch copy per the project's own README (network use limited to
what was already installed — PyYAML 6.0.3 was present under the invoking
user's `python3`, so no install was needed):

```sh
cp -r examples/yaml-json /tmp/yaml-json-repro
cp -r packages/yaml /tmp/yaml-pkg-build
mfb build /tmp/yaml-pkg-build
cp /tmp/yaml-pkg-build/yaml.mfp /tmp/yaml-json-repro/packages/yaml.mfp
mfb build /tmp/yaml-json-repro
```

### (a) C1 control character U+0081

```sh
python3 -c "import json; json.dump({'value':'abc\u0081def'}, open('/tmp/c1-control.json','w'))"
/tmp/yaml-json-repro/build/yamljson.out to-yaml /tmp/c1-control.json > /tmp/c1-control.yaml
xxd /tmp/c1-control.yaml
python3 -c 'import yaml,sys; print(yaml.safe_load(sys.stdin))' < /tmp/c1-control.yaml
```

- Observed bytes (`xxd`): `7661 6c75 653a 2061 6263 c281 6465 660a` `0a`
  — `value: abc` followed by the raw UTF-8 bytes `c2 81` (U+0081) and `def`,
  unquoted, unescaped.
- Observed PyYAML output (verbatim):
  ```
  yaml.reader.ReaderError: unacceptable character #x0081: special characters are not allowed
    in "<stdin>", position 10
  ```
- Expected: `value: "abc\x81def"` (or equivalent `\x81` escape), which PyYAML
  accepts (`yaml.safe_dump` itself emits exactly this form for the same
  input: `'value: "abc\\x81def"\n'`).

Quoting alone does not fix this — `quoteScalar` (`emit.mfb:74-77`) delegates to
`json::stringify`, which does not escape C1 bytes. Forcing the quoted path by
adding `": "` to the string reproduces the same PyYAML failure with the
character now sitting inside double quotes:

```sh
python3 -c "import json; json.dump({'value':'abc: \u0081def'}, open('/tmp/c1-forced-quote.json','w'))"
/tmp/yaml-json-repro/build/yamljson.out to-yaml /tmp/c1-forced-quote.json > /tmp/c1-forced-quote.yaml
xxd /tmp/c1-forced-quote.yaml
# 7661 6c75 653a 2022 6162 633a 20c2 8164  value: "abc: ..d
# 6566 220a 0a                             ef"..
python3 -c 'import yaml,sys; print(yaml.safe_load(sys.stdin))' < /tmp/c1-forced-quote.yaml
# yaml.reader.ReaderError: unacceptable character #x0081: special characters are not allowed
#   in "<stdin>", position 13
```

### (a) NEL U+0085 and LINE SEPARATOR U+2028

```sh
python3 -c "import json; json.dump({'value':'abc\u0085def'}, open('/tmp/nel.json','w'))"
/tmp/yaml-json-repro/build/yamljson.out to-yaml /tmp/nel.json > /tmp/nel.yaml
xxd /tmp/nel.yaml
python3 -c 'import yaml,sys; print(yaml.safe_load(sys.stdin))' < /tmp/nel.yaml
```

- Observed bytes: `7661 6c75 653a 2061 6263 c285 6465 660a` `0a` —
  `value: abc` + raw `c2 85` (U+0085 NEL) + `def`, unquoted.
- Observed PyYAML output (verbatim):
  ```
  yaml.scanner.ScannerError: while scanning a simple key
    in "<stdin>", line 2, column 1
  could not find expected ':'
    in "<stdin>", line 4, column 1
  ```
  (PyYAML follows YAML 1.1's line-break rule for NEL, so it reads `abc`,
  then a line break, then `def` as the start of a second block-mapping key,
  which is missing its `:`.)
- U+2028 (LINE SEPARATOR) is identical in shape: `python3 -c "import json;
  json.dump({'value':'abc def'}, open('/tmp/lsep.json','w'))"` produces
  bytes `... 6263 e280 a864 6566 ...` (raw `e2 80 a8`, unquoted) and PyYAML
  raises the same `ScannerError: while scanning a simple key ... could not
  find expected ':'`.
- Expected: both characters escaped in a double-quoted scalar (e.g.
  `"abc\x85def"` / `"abc def"`), which does not confuse any reader's
  line-counting.
- The same failure reaches block scalars, not just plain ones: a multi-line
  string with an embedded NEL (`'line one\nline\u0085two\nline three\n'`)
  emits as a literal block (`value: |`) with the NEL raw on the second body
  line, and PyYAML fails with the identical `ScannerError: ... could not find
  expected ':'` two lines later than expected — the block-scalar safety check
  has the same gap as the plain-scalar one.

Contrast case that works correctly today: the ASCII C0 controls and DEL
(`\x00`–`\x1F`, `\x7F`) are already rejected by `isPlainSafe` (`emit.mfb:65`)
and `isBlockSafe` (`emit.mfb:88`), forcing them into a quoted scalar where
`json::stringify` does escape them (JSON requires escaping `\x00`–`\x1F`).
That path round-trips correctly; only the C1 range and NEL/LINE SEPARATOR are
missed.

### (b) doubled trailing newline

```sh
/tmp/yaml-json-repro/build/yamljson.out to-yaml examples/yaml-json/samples/package.json > /tmp/pkg-to-yaml.out
od -c /tmp/pkg-to-yaml.out | tail -5
```

- Observed (verbatim, last line of `od -c`):
  ```
  y   :       {   }  \n   n   o   n   e   :       [   ]  \n  \n
  ```
  — the file ends `...[]\n\n`: content, one newline that terminates the
  `none: []` line, and a second, empty line before EOF.
- Expected (a single trailing newline, shown for contrast with `printf
  'value: abc\n'`): `...[]\n` with no second `\n`.
- Every one of the fixtures captured above for (a) shows the same doubled
  `\n\n` at EOF (e.g. `/tmp/c1-control.yaml`: `...6465 660a` `0a`), confirming
  it is unconditional, not input-dependent.

## Root Cause

**(a) Missing codepoints in the "must quote" checks.**
`examples/yaml-json/src/emit.mfb:isPlainSafe` (lines 54-68) and
`emit.mfb:isBlockSafe` (lines 84-102) each loop over the string's scalars and
reject only `toInt(scalar) < 32 OR toInt(scalar) = 127` — the ASCII C0
controls and DEL. Neither function's per-character loop (`isPlainSafe:64-66`,
`isBlockSafe:86-89`) has an arm for `0x80`–`0x9F` (C1 controls), `0x85` (NEL),
or `0x2028` (LINE SEPARATOR), so strings containing them are classified as
plain-safe (or block-safe) and pass straight through `scalarText`
(`emit.mfb:122-137`) / `blockScalar` (`emit.mfb:107-119`) unescaped. Even when
a string is otherwise forced into the quoted path, `quoteScalar`
(`emit.mfb:74-77`) hands the raw string to `json::stringify`, whose escaping
follows JSON's rules (control chars below `0x20`, `"`, `\`) — JSON does not
require escaping `0x80`–`0x9F`, `0x85`, or `0x2028`, so `json::stringify`
passes them through as raw UTF-8 too. The C1 range is unreadable to a
conforming YAML 1.1 or 1.2 reader regardless of quoting (confirmed above:
PyYAML rejects it even inside a forced double-quoted scalar), and NEL/LINE
SEPARATOR are line-break characters under YAML 1.1, so an unquoted occurrence
silently splits the document exactly the way the module's own comment
(`emit.mfb:27-32`) says it takes pains to avoid for the boolean-like words.

**(b) Two newline-appenders stacked.** `toYaml`
(`examples/yaml-json/src/emit.mfb:215-223`) always returns a string ending in
`"\n"` (`blockScalar(...) & "\n"` at line 218, `emitBlock(...) & "\n"` at
lines 219 and 221). Its only caller, `jsonToYaml`
(`examples/yaml-json/src/main.mfb:53-55`), returns that string unchanged to
`main` (line 106: `io::print(jsonToYaml(text))`), and `io::print` itself
appends a trailing LF after writing its argument
(`src/codegen/builtins/io/func_print.rs:19-25`, `lower_print` →
`lower_write_family(..., true, "io.print")`; documented on the same
`RegistryFunction` as "writes `value` to standard output and then appends a
single line feed"). The two appended newlines are why every `to-yaml`
invocation's output ends `\n\n` rather than `\n`, independent of the input
value (confirmed above across three different JSON inputs).

## Goal

- No character in a YAML document emitted by `to-yaml` falls outside the set
  every conforming reader (YAML 1.1 or 1.2) accepts unescaped: C0/C1 controls,
  DEL, NEL (U+0085) and LINE SEPARATOR (U+2028) are all escaped in a
  double-quoted scalar when present.
- `quoteScalar` (or its caller) actually escapes those codepoints — routing a
  string into the quoted path is not sufficient by itself, since
  `json::stringify` does not escape them today.
- `to-yaml` output ends with exactly one trailing newline.

### Non-goals (must NOT change)

- The existing YAML-1.1-boolean quoting behavior (`isReservedWord`,
  `emit.mfb:33-45`) — already correct, not touched by this bug.
- The block-vs-plain-vs-quoted selection logic's general shape
  (`isBlockSafe`/`isPlainSafe`/`isInline`) beyond adding the missing
  codepoint checks — no redesign of the emitter's quoting strategy.
- `to-json` (`yamlToJson`, `main.mfb:40-50`) and the `yaml` package's reader —
  neither is implicated; this bug is entirely in the example's emitter and
  its one caller.
- `json::stringify`'s own escaping rules (used elsewhere for JSON output) —
  it is JSON-conformant as is; the fix must not weaken or change JSON output,
  only how the YAML emitter uses/wraps it.

## Blast Radius

```
$ grep -rn "toYaml\|emit.mfb" examples/ packages/
examples/yaml-json/README.md          (prose reference only)
examples/yaml-json/src/emit.mfb:215   (PUBLIC FUNC toYaml — definition)
examples/yaml-json/src/main.mfb:54    (RETURN toYaml(json::parse(text)) — the only call site)
```

- `examples/yaml-json/src/main.mfb:54` (`jsonToYaml`) — the only consumer of
  `toYaml`; fixed by this bug via the `io::print` double-newline fix and by
  fixing the escaping it depends on.
- `examples/yaml-json/src/main.mfb:106` (`io::print(jsonToYaml(text))`) —
  the other half of the doubled-newline bug; fixed here.
- No other example or package emits YAML or depends on `emit.mfb`'s escaping
  logic — the grep above is exhaustive within `examples/` and `packages/`.
  The `yaml` package (`packages/yaml`) only reads YAML; it has no emitter.

## Fix Design

Two independent, small fixes:

1. **Escaping.** Add `0x80`–`0x9F`, `0x85`, and `0x2028` (and, for
   completeness under the same reasoning, `0x2029` PARAGRAPH SEPARATOR, which
   YAML 1.1 also treats as a line break) to the per-character rejection loops
   in `isPlainSafe` (`emit.mfb:64-66`) and `isBlockSafe` (`emit.mfb:86-89`),
   forcing any string containing one into the quoted path. Separately, fix
   `quoteScalar` (`emit.mfb:74-77`) so the quoted output actually escapes
   those codepoints instead of passing them through raw — `json::stringify`
   must not be changed (it is JSON-conformant), so this means either a
   post-process step in `quoteScalar` that additionally backslash-escapes
   `0x80`–`0x9F`/`0x85`/`0x2028`/`0x2029` in the JSON-quoted result, or
   building the double-quoted form directly for the codepoints this module
   cares about rather than delegating entirely to `json::stringify`. Rejected
   alternative: rejecting these codepoints outright (turning them into a
   `to-yaml` error) — the JSON input is fully entitled to hold arbitrary
   Unicode text, so the emitter must represent it, not refuse it, matching
   the existing precedent for C0 controls (quoted, not rejected).
2. **Trailing newline.** Remove exactly one of the two newline-appenders.
   Recommended: drop the trailing `& "\n"` from `toYaml`
   (`emit.mfb:215-223`, all three return sites) and let `io::print`
   (`main.mfb:106`) supply the single newline, since `io::print`'s newline is
   the one every other command-line output in this project already relies on
   (`yamlToJson`'s `json::stringify` result has no trailing newline of its
   own either — `main.mfb:104`, `io::print(yamlToJson(...))` — so `toYaml`
   keeping its own newline is the outlier, not the norm). Rejected
   alternative: switch `main.mfb:106` to a hypothetical "print without
   newline" call instead — no such builtin is otherwise needed here, and it
   would make `to-yaml` inconsistent with `to-json`'s already-correct,
   newline-free-string-plus-`io::print` pattern.

No change to `json::stringify`, the `yaml` package, or any generated/expected
output outside `examples/yaml-json` is expected.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `examples/yaml-json/tests/emitter-conformance.sh` (or equivalent)
      per the Regression Test below, encoding the C1/NEL/LINE-SEPARATOR and
      doubled-newline reproductions above as an executable check that fails
      against current `toYaml`/`main.mfb`.
- [x] Confirm the blast-radius grep above stays empty (no other emit.mfb
      consumer appeared since).

Acceptance: the new script fails for the documented reason on current code.
Commit: 953f97635 — confirmed RED: all five PyYAML-acceptance fixtures raise
a `ReaderError`/`ScannerError`, and the trailing-newline check finds two
newlines. `git diff 0cd2db799 -- examples/yaml-json packages/yaml` (i.e. the
blast-radius grep re-run) still finds no second `emit.mfb`/`toYaml` consumer.

### Phase 2 — the fix

- [x] Add the missing codepoints to `isPlainSafe` (`emit.mfb:64-66`) and
      `isBlockSafe` (`emit.mfb:86-89`).
- [x] Make `quoteScalar` (`emit.mfb:74-77`) actually escape
      `0x80`-`0x9F`/`0x85`/`0x2028`/`0x2029` in its output.
- [x] Remove the extra `& "\n"` from `toYaml`'s three return sites
      (`emit.mfb:218`, `219`, `221`).

Acceptance: Phase 1's script passes; `examples/yaml-json/smoke.sh` still
passes unchanged; the samples in `examples/yaml-json/samples/` still round-trip.
Commit: 378194435 — added a shared `isUnsafeControlOrLineBreak` predicate
(the existing C0/DEL check plus C1/NEL/LS/PS) used by both safety checks, and
a `escapeUnsafeForYaml` post-process over `quoteScalar`'s `json::stringify`
output (also added `0x2029` PARAGRAPH SEPARATOR per the Open Decision
recommendation below). `\uXXXX` was used for the escape (not `\xXX`, since
this language's own string-literal lexer has no `\xNN` form and the two are
equally valid inside a YAML double-quoted scalar).

### Phase 3 — regenerate expected outputs + full validation

- [x] No goldens exist for this example; re-run the exact reproduction
      commands above and confirm PyYAML now accepts every case, and `od -c`
      shows exactly one trailing `\n`.
- [x] Run `examples/yaml-json/smoke.sh`.

Acceptance: PyYAML `safe_load` succeeds on every fixture from the
Reproduction section; `to-yaml` output ends in a single `\n`; smoke.sh green.
Commit: 378194435 — `examples/yaml-json/tests/emitter-conformance.sh` and
`examples/yaml-json/smoke.sh` both green against the rebuilt `yamljson.out`;
all five reproduction commands from **Failing Reproduction** re-run by hand,
PyYAML now accepts every one, and `od -c`/`xxd` on `to-yaml` output shows
exactly one trailing `\n`.

## Validation Plan

- Regression test(s): `examples/yaml-json/tests/emitter-conformance.sh` (new)
  — builds the example (per its README), feeds it the three fixtures above
  (C1 control, NEL, LINE SEPARATOR, plus the block-scalar NEL case) via
  `to-yaml`, pipes each result through `python3 -c 'import yaml,sys;
  yaml.safe_load(sys.stdin)'` and asserts exit 0, then checks the output of
  a plain `to-yaml` run ends with exactly one `\n` via `od -c` / `tail -c`.
  Lives alongside `examples/yaml-json/smoke.sh` since this project's tests
  are shell scripts, not a `cargo test` suite.
- Runtime proof: the exact `xxd`/`od -c` + PyYAML commands in Failing
  Reproduction, re-run post-fix.
- Doc sync: none expected — `examples/yaml-json/README.md`'s existing
  description of the emitter's quoting philosophy already covers this
  behavior in spirit; no README claim needs correcting once the code matches
  it.
- Full suite: this is an example, not part of `cargo test`; `mfb build
  examples/yaml-json` and `examples/yaml-json/smoke.sh` are the applicable
  gates.

## Open Decisions

- Escaping mechanism for `quoteScalar` — patch `json::stringify`'s output
  after the fact (simplest, keeps `json::stringify` untouched) vs. build the
  double-quoted scalar for these specific codepoints without going through
  `json::stringify` at all. Recommended: post-process, since it is the
  smaller diff and does not duplicate JSON's escaping rules for the common
  case. (§Fix Design)
- Whether to include U+2029 (PARAGRAPH SEPARATOR) alongside U+2028 — not
  reproduced above (no fixture was run for it), but it is line-break-under-
  YAML-1.1 for the identical reason as U+2028. Recommended: include it, on
  the same reasoning already used for `isReservedWord`'s deliberately-wider-
  than-strictly-necessary list (`emit.mfb:27-32`). (§Fix Design)

## Summary

Both sub-issues are narrowly scoped to `examples/yaml-json/src/emit.mfb` and
its one call site in `src/main.mfb`; no package or other example is affected
(confirmed by grep). The escaping fix is a two-line addition to two existing
character-rejection loops plus a small change to `quoteScalar`'s output; the
newline fix is a one-line deletion repeated at `toYaml`'s three return sites.
The main engineering risk is verifying the codepoint list is complete (C1
range, NEL, LINE SEPARATOR, and by the same logic PARAGRAPH SEPARATOR) rather
than the mechanics of either fix.

## STATUS: FIXED (378194435)

Both Open Decisions resolved as recommended: `quoteScalar` post-processes
`json::stringify`'s output rather than duplicating JSON's escaping rules, and
U+2029 (PARAGRAPH SEPARATOR) is included alongside U+2028. No deviation from
the Fix Design otherwise. One implementation detail not anticipated by the
doc: the escape written is `\uXXXX` (4 hex digits, matching JSON's own
`\uXXXX` form), not the `\xXX` PyYAML happens to emit for the same
codepoints in its own `safe_dump` — this language's string-literal lexer has
no `\xNN` escape form at all (only `\u{HEX}`), and `\uXXXX` is equally valid
inside a YAML double-quoted scalar, so there was no reason to hand-build a
`\xXX` form instead.
