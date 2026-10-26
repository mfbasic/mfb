### 1. The one-argument form is declared infallible but builds a new value through an allocating step
UNIT:      man-page:astrings/clearAttributes
CLAIM:     "The three-argument form can also raise ErrOutOfMemory, meaning the program ran out of memory; the one-argument form never fails." (and the derived row "Overload 1 raises no errors.")
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/astrings/func_clear_attributes.rs:BODY_WHOLE` is `RETURN astrings::writeSpans(a, empty)`. `func_write_spans.rs:register` declares `errors: vec!["ErrOutOfMemory"]` with the comment "bug-928: builds a new block, so a failed arena allocation raises ErrOutOfMemory"; its lowering `gen_astrings.rs:lower_astrings_write_spans` calls `emit_build_inlined_record`, whose contract (`src/codegen/memory/marshal/record.rs:emit_build_inlined_record`) "Branches to `alloc_fail` on allocation failure". The sibling with the identical shape, `func_from_string.rs`, declares `ErrOutOfMemory` for the same reason. Overload 1 of `clearAttributes` declares `errors: vec![]`. The guard that should catch this, `src/codegen/builtins/tests/mfb_body_declared_errors/mod.rs`, states it checks "per member (its implementations' bodies unioned)", so overload 2's `ErrOutOfMemory` masks overload 1's missing declaration. Per `.ai/man-content.md` §0 a wrong derived Errors table is a descriptor bug, not a prose fix.
SUGGESTED: Descriptor: overload 1 `errors: vec!["ErrOutOfMemory"]`. Prose: "Either form can raise `ErrOutOfMemory`, meaning the program ran out of memory while building the result; the one-argument form raises nothing else."

### 2. "member" is undefined on this page and names an internal span field
UNIT:      man-page:astrings/clearAttributes
CLAIM:     "(regardless of member — unlike removeAttribute, no structural match is required)"
VERDICT:   out-of-scope
EVIDENCE:  "member" here is the `member` field of `AttrSpan` (`helper_split_span.rs` copies `s.member`), a record that reaches the developer only through `readSpans`/`writeSpans`, both `internal_only: true` (`func_read_spans.rs:37`, `func_write_spans.rs:37`; `mfb man astrings readSpans` prints "unknown astrings function"). Nothing on this page or on `mfb man astrings types` defines "member" or "structural match" for the reader; the probe confirms the behaviour meant (a bold span 0–10 and an italic span 3–5 are both cut by `clearAttributes(b, 2, 7)`: `**he**llo wo**rld**`).
SUGGESTED: "(whichever attributes they are — unlike removeAttribute, which clears only the one attribute you name)"

### 3. The guard belongs on the ranged call, not on construction
UNIT:      man-page:astrings/clearAttributes
CLAIM:     "so guard construction from text that might be empty"
VERDICT:   misleading
EVIDENCE:  Probe (`/tmp/plan-125-scratch/D-phase6-current/man-page-astrings-clearAttributes/prproj/src/main.mfb`): `astrings::fromString("")` succeeds and `astrings::clearAttributes(em)` on it succeeds, printing `empty whole ok: []`; only `clearAttributes(em, 0, 0)` raises (`code 77050001 msg=attribute range out of bounds`). Constructing from empty text is fine; it is the three-argument call that must be guarded.
SUGGESTED: "so check that the text is non-empty before calling the three-argument form."

### 4. The example shows none of the behaviour the page describes
UNIT:      man-page:astrings/clearAttributes
CLAIM:     The Examples block (`LET ranged … clearAttributes(b, 2, 7)` / `LET whole … clearAttributes(b)`), which binds two results and prints nothing.
VERDICT:   incomplete
EVIDENCE:  Copied verbatim to `exproj/src/main.mfb`; `mfb build exproj` succeeds and `./exproj/build/exproj.out` prints nothing (exit 0). The page's central sharp edge, that a straddling span is split and its flanks survive, is unobservable. My probe with `astrings::toMarkdown` prints `b before: **hel*****lo *****world**`, `ranged  : **he**llo wo**rld**`, `whole   : hello world`, and `b after : **hel*****lo *****world**`, which also demonstrates the "returns a new AttributedString" claim (the input is unchanged).
SUGGESTED: Add `IMPORT io` and three prints: `io::print(astrings::toMarkdown(b))` → `**hello world**`, `io::print(astrings::toMarkdown(ranged))` → `**he**llo wo**rld**`, `io::print(astrings::toMarkdown(whole))` → `hello world`. Also change `MUT b` to `LET b`, since the example never reassigns it.

Everything else on the page checked out against the probe: inclusive range (`0,10` clears all, `10,10` leaves `**hel*****lo *****worl**d`), `start < 0` or `start > endIndex` → `77050002 invalid attribute range` (checked before the bounds test, matching the prose order: `-1,20` raises `77050002`), `endIndex >= length` → `77050001 attribute range out of bounds`, scalar rather than byte indexing (`"héllo"` accepts index 4 and rejects 5), and no banned vocabulary on the rendered page.
