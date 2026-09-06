# bug-530: `encoding::utf8Encode` is a return-type overload whose second form does not appear in its rendered signature

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: FIXED — the DESCRIPTOR was the defect (hypothesis 2), not the renderer
Regression Test: `src/cli/man.rs` — `a_return_type_overload_renders_every_form`,
`only_a_return_type_overload_gets_the_expected_type_note`,
`every_member_renders_one_declaration_per_implementation`; `src/codegen/registry/mod.rs`
— `no_member_defines_more_source_forms_than_its_descriptor_declares`

`encoding::utf8Encode` is the only return-type overload in the language: the
same `String` argument produces either a `List OF Byte` or a `List OF Integer`,
chosen by the expected type at the call site. A call with no type context is a
**compile error**, `TYPE_OVERLOAD_AMBIGUOUS` (`2-203-0101`).

`mfb man encoding utf8Encode` renders it as if it had one signature:

```
Declaration
───────────

`encoding::utf8Encode(value AS String) AS List OF Byte`

Returns List OF Byte.
```

There is no **Overloads** block. Every other multi-form member in the tree gets
one — `tls::poll`, `tcp::poll`, `crypto::encrypt`, `datetime::parse` all render
their forms as a list. So the `List OF Integer` form is invisible in the two
places a reader looks for the signature, appearing only in a paragraph
further down:

> `utf8Encode` is a return-type overload: the same `String` argument produces
> either a `List OF Byte` or a `List OF Integer`, chosen by the expected
> (contextual) type at the call site.

A reader who takes the Declaration block at face value concludes the member
returns `List OF Byte` unconditionally — and is then surprised by a compile
error on a perfectly ordinary-looking call, from a rule that applies to nothing
else in the language.

The single correct behavior a fix produces: `mfb man encoding utf8Encode`
renders both forms in an Overloads block, and states in the signature area —
not only in prose — that an expected type is required.

References:

- `src/codegen/builtins/encoding/func_utf8_encode.rs:23-27`
- `src/codegen/builtins/encoding/mod.rs:11`
- `src/cli/man.rs` — the renderer that chooses Declaration vs. Overloads
- `src/rules/table.rs` — `TYPE_OVERLOAD_AMBIGUOUS` (`2-203-0101`)
- Spike: `spikes/api-review/bug-530-utf8encode-return-overload/`

## Failing Reproduction

```
./target/release/mfb man encoding utf8Encode | head -20
./target/release/mfb man tcp poll | head -14
```

- Observed: `utf8Encode` renders a single `Declaration` line ending
  `AS List OF Byte`; `tcp::poll` renders an `Overloads` block with two entries.

Then the error the page does not prepare you for:

```
io::print(toString(len(encoding::utf8Encode("hi"))))
```

```
error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]: return-type overload cannot be resolved without an expected type
    Call to `encoding.utf8Encode` matches 2 overloads that differ only by return
    type; supply the expected type (e.g. a `LET … AS` annotation) to select one.
```

- Expected: an `Overloads` block listing both forms.

Contrast cases, correct today:

- Both forms work when annotated — the spike prints `List OF Byte : 2` and
  `List OF Integer : 2`. The *behavior* is fine.
- `strings::toBytes` is the unambiguous byte path with one signature and no
  contextual requirement, and the page correctly points at it.
- The diagnostic itself is excellent: it names the member, the count, and the
  fix. The problem is that the page gave no warning it was possible.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ (renderer defect; target-independent) |

## Root Cause

**RESOLVED (2026-09-05): hypothesis 2. The renderer was innocent.**

`render_function_markdown` (`src/cli/man.rs`) does not de-duplicate anything — it
renders one declaration per `Implementation` and picks `## Overloads` over
`## Declaration` purely on `function.implementations.len() > 1`. The defect was
in the descriptor: `func_utf8_encode.rs` registered **one** `Implementation`
returning `List OF Byte` for a member whose injected source defines **two**
`__encoding_utf8Encode` bodies. Overload selection never noticed, because it
happens over the source bodies in the monomorphizer; the registry row is what
`mfb man` reads, and it under-reported the member.

The census that establishes the scope was run over the injected sources rather
than over prose:

```
for d in src/codegen/builtins/*/; do
  grep -rhoE 'FUNC [A-Za-z_][A-Za-z0-9_]*\([^)]*\) AS [^"]*$' $d | sort -u |
  awk -F') AS ' '{sig=$1")"; if (sig in seen) { if (seen[sig]!=$2) print sig, seen[sig], $2 } else seen[sig]=$2}'
done
```

One hit in the whole tree: `FUNC __encoding_utf8Encode(value AS String)` with
`List OF Byte` AND `List OF Integer`. `utf8Encode` really is the language's only
return-type overload, and no member renders fewer declarations than it registers
(the new `every_member_renders_one_declaration_per_implementation` pin walks all
of them).

The original hypotheses, for the record:

1. **Most likely.** The renderer chooses `Overloads` vs. `Declaration` by
   comparing *parameter lists*. `utf8Encode`'s two forms have identical
   parameters (`value AS String`) and differ only in return type, so the
   renderer collapses them to one and prints the first form's return type.
2. The two forms are registered as one `Implementation` with a
   context-dependent return type, in which case there is only ever one
   signature to render and the fix is in the descriptor, not the renderer.

Either way the underlying cause is the same: return-type overloading is a
one-off in this language, and the man renderer's de-duplication was written when
"same parameters" implied "same function".

## Goal

- `mfb man encoding utf8Encode` renders an `Overloads` block with both forms.
- The requirement for an expected type is stated in the signature area.
- A renderer test fails if any member with more than one registered form renders
  a single `Declaration`.

### Non-goals (must NOT change)

- The return-type overload itself. It is a deliberate design and both forms have
  callers; this bug is about how it is *shown*.
- `TYPE_OVERLOAD_AMBIGUOUS`'s severity, code, or message — the diagnostic is
  good.
- `strings::toBytes`.
- The rendering of members that genuinely have one form.
- **Tempting wrong fix, forbidden:** deleting the `List OF Integer` overload so
  the single `Declaration` becomes true. It is used (`encoding::utf8Decode`
  accepts both, and the page's own second example round-trips through the
  integer form), and removing surface to fix a renderer is backwards.

## Blast Radius

- `src/cli/man.rs` — the renderer; fixed by this bug.
- `src/codegen/builtins/encoding/func_utf8_encode.rs` — the one member
  affected today, plus its prose.
- **Any other member whose overloads differ only in return type.**
  `grep -rn "TYPE_OVERLOAD_AMBIGUOUS\|return-type overload" src/codegen/builtins/`
  returns only `encoding`, so `utf8Encode` appears to be unique — but Phase 1
  must confirm by walking the registry rather than by grep, since a second such
  member would not necessarily mention the rule in prose.
- **Any member whose overloads differ only in a parameter the renderer also
  de-duplicates on.** If hypothesis 1 holds, the same collapse could hide an
  overload that differs by a `RES` marker or a `STATE` clause — which is
  bug-526's failure mode arriving from the other direction. Worth checking in
  the same pass.
- `encoding::utf8Decode` — takes both list types as *parameters*, so it is an
  ordinary overload and renders correctly. Unaffected, and the contrast that
  makes the asymmetry visible.

## Fix Design

Depends on which hypothesis Phase 1 confirms.

If the renderer de-duplicates on parameters (hypothesis 1): include the return
type in the key, so two forms differing only there render as two entries. That
is a small change and generalizes correctly — an Overloads block whose entries
differ only in their trailing `AS …` is exactly what a return-type overload
looks like.

If the descriptor registers one form (hypothesis 2): the descriptor is the
defect and must register both, which then also gives overload resolution two
rows to choose between rather than one row with special-cased typing. Larger,
and it must be proven not to change resolution.

In either case, add a line to the signature area for a return-type overload —
something the renderer emits when it detects that two entries share a parameter
list — so the reader learns the contextual-type requirement without reading to
the third paragraph.

Then the general pin: walk every registry member, and assert that the number of
signatures rendered equals the number of `Implementation` rows. That is the test
that would have caught this, and it also covers bug-526's class.

Rejected: adding an "Overloads" note by hand to `utf8Encode`'s prose. The page
already has the sentence; a second copy does not fix the block a reader
actually reads.

## Phases

### Phase 1 — failing test + root cause (no behavior change)

- [x] Land `spikes/api-review/bug-530-utf8encode-return-overload/` (done).
- [x] Read `src/cli/man.rs` and determine which hypothesis holds. Recorded above:
      **hypothesis 2**, the descriptor.
- [x] Renderer test `a_return_type_overload_renders_every_form` — RED before the
      fix ("a two-form member renders an Overloads block, not a Declaration").
- [x] Registry-wide census (above): `utf8Encode` is the only member whose source
      defines more forms than its descriptor declares.

### Phase 2 — the fix

- [x] `func_utf8_encode.rs` registers BOTH forms. Selection is unchanged:
      the two rows share a parameter list and `match_overload` takes the first
      row that unifies, so `resolve_call("encoding.utf8Encode", ["String"])` is
      still `List OF Byte` and the expected type still picks the form in the
      monomorphizer. `expected_arguments` is now the hand-authored `"String"`,
      because a multi-implementation member yields no per-position render and
      the argument-mismatch clause would otherwise read "no arguments".
- [x] The renderer emits the contextual-type requirement in the signature area
      whenever a set's forms share one parameter list
      (`is_return_type_overload_set`), so a second such member gets it for free.
- [x] No sibling to fix.

### Phase 3 — the general pin + validation

- [x] `every_member_renders_one_declaration_per_implementation` — one rendered
      declaration per registered form, for every member (>400 walked).
- [x] `no_member_defines_more_source_forms_than_its_descriptor_declares` — the
      pin that would have caught THIS bug; verified RED against the one-row
      descriptor ("the package source defines 2 `FUNC __encoding_utf8Encode(`
      form(s) but the descriptor declares 1").
- [x] `only_a_return_type_overload_gets_the_expected_type_note` — the positive
      pin: `utf8Decode` (a PARAMETER overload) and `csv::parse` (one form) must
      not grow the note.
- [x] `cargo test --release --no-fail-fast`: 4837 passed, 0 failed, cargo exit 0.
- [x] `scripts/test-accept.sh`: **1410 ran**, passed, exit 0 — no golden moved,
      as predicted: a `Body::Intrinsic` row injects no source and prose fields
      are `&'static str` no gate reads.
- [x] `scripts/man-run-examples.sh encoding --run`: 62 examples, 62 built, 62 ran,
      0 failed. `scripts/man-census.sh --memory-scope encoding`: 0 unclassified.

## Validation Plan

- Regression tests: the `utf8Encode` renderer assertion, and the general
  signature-count pin.
- Runtime proof: `spikes/api-review/bug-530-utf8encode-return-overload/`, whose
  commented-out line is the error the page should have predicted.
- Doc sync: `func_utf8_encode.rs` prose (shortened once the block carries the
  fact), `encoding/mod.rs`.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- Whether the language should keep return-type overloading for one member. The
  user's read is that this is "the sharpest language-level edge in the whole
  set" and that new code should prefer `strings::toBytes`. **Recommend keeping
  it and fixing the rendering** — it is used, and removing a public form is a
  much larger decision than this bug — but a follow-up asking whether one
  member justifies a whole resolution rule is legitimate.

## Summary

Small fix, valuable pin. The risk is picking the wrong root cause and patching
the descriptor when the renderer is at fault (or the reverse), which is why
Phase 1 reads `src/cli/man.rs` before anything is changed. The general
signature-count pin is the part worth keeping: it covers this bug and bug-526
with one assertion.
