# bug-731: `strings` members that build a new string or list raise `ErrOutOfMemory` without declaring it

Last updated: 2026-09-25
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: Open
Regression Test: none yet — see Phase 1

No `strings` descriptor lists `ErrOutOfMemory`
(`grep -l 'errors: vec!\[.*ErrOutOfMemory' src/codegen/builtins/strings/*.rs` → nothing),
yet ten lowering files raise it:

```
$ grep -ln ErrOutOfMemory src/codegen/builtins/strings/*.rs
func_join.rs  func_normalize_nfc.rs  func_pad_left_to_width.rs
func_pad_right_to_width.rs  func_repeat.rs  func_split.rs  func_to_bytes.rs
gen_case_map.rs  gen_graphemes.rs  gen_pad.rs
```

So `join`, `normalizeNfc`, `padLeftToWidth`, `padRightToWidth`, `repeat`, `split`,
`toBytes`, `upper`/`lower`/`caseFold` (`gen_case_map`), the grapheme members
(`gen_graphemes`) and `padLeft`/`padRight` (`gen_pad`) render no `ErrOutOfMemory`
row. Measured on one of them:

```
$ # strings::padLeftToWidth("a", 9000000000000000000, "-") TRAP(e)  (reviewer probe p_big)
ERR 77010001 Allocation failed.
$ mfb man strings padLeftToWidth | grep -A4 Errors     # lists only 77050002
```

`padLeftToWidth`/`padRightToWidth` even name the error in their prose, so the page
contradicts its own derived table. Elsewhere 35 descriptors do declare it
(`grep -rln 'errors: vec!\[.*"ErrOutOfMemory"' src/codegen/builtins | wc -l`), and
the `fs` package was brought into line by bug-694.

This is the `strings` instance of the class filed for `encoding` (bug-606), `udp`
(bug-608), `collections` (bug-641), `io` (bug-722), `astrings` (bug-724) and `term`
(bug-727).

**The single correct behavior a fix produces:** every `strings` member whose
lowering can raise `ErrOutOfMemory` declares it, per overload, so its page renders
the row; members that cannot raise it do not.

Found by plan-125-D Phase 4 (the Fable reviews of `strings/join` and
`strings/padLeftToWidth`).
**Filed, not fixed**, by user instruction during a documentation-only plan ("file
all bugs, make no fixes").

## Reproduction

```basic
IMPORT io
IMPORT strings

FUNC main() AS Integer
  LET s AS String = strings::padLeftToWidth("a", 9000000000000000000, "-") TRAP(e)
    io::print(toString(e.code))
    RETURN 0
  END TRAP
  RETURN 0
END FUNC
```

Prints `77010001`; `mfb man strings padLeftToWidth` has no such row.

## Root cause

The descriptors were written before the lowerings grew their allocation-failure
branches; nothing checks declared errors against raise sites for this package.

## Non-goals

- Declaring `ErrOutOfMemory` on members that only read (`contains`, `find`, …).

## Blast-radius audit

- The `astrings` overloads that call these members raise the same error.
- A registry-wide check that every `raise_error*("X")` in a member's lowering is in
  its `errors` would catch the whole class; see how bug-694 verified `fs`.

## Fix

Phase 1 — RED test asserting each listed member declares `ErrOutOfMemory`. Commit:

Phase 2 — add it to each descriptor (per overload); re-render the pages. Commit:
