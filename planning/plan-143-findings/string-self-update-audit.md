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

Paths: `bia` = `src/codegen/collection/assign/builder_inplace_assign.rs`,
`bc` = `src/codegen/engine/control/builder_control.rs`,
`su` = `src/codegen/collection/assign/self_update.rs`,
`idest` = `src/codegen/collection/assign/inplace_dest.rs`. Symbols are cited by
name (line numbers at `efdb54bb7` in parentheses where useful).

### B.1 Where a `String` self-update statement is dispatched, per site

- **S1 (a local).** `HirStatement::Assign` of a function local lowers to
  `IrOp::Assign` → `NirOp::Assign` (plan-141 Appendix B.1, unchanged).
  `NirOp::Assign` in `bc` (`:1192`) builds one `SelfUpdateSite` (`dest =
  InPlaceDest::Direct`, `by_ref = false`) and calls `try_inplace_self_update`
  (`su`), which runs `SELF_UPDATE_ARMS` in order; then the eight
  `try_inplace_record_field_*` arms; then the copying reassignment
  (`lower_value_owned`, then `emit_owned_value_drop` of the old block when the
  type is freeable-flat, `reassign_value` slot).
- **S2 (a module-level global).** `NirOp::StoreGlobal` in `bc` (`:1076`) builds a
  `SelfUpdateSite` with `dest = InPlaceDest::Global` **only when**
  `is_global_self_update_call(value, name)` (a `Call` whose target
  `self_update_builtin` names and whose first argument is the global, `su`) or
  `string_self_append_operands_of(value, <the global>)` (a `&` chain rooted at
  the global) holds. The site's `open_inplace_ref_dest` loads the global's block
  into the `su_global_block` slot before the arms run — emitted even when every
  arm then declines (observation O1). Otherwise, or when no arm fires: the
  copying path — `lower_value_owned`, a free of the old block through
  `store_global_old`/`store_global_new`, the store, and a reset of the hidden
  capacity global when one exists.
- **S7 (a `FOR EACH` over the binding).** Unreachable: `FOR EACH c IN s` on a
  `String` or an `AttributedString` is rejected by the type checker —
  `error[2-203-0050 TYPE_FOR_EACH_REQUIRES_COLLECTION]: FOR EACH source must be a
  List or Map` (probes `/tmp/plan-143-probes/s7`, `s7a`). Every S7 cell is
  `n/a (not iterable)`.
- **S9 (a by-ref lambda capture).** The lifted lambda's `NirOp::Assign` to the
  captured local: `by_ref = true`. When `is_self_update_call(value, name)` holds
  (a `Call` that `self_update_builtin` names, first argument the local), the site
  gets `dest = InPlaceDest::Ref` (`su_ref_block` slot), which discharges G1 for the
  collection arms (`resolve_self_update` sets `InPlaceGate.by_ref` only for a
  non-`Ref` destination, `idest`). A `&` chain is a `Binary`, so it never gets a
  `Ref` destination and the concat arm declines at G1. The copying path is the
  by-ref reassignment (`reassign_ref_old`: free the parent's old block through the
  reference, store the new one).

**Which call targets the seam can see.** `self_update_builtin` (`su`) answers
for (a) `native_builtin_target`'s bare names — `strings.`/`collections.`
`find`/`mid`/`replace` by name, and every `Body::abi_inline` member of any package
through `native_bare_target` (`src/codegen/registry/mod.rs`), which is why
`strings.trim` → `trim` and `fs.pathBaseName` → `pathBaseName` — and (b)
`#collections_*` monomorphs. It answers `None` for a `Body::mfb`/`Rewrite`
member (the call target is an internalized `#pkg_f` MFBASIC function), for a
`Body::abi_function` helper, and for the `#astrings_*` targets the Tier-B
rewrite produces. So of the §1 rows, the 22 `String` rows lowered
`Body::abi_inline`/`Body::Intrinsic` build a site at S2/S9 (marker
`su_global_block`/`su_ref_block`) and the other 41 never do.

**No arm recognises a `String` builtin.** The builtin names the arms match
(`grep -rhoE '(resolve_[a-z_]+|self_update_builtin\(target\) != Some)\([^)]*"[a-zA-Z]+"' src/codegen/collection/assign/`
→ `add append distinct drop filter insert mapValues merge mid prepend remove
removeAt removeKey replace set sort sortBy symmetricDifference take transform
union`, plus `intersection`/`difference` through `try_inplace_filter_set` and
`math.*` through `math_self_update_function`) meet the §1 function names only in
`mid` and `replace`. Both arms resolve through `resolve_self_update` (`idest`):
G2 (a `Call`) → G3/G4 (name, arity) → G5/G6 (`args[0]` is the binding) →
G-global-operand (S2) → `InPlaceGate` {G1, G7, **G10**}. G10 —
`CollectionTypeLayout::from_type(type)` is `None` — declines every `String` and
`AttributedString` by construction: `from_type`
(`src/codegen/engine/validation/validation.rs`) answers only `ListOf`, a set, or
a map type. `strings.mid`/`strings.replace` reach the arm at all only because
`native_builtin_target` dequalifies the `strings.` and `collections.` spellings to
the same bare name (`src/codegen/builtins/mod.rs`, comment there). The concat arm
(`try_inplace_concat_assign`, `bia:1609`) matches only a `Binary` `&` chain.

### B.2 Lowering of each §1 row

Every row's result is a freshly allocated block (or record) that does not reuse
`s`'s block; no builtin argument is moved into its result.

| # | function definition | body kind | result block |
|---|---|---|---|
| r01 | `astrings::addAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_addAttribute` | fresh record: `astrings::writeSpans(a, spans)` (`gen_astrings.rs`) builds a new `AttributedString` record with a copy of the text |
| r02 | `astrings::clearAttributes(value AS AttributedString) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_clearAttributes` | fresh record: `astrings::writeSpans(a, [])` |
| r03 | `astrings::clearAttributes(value AS AttributedString, start AS Integer, endIndex AS Integer) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_clearAttributesRange` | fresh record: `astrings::writeSpans(a, out)` |
| r04 | `astrings::removeAttribute(value AS AttributedString, start AS Integer, endIndex AS Integer, attr AS astrings::Attribute) AS AttributedString` | `Body::mfb` → MFBASIC `__astrings_removeAttribute` | fresh record: `astrings::writeSpans(a, spans)` |
| r05 | `encoding::formUrlDecode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_formUrlDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r06 | `encoding::formUrlEncode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_formUrlEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r07 | `encoding::htmlEscape(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_htmlEscape` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r08 | `encoding::htmlUnescape(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_htmlUnescape` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r09 | `encoding::percentDecode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_percentDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r10 | `encoding::percentEncode(value AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_percentEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r11 | `encoding::punycodeDecode(asciiDomain AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_punycodeDecode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r12 | `encoding::punycodeEncode(domain AS String) AS String` | `Body::mfb` → MFBASIC `__encoding_punycodeEncode` body (`builtins/encoding/func_*.rs`) | fresh block: the body builds and `RETURN`s a new `String` |
| r13 | `fs::canonicalPath(path AS String) AS String` | `Body::abi_function` → `gen_canonical::lower_fs_canonical_path_helper` | fresh block copied from the `realpath` `PATH_MAX` scratch buffer; `path` itself is marshalled into a scratch C string `c_path` first (bug-574 scratch, freed at the helper's exit) |
| r14 | `fs::pathBaseName(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_base_name_nl` (`gen_path_builder.rs`) | fresh block: `emit_materialize_string_from_bytes` of the last component's span |
| r15 | `fs::pathDirName(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_dir_name_nl` (`gen_path_builder.rs`) | fresh block of the directory span (`emit_materialize_string_from_bytes`), **or a read-only constant**: `load_string_constant(".")` / `("/")` for a path with no directory part / the root (`gen_path_builder.rs`, labels `dot`, `root`) — see §3.2 finding F3 |
| r16 | `fs::pathExtension(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_extension_nl` (`gen_path_builder.rs`) | fresh block: `emit_materialize_string_from_bytes` of the extension span |
| r17 | `fs::pathNormalize(path AS String) AS String` | `Body::abi_inline` → `lower_fs_path_normalize_nl` (`gen_path_builder.rs`) | fresh block: the normalized bytes are assembled and materialized |
| r18 | `fs::readText(path AS String) AS String` | `Body::abi_function` → `lower_fs_read_text_path_helper` (`gen_atomic_write.rs`) | fresh block holding the file's bytes; `path` is copied into a scratch C path first (`{symbol}_path_copy_loop`) |
| r19 | `io::input([prompt AS String]) AS String` | `Body::abi_function` → `lower_read_line_family(with_prompt = true)` (`gen_read_line_family.rs`) | fresh block: a grown line buffer copied into the result (`result_copy_loop`); `prompt` is only written to stdout |
| r20 | `net::percentDecode(s AS String) AS String` | `Body::mfb` → MFBASIC `__net_percentDecode` → `__net_percentDecodeImpl` | fresh block: the body builds and `RETURN`s a new `String` |
| r21 | `os::getEnv(name AS String) AS String` | `Body::abi_function` → `gen_env::lower_get_env(with_fallback = false)` | fresh block holding the variable's value; `name` is copied into a scratch C string (`marshal_cstring`, bug-574 scratch, freed at `done`) |
| r22 | `os::getEnvOr(name AS String, fallback AS String) AS String` | `Body::abi_function` → `gen_env::lower_get_env(with_fallback = true)` | fresh block holding the value or a copy of `fallback`; `name` is copied into a scratch C string (`marshal_cstring`) |
| r23 | `os::resourcePath(relative AS String) AS String` | `Body::abi_function` → `lower_resource_path` (`os/func_resource_path.rs`) | fresh block: `base + "/" + relative` concatenated into an owned arena `String` |
| r24 | `regex::replace(value AS String, pattern AS String, replacement AS String) AS String` | `Body::mfb` → MFBASIC `__regex_replace` | fresh block: the body builds and `RETURN`s a new `String` |
| r25 | `strings::caseFold(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r26 | `strings::graphemeAt(value AS String, index AS Integer) AS String` | `Body::abi_inline` → `func_grapheme_at::lower` | fresh block: `emit_materialize_string_from_bytes` of the grapheme's byte span |
| r27 | `strings::left(value AS String, count AS Integer) AS String` | `Body::abi_inline` → `gen_left_right::lower_strings_left_right` | fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it |
| r28 | `strings::lower(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r29 | `strings::mid(value AS String, start AS Integer, count AS Integer) AS String` | `Body::Intrinsic` → `native_builtin_target` = `mid` → `lower_mid` (`collection/search/builder_search.rs`, `String` branch) | fresh block: `emit_arena_alloc_call` (`mid_alloc_ok`) and a span copy |
| r30 | `strings::normalizeNfc(value AS String) AS String` | `Body::abi_inline` → `func_normalize_nfc::lower` | fresh block: `emit_arena_alloc_call` (three sites in `func_normalize_nfc.rs`) |
| r31 | `strings::padLeft(value AS String, width AS Integer, [padChar AS String]) AS String` | `Body::abi_inline` → `gen_pad::lower_strings_pad` | fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`) |
| r32 | `strings::padLeftToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | `Body::Rewrite("__strings_padLeftToWidth")` → MFBASIC helper (`helper_pad_to_width.rs`) | fresh block: the helper builds and `RETURN`s a new `String` |
| r33 | `strings::padRight(value AS String, width AS Integer, [padChar AS String]) AS String` | `Body::abi_inline` → `gen_pad::lower_strings_pad` | fresh block: `emit_arena_alloc_call` of the padded length (`gen_pad.rs`) |
| r34 | `strings::padRightToWidth(value AS String, columns AS Integer, [padChar AS String]) AS String` | `Body::Rewrite("__strings_padRightToWidth")` → MFBASIC helper (`helper_pad_to_width.rs`) | fresh block: the helper builds and `RETURN`s a new `String` |
| r35 | `strings::repeat(value AS String, times AS Integer) AS String` | `Body::abi_inline` → `func_repeat::lower` | fresh block: `emit_arena_alloc_call` of `len × times + 9` |
| r36 | `strings::replace(value AS String, old AS String, new AS String) AS String` | `Body::Intrinsic` → `native_builtin_target` = `replace` → `lower_replace` (`string/repr/builder_strings.rs`, `String` branch) | fresh block on both arms: `emit_arena_alloc_call` when a match is replaced, `copy_flat_block` of `value` when none is (bug-536 shape B) |
| r37 | `strings::right(value AS String, count AS Integer) AS String` | `Body::abi_inline` → `gen_left_right::lower_strings_left_right` | fresh block: computes a `(ptr, len)` window into `value`, then `emit_materialize_string_from_bytes` (`builder_collection_layout.rs`) copies it |
| r38 | `strings::stripPrefix(value AS String, prefix AS String) AS String` | `Body::abi_inline` → `gen_strip::lower_strings_strip` | fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes` |
| r39 | `strings::stripSuffix(value AS String, suffix AS String) AS String` | `Body::abi_inline` → `gen_strip::lower_strings_strip` | fresh block: window into `value` (the whole of it when the affix is absent), then `emit_materialize_string_from_bytes` |
| r40 | `strings::trim(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r41 | `strings::trimChars(value AS String, chars AS String) AS String` | `Body::abi_inline` → `func_trim_chars::lower` | fresh block: window into `value`, then `emit_materialize_string_from_bytes` |
| r42 | `strings::trimEnd(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r43 | `strings::trimStart(value AS String) AS String` | `Body::abi_inline` → `gen_trim::lower_strings_trim` | fresh block: `[start, end)` window into `value`, then `emit_materialize_string_from_bytes` |
| r44 | `strings::upper(value AS String) AS String` | `Body::abi_inline` → `gen_case_map::lower_strings_case_map` | fresh block: `emit_arena_alloc_call` of `byteLen + 9` on the all-ASCII path, or of the counted mapped length on the Unicode path (`gen_case_map.rs`) |
| r45 | `strings::left(value AS AttributedString, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_left` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_left` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r46 | `strings::right(value AS AttributedString, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_right` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_right` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r47 | `strings::mid(value AS AttributedString, start AS Integer, count AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_mid` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_mid` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r48 | `strings::trim(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trim` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trim` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r49 | `strings::trimStart(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimStart` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimStart` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r50 | `strings::trimEnd(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimEnd` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimEnd` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r51 | `strings::trimChars(value AS AttributedString, chars AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_trimChars` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_trimChars` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r52 | `strings::stripPrefix(value AS AttributedString, prefix AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_stripPrefix` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_stripPrefix` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r53 | `strings::stripSuffix(value AS AttributedString, suffix AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_stripSuffix` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_stripSuffix` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r54 | `strings::padLeft(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padLeft` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padLeft` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r55 | `strings::padLeftToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padLeftToWidth` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padLeftToWidth` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r56 | `strings::padRight(value AS AttributedString, width AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padRight` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padRight` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r57 | `strings::padRightToWidth(value AS AttributedString, columns AS Integer, [padChar AS String]) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_padRightToWidth` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_padRightToWidth` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r58 | `strings::repeat(value AS AttributedString, times AS Integer) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_repeat` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_repeat` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r59 | `strings::replace(value AS AttributedString, old AS String, new AS String) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_replace` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_replace` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r60 | `strings::upper(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_upper` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_upper` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r61 | `strings::lower(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_lower` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_lower` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r62 | `strings::caseFold(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_caseFold` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_caseFold` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |
| r63 | `strings::normalizeNfc(value AS AttributedString) AS AttributedString` | IR rewrite (`strings::tier_b_transform_impl`, `TIER_B_TRANSFORMS` in `src/codegen/builtins/strings/mod.rs`) to `Body::mfb` MFBASIC `__astrings_normalizeNfc` (`builtins/astrings/helper_*.rs`); the call reaches codegen as `#astrings_normalizeNfc` | fresh record: the helper builds a new text and a remapped span list and assembles a new `AttributedString` (`__astrings_assemble` / `astrings::writeSpans`) |

### B.3 The `String` representation facts the form column depends on

1. **The tight block.** A `String` is `{U64 byteLength, Byte[byteLength],
   U8 NUL}` and its allocation size is exactly `byteLength + 9`
   (`mfb spec memory heap-values`, "Standalone String";
   `src/docs/spec/memory/03_heap-values.md`). The arena free takes that size
   (`emit_owned_value_drop` sizes a `String` as `len + 9`, plus the capacity
   shadow when there is one — bug-560). So a block whose length changed in
   place must have its true size tracked somewhere, or the drop frees the wrong
   size: an in-place **shrink** leaves spare bytes that only a capacity shadow can
   describe, exactly as the self-append's **grow** does today. A `shrink` or
   `rewrite` arm therefore needs the capacity shadow (fact 2) at every site it
   runs, not just the self-append targets.
2. **The capacity shadow.** `prescan_string_self_appends` (`bc:2374`) allocates a
   frame slot `strcap_<name>` for every local that is the target of a `&`
   self-append anywhere in the function, unless the local is in
   `address_taken_locals` (a by-ref capture, `rt_byref_string_capture_capacity`).
   It holds the spare bytes past `byteLength` in the live block and is zeroed in
   the prologue; `reset_string_capacity_shadow` zeroes it on every other
   bind/assign (which installs a tight block); only the concat arm's regrow makes
   it non-zero (`lower_string_self_append_one`, geometric step, `bia:1688`);
   `string_capacity_slot_for` (`bc:2351`) hands it to the drop so the grown block
   is freed at its real size (bug-560). A global's shadow is the hidden global
   `$strcap$<name>` (`add_global_string_capacities`, `su`), declared only for a
   global `String` that is a `&` self-append target, reset to 0 by every other
   `StoreGlobal` (plan-142-H). The shadow never escapes: every copy, return or
   transfer reads `byteLength` bytes and freezes the value to the tight form.
3. **Read-only data.** A `String` literal and the Unicode property names are
   rodata. A `MUT` binding never holds one at the moment of a self-update: its
   bind goes through `lower_value_owned`, whose `value_needs_owning_copy`
   (`src/codegen/engine/value/builder_values.rs`) copies a `static_string_value`,
   a `call_returns_rodata_string` target, an aliasing source or a parameter
   borrow (probe `/tmp/plan-143-probes/rodata`: `bindOnly` carries
   `flat_copy_result`; a global's initializer is a `StoreGlobal` and takes the
   same copy). **Two producers escape that predicate** and hand an owning store a
   block the binding must not free — findings F1 (`toString(<String>)` returns
   its argument's block) and F3 (`fs::pathDirName` returns a rodata `.`/`/`) —
   plus the known bug-667 (`toString(<Boolean>)` returns rodata). A fix that
   writes into `s`'s block in place inherits every one of these: it must only
   run on a block `s` owns.
4. **How a `String` argument is passed.** Borrowed, never copied, for every §1
   row: a native lowering (`abi_inline`/`Intrinsic`) reads `args[0]`'s pointer
   directly; a `bl` to an MFBASIC body or an `abi_function` helper passes the
   pointer in the argument register (`mfb spec language memory-semantics` §14:
   "Native lowering passes a global argument without copying"). The exceptions
   are copies made **to protect** the borrow: `want_arguments_the_call_can_free`
   (`src/codegen/engine/value/operand_snapshot.rs`) snapshots a global argument
   when the call can reach a store to that global (bug-665), and the sibling
   rule `operand_reachable_by_later_call` snapshots an argument a *later*
   operand's call could reassign — observed at S2 and S9 for `addAttribute` and
   `removeAttribute`, whose later operand `astrings::bold()` is such a call
   (slot `operand_snapshot` in `r01_S2`, `r04_S2`, `$lambda0`, `$lambda3`).
   The `abi_function` helpers `fs::readText`, `fs::canonicalPath`, `os::getEnv`
   and `os::getEnvOr` additionally copy the argument's bytes into a scratch C
   string inside the helper (`marshal_cstring` / the path copy loop), freed
   before the helper returns.

## Appendix C — `--ncode` probes
