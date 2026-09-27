# bug-731: `strings` members that build a new string or list raise `ErrOutOfMemory` without declaring it

Last updated: 2026-09-25
Effort: small
Severity: MEDIUM
Class: Correctness (declared-error data disagrees with the lowering)

Status: FIXED (2026-09-27)
Regression Test: `codegen::engine::tests::builtins_codegen::declared_errors_all::
every_strings_member_declares_exactly_the_errors_its_lowering_raises` (part of
bug-698's generalized declared-errors sweep)

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

## Fix (landed via bug-698)

bug-698's generalized lowering sweep (extended with `concretize()` so a
generic `strings` member's `T`/`K` type variables resolve to a concrete type
before lowering, the same fix `collections` needed) measures every native
`strings` member's real raise set directly. Found 21 declaring no
`ErrOutOfMemory` despite building a new `String`/`List OF Byte` result: `trim`,
`trimStart`, `trimEnd`, `upper`, `lower`, `caseFold`, `normalizeNfc`,
`graphemes`, `split`, `join`, `stripPrefix`, `stripSuffix`, `left`, `right`,
`repeat`, `padLeft`, `padRight`, `graphemeAt`, `graphemesCount`, `trimChars`,
`toBytes` (this doc's own list named 11 of the 21; the sweep's direct
measurement found the other 10 too, including `left`/`right`/`padLeft`/
`padRight`, which already declared `ErrInvalidArgument` but not
`ErrOutOfMemory`). Declared it on each; added the standard prose clause to
the 16 that lacked one.

An existing test, `codegen::builtins::mod.rs::the_raising_strings_members_are_
not_infallible`, asserted `strings.upper`/`lower`/`trim` stay infallible —
exactly the belief this bug disproves, verified the same way (lowering the
real body and scanning for the raise, not inferred). Moved the three from its
infallible-neighbours list to its raising list;
`byteLen`/`contains`/`displayWidth` (genuinely untouched by this bug) keep
their infallible assertion.

`padLeftToWidth`/`padRightToWidth` (named in this doc's original prose) are
`Body::Rewrite` (rewritten to an internal MFBASIC function name), a third
non-native body kind this sweep does not check — so they were silent, not
confirmed clean. Both already declared `ErrInvalidArgument` but not
`ErrOutOfMemory`, despite `gen_pad.rs` raising it
(`builder.raise_error_bare("ErrOutOfMemory")`, line 267) and both pages'
prose already documenting the condition. Fixed directly from that evidence.

**Known remaining gap:** `strings`/`astrings`/`collections` have roughly fifty
more `Body::Mfb`/`Body::Rewrite`/`Body::Intrinsic` members this sweep cannot
measure (`grep -rl 'Body::Rewrite\|Body::mfb\|Body::Intrinsic'
src/codegen/builtins/{strings,astrings,collections}/*.rs`). Only the ones a
bug doc named by hand (this bug, bug-641, bug-724) were individually
verified against a real raise site; the rest are unmeasured, not confirmed
correct. Building bug-698's `Body::Mfb` guard (deriving a raise set from a
member's own `FAIL`s plus what it calls) would close this for good — tracked
there, not solved here.

Commit: (this worktree, bug-698 branch)
