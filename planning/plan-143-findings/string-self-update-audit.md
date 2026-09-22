# plan-143 findings: which `String` self-updates the compiler performs in place

Measured against: **a `MUT` updating itself mutates in place, with no copy.**
Every verdict below is read from the compiler source at `efdb54bb7` (plan-142
landed) and cross-checked against `mfb build --ncode` (Appendix C). This is the
`String` counterpart of `planning/plan-141-findings/inplace-audit.md`; its method,
gate codes (G1–G26) and marker technique are reused unchanged.

## Binding sites (the columns)

| Site | Meaning |
|---|---|
| S1 local | `MUT s` declared in a FUNC/SUB body |
| S2 global | `MUT s` at module level |
| S7 loop-live | `MUT s` while a `FOR EACH` walks the same binding |
| S9 captured | `MUT s` assigned inside a non-escaping `forEach` `LAMBDA` that captures it (a `by_ref` local) |

Records, `RES` and `STATE` are excluded (plan §1 non-goals): a `String` record
field is not a column.

Cell values: `y` = in place; `n (<gate>)` = the named gate declines (the first one
in code order that does) and the statement takes the copying path;
`n (no arm)` = no arm of `SELF_UPDATE_ARMS` recognises the call, so the statement
always takes the copying path; `n/a` = the form cannot be written (reason given).

## 1. Overloads

| function definition | form | S1 | S2 | S7 | S9 | evidence |
|---|---|---|---|---|---|---|
| `astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | | | | | | |
| `astrings::clearAttributes(value AS AttributedString) AS AttributedString` | | | | | | |
| `astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString` | | | | | | |
| `astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | | | | | | |
| `encoding::formUrlDecode(value AS String) AS String` | | | | | | |
| `encoding::formUrlEncode(value AS String) AS String` | | | | | | |
| `encoding::htmlEscape(value AS String) AS String` | | | | | | |
| `encoding::htmlUnescape(value AS String) AS String` | | | | | | |
| `encoding::percentDecode(value AS String) AS String` | | | | | | |
| `encoding::percentEncode(value AS String) AS String` | | | | | | |
| `encoding::punycodeDecode(asciiDomain AS String) AS String` | | | | | | |
| `encoding::punycodeEncode(domain AS String) AS String` | | | | | | |
| `fs::canonicalPath(path AS String) AS String` | | | | | | |
| `fs::pathBaseName(path AS String) AS String` | | | | | | |
| `fs::pathDirName(path AS String) AS String` | | | | | | |
| `fs::pathExtension(path AS String) AS String` | | | | | | |
| `fs::pathNormalize(path AS String) AS String` | | | | | | |
| `fs::readText(path AS String) AS String` | | | | | | |
| `io::input([prompt AS String]) AS String` | | | | | | |
| `net::percentDecode(s AS String) AS String` | | | | | | |
| `os::getEnv(name AS String) AS String` | | | | | | |
| `os::getEnvOr(name AS String, fallback AS String) AS String` | | | | | | |
| `os::resourcePath(relative AS String) AS String` | | | | | | |
| `regex::replace(value AS String, pattern AS String, replacement AS String) AS String` | | | | | | |
| `strings::caseFold(value AS String) AS String` | | | | | | |
| `strings::graphemeAt(value AS String, index AS Integer) AS String` | | | | | | |
| `strings::left(value AS String, count AS Integer) AS String` | | | | | | |
| `strings::lower(value AS String) AS String` | | | | | | |
| `strings::mid(value AS String, start AS Integer, count AS Integer) AS String` | | | | | | |
| `strings::normalizeNfc(value AS String) AS String` | | | | | | |
| `strings::padLeft(value AS String, width AS Integer, [padChar AS String]) AS String` | | | | | | |
| `strings::padLeftToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | | | | | | |
| `strings::padRight(value AS String, width AS Integer, [padChar AS String]) AS String` | | | | | | |
| `strings::padRightToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | | | | | | |
| `strings::repeat(value AS String, times AS Integer) AS String` | | | | | | |
| `strings::replace(value AS String, old AS String, new AS String) AS String` | | | | | | |
| `strings::right(value AS String, count AS Integer) AS String` | | | | | | |
| `strings::stripPrefix(value AS String, prefix AS String) AS String` | | | | | | |
| `strings::stripSuffix(value AS String, suffix AS String) AS String` | | | | | | |
| `strings::trim(value AS String) AS String` | | | | | | |
| `strings::trimChars(value AS String, chars AS String) AS String` | | | | | | |
| `strings::trimEnd(value AS String) AS String` | | | | | | |
| `strings::trimStart(value AS String) AS String` | | | | | | |
| `strings::upper(value AS String) AS String` | | | | | | |
| `strings::left(value AS AttributedString, count AS Integer) AS AttributedString` | | | | | | |
| `strings::right(value AS AttributedString, count AS Integer) AS AttributedString` | | | | | | |
| `strings::mid(value AS AttributedString, start AS Integer, count AS Integer) AS AttributedString` | | | | | | |
| `strings::trim(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::trimStart(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::trimEnd(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::trimChars(value AS AttributedString, chars AS String) AS AttributedString` | | | | | | |
| `strings::stripPrefix(value AS AttributedString, prefix AS String) AS AttributedString` | | | | | | |
| `strings::stripSuffix(value AS AttributedString, suffix AS String) AS AttributedString` | | | | | | |
| `strings::padLeft(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | | | | | | |
| `strings::padLeftToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | | | | | | |
| `strings::padRight(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | | | | | | |
| `strings::padRightToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | | | | | | |
| `strings::repeat(value AS AttributedString, times AS Integer) AS AttributedString` | | | | | | |
| `strings::replace(value AS AttributedString, old AS String, new AS String) AS AttributedString` | | | | | | |
| `strings::upper(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::lower(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::caseFold(value AS AttributedString) AS AttributedString` | | | | | | |
| `strings::normalizeNfc(value AS AttributedString) AS AttributedString` | | | | | | |

## 2. Operator and unqualified forms

| form | S1 | S2 | S7 | S9 | evidence |
|---|---|---|---|---|---|

## 3. Summary

## Appendix A — census script

`census.py rows` generated the 63 rows of §1 (signatures verbatim from each
`mfb man <pkg> <f>` page for the 44 literal hits; the 19 `AttributedString` rows
substitute `AttributedString` for the leading `String` parameter and the return of
the `String` overload, as `strings::resolve_return_type` does for a Tier-B
transform). Run from the repo root against `target/release/mfb` built from
`efdb54bb7`:

```
$ python3 census.py stats
packages 42 overloads 828
literal 44 {'astrings': 4, 'encoding': 8, 'fs': 6, 'io': 1, 'net': 1, 'os': 3, 'regex': 1, 'strings': 20}
generic-candidates 25
$ python3 census.py rows | wc -l
63
```

The 25 generic candidates (`census.py generic`) are: 7 `astrings` overloads and
2 `io` overloads whose "variable" is the nominal `AttributedString`, 5 `strings::is*`
predicates over the nominal `Scalar`, and 11 true generics (`collections::get`×2,
`getOr`×2, `reduce`, `reduceRight`; `thread::accept`×2, `receive`×2, `waitFor`).
None of the 11 can be a `String` self-update: each one's first parameter is a
`List`/`Map`/`Thread`/`ThreadWorker`, never a `String`, so `s = f(s, …)` does not
type-check.

```python
"""Row census for planning/plan-143-findings/string-self-update-audit.md.

plan-142-A's Appendix census over every `mfb man <pkg> <f>` page, with the type
test replaced by the String/AttributedString rule, plus a generic scan: every
overload whose first parameter or return type is a bare type variable (one
capitalised identifier that is not a builtin type name) is listed so the reader
can decide whether it can be instantiated to a String self-update.

Run from the repo root (it calls target/release/mfb).

  python3 census.py literal   -> the literal hits (first param type == return type,
                                  both String / AttributedString)
  python3 census.py generic   -> overloads with a type-variable first param or return
  python3 census.py stats     -> packages / overloads / literal per package
  python3 census.py rows      -> one empty §1 table row per overload: the literal
                                  hits, then the Tier-B `AttributedString` overloads
                                  of `strings::` (prose-documented, so invisible to
                                  the man census; list from TIER_B_TRANSFORMS in
                                  src/codegen/builtins/strings/mod.rs)
"""
import re
import subprocess
import sys
from collections import Counter

M = "target/release/mfb"
STRINGY = ("String", "AttributedString", "astrings::AttributedString")
BUILTIN = {"String", "Integer", "Float", "Boolean", "Byte", "Fixed", "Money",
           "Nothing", "Error", "Duration", "Big", "Char"}


def is_var(t):
    return bool(re.fullmatch(r"[A-Z][A-Za-z0-9]*", t)) and t not in BUILTIN


top = subprocess.run([M, "man"], capture_output=True, text=True).stdout
pkgs = re.findall(r"^│ ([a-zA-Z]+) +│", top[top.index("Builtin packages"):], re.M)
pkgs = [p for p in pkgs if p != "Package"]
literal, generic, total = [], [], 0
for pkg in pkgs:
    page = subprocess.run([M, "man", pkg], capture_output=True, text=True).stdout
    if "\nFunctions\n" not in page:
        continue
    funcs = []
    for f in re.findall(rf"│ {pkg}::([a-zA-Z0-9_]+)", page[page.index("\nFunctions\n"):]):
        if f not in funcs:
            funcs.append(f)
    for f in funcs:
        fp = subprocess.run([M, "man", pkg, f], capture_output=True, text=True).stdout
        lines = fp.splitlines()
        try:
            i = next(k for k, l in enumerate(lines) if l.strip() in ("Overloads", "Declaration"))
        except StopIteration:
            continue
        j, block = i + 2, []
        while j < len(lines) and not (lines[j].strip() and j + 1 < len(lines)
                                      and set(lines[j + 1].strip()) == {"─"}):
            block.append(lines[j])
            j += 1
        text = " ".join(l.strip() for l in block)
        for s in re.findall(rf"`({pkg}::[^`]+)`", text):
            s = re.sub(r"\s+", " ", s)
            m = re.match(r"\w+::\w+\((.*)\) AS (.*)$", s)
            if not m:
                continue
            total += 1
            params, ret = m.group(1), m.group(2)
            first = re.match(r"\[?\w+ AS (.*?)(?:, \[?\w+ AS |\]?$)", params)
            ft = first.group(1).rstrip("]") if first else ""
            if ft in STRINGY and ft == ret:
                literal.append((pkg, s))
            if is_var(ft) or is_var(ret):
                generic.append((pkg, s))
TIER_B = ["left", "right", "mid", "trim", "trimStart", "trimEnd", "trimChars",
          "stripPrefix", "stripSuffix", "padLeft", "padLeftToWidth", "padRight",
          "padRightToWidth", "repeat", "replace", "upper", "lower", "caseFold",
          "normalizeNfc"]
mode = sys.argv[1] if len(sys.argv) > 1 else "stats"
if mode == "rows":
    out = [s for _, s in literal]
    by_name = {re.match(r"strings::(\w+)\(", s).group(1): s
               for _, s in literal if s.startswith("strings::")}
    for f in TIER_B:
        s = by_name[f]
        s = re.sub(r"^(strings::\w+\(\w+) AS String", r"\1 AS AttributedString", s)
        s = re.sub(r"\) AS String$", ") AS AttributedString", s)
        out.append(s)
    for s in out:
        print(f"| `{s}` | | | | | | |")
elif mode == "literal":
    for _, s in literal:
        print(s)
elif mode == "generic":
    for _, s in generic:
        print(s)
else:
    print("packages", len(pkgs), "overloads", total)
    print("literal", len(literal), dict(Counter(p for p, _ in literal)))
    print("generic-candidates", len(generic))
```

## Appendix B — recogniser and lowering map

## Appendix C — `--ncode` probes
