# bug-523: every built-in resource page omits that a `RES` may be a record field, a collection element, or handed to another thread

Last updated: 2026-09-05
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Footgun

Status: **FIXED** (`593d68965`) — every `mfb man <pkg> types` page states the
record-field and collection-element shapes once, and derives thread
transferability from the `sendable` bit; `mfb man variable` gained a runnable
example of each shape. Two pages that claimed a record field was forbidden are
corrected.
Regression Test: `cli::man::tests::every_resource_page_states_transferability_from_the_sendable_bit`

There are eleven built-in resource types. Rendering all eleven
(`mfb man <pkg> types` for `fs`, `tcp`, `udp`, `tls`, `process`, `audio`,
`canvas`) produces eleven descriptions with the same shape: what the handle is,
and that it closes when its binding goes out of scope. For example, the whole of
`mfb man fs types`:

> **fs::File** — An opaque handle to an open file, closed automatically when
> its binding goes out of scope.

That is the complete documented surface of the type. It omits three capabilities
a `RES` value actually has:

1. **It can be a field of a record.** `TYPE Holder … handle AS RES fs::File`
   compiles, and `h.handle` is usable. The construction form is the
   bracket-positional `Holder["log", f]`; the named-argument form does not
   typecheck for a record with a resource field — a sharp edge documented
   nowhere.
2. **It can be an element of a collection.** `List OF RES fs::File` works. The
   element must carry the `RES` marker; `List OF fs::File` is
   `TYPE_RESOURCE_REQUIRES_RES`. The diagnostic teaches this, which means the
   only documentation is the error you get after guessing wrong.
3. **It may be handed to another thread** — for six of the eleven. Which six is
   stated on two pages that contradict each other (bug-522), and on none of the
   type pages.

The single correct behavior a fix produces: a reader looking at any resource
type page can find out whether that handle may live in a record, live in a
collection, and cross a thread — without discovering it from a compiler error.

Two pages hint at a restriction without naming it: `audio::AudioInput` and
`audio::AudioOutput` are described as "opaque, **move-only** PCM stream". Nothing
on the page or in `mfb man` defines "move-only", and the term appears on no
other resource.

References:

- `mfb man fs types`, `mfb man tcp types`, `mfb man udp types`,
  `mfb man tls types`, `mfb man process types`, `mfb man audio types`,
  `mfb man canvas types` — the eleven descriptions
- `src/codegen/builtins/*/mod.rs` — the `RegistryResource.description` fields
- `mfb man variable`, `mfb spec` §14 — where the ownership model is defined
- `tests/rt-behavior/resources/record-res-field-export-rt/` — the fixture that
  proves capability 1, including the named-argument limitation
- Spike: `spikes/api-review/bug-523-res-shapes-undocumented/`
- Related: bug-522 (the thread-transfer set is stated twice, incompatibly)

## Failing Reproduction

```
for p in fs tcp udp tls process audio canvas; do ./target/release/mfb man $p types; done
```

- Observed: eleven one-sentence descriptions. `grep`ing all eleven for
  `record`, `List`, `collection`, `thread` or `transfer` matches nothing.

Then the capabilities that are not described:

```
./target/release/mfb build spikes/api-review/bug-523-res-shapes-undocumented
./spikes/api-review/bug-523-res-shapes-undocumented/build/mfb_project.out
```

```
1. RES as a record field: works, and nothing in `mfb man fs types` says so
2. RES in a collection: 2 handles in one List
3. RES across a thread: see the bug-522 spike -- documented twice, incompatibly
```

- Expected: each resource page states its three shapes, or the package page
  does and the type page links it.

Contrast cases, documented well today, that show the standard to meet:

- `canvas::Image` and `canvas::Font` *do* carry an extra clause — "A scene names
  one through a `canvas::ImageRef`, never directly, so destroying an image a
  scene still draws is safe." That is exactly the kind of shape statement the
  other nine lack.
- `mfb man variable` and `mfb spec` §14 define the copy/alias model properly.
  This bug is not asking to restate that model on eleven pages; it is asking
  each page to say which of its shapes are available and to link the model.

| Environment | arch/config | Result |
| --- | --- | --- |
| macOS | aarch64, release | fails ✗ (prose defect; target-independent) |

## Root Cause

Not a code defect. `RegistryResource` has exactly one prose field,
`description`, and every package filled it with a one-line gloss. There is no
field for "may this be a collection element", no field for "may this be a
record field", and the `sendable` bit — which *is* structured data — is not
surfaced in the rendered type page at all.

So the renderer has the thread-transfer answer available and does not print it,
and it has no answer at all for the other two.

The `audio` "move-only" wording is a leftover from a period when the term had a
defined meaning in the docs; it survives as an undefined term because prose
fields are `&'static str` the compiler never reads.

## Goal

- Every `mfb man <pkg> types` resource entry states, or links, whether the
  handle may be a record field, a collection element, and a thread transfer.
- The thread-transfer answer on a type page is derived from the registry
  `sendable` bit, not restated in prose, so it cannot drift (bug-522's
  failure mode).
- "move-only" is either defined where it is used or removed.
- The `RES`-marker requirement on collection elements
  (`List OF RES T`, not `List OF T`) is stated somewhere a reader finds it
  before hitting `TYPE_RESOURCE_REQUIRES_RES`.

### Non-goals (must NOT change)

- Any resource's behavior, `sendable` bit, or close op.
- The ownership model itself. `mfb man variable` and `mfb spec` §14 own it;
  this bug adds pointers, not a second explanation.
- The banned memory vocabulary. Per `.ai/man-content.md`, the new prose may use
  only *copy*, *mutate*, *value*, and *alias* (the last for a `RES` handle);
  "move-only", if kept, needs a definition in permitted words or it goes.
- Per-page essays. Eleven pages each growing three paragraphs is worse than
  what exists; the fix should be short and mostly rendered.
- **Tempting wrong fix, forbidden:** restating the transferable set as prose on
  each type page. That is precisely how bug-522 happened — eleven more copies
  of a fact that drifts.

## Blast Radius

The eleven `RegistryResource` rows, from `grep -rn "RegistryResource {"
src/codegen/builtins/`:

- `fs/mod.rs:165` `fs::File` — sendable
- `tcp/mod.rs:180` `tcp::Socket`, `tcp/mod.rs:199` `tcp::Listener` — sendable
- `udp/mod.rs:171` `udp::Socket` — sendable
- `tls/mod.rs:177` `tls::Socket`, `tls/mod.rs:239` `tls::Listener` — sendable
- `process/mod.rs:228` `process::Process` — not sendable
- `audio/mod.rs:408` `audio::AudioInput`, `audio/mod.rs:427` `audio::AudioOutput`
  — not sendable, and both carry the undefined "move-only"
- `canvas/mod.rs:965` `canvas::Image`, `canvas/mod.rs:986` `canvas::Font` —
  not sendable

Other affected surfaces:

- `src/cli/man.rs` — the renderer; it must learn to emit the derived
  transferability line.
- `src/codegen/registry/mod.rs` — `RegistryResource`; may gain a field.
- **User-declared resources** (`RESOURCE … THREAD_SENDABLE`) — the same three
  questions apply and are answered in `mfb man variable` /
  `.ai/resources-packages.md`. Out of scope here (this bug is the built-in type
  pages), but the wording chosen should be reusable.
- `mfb man variable`, `mfb spec` §14 — the link targets; check in Phase 1 that
  they actually cover the record-field and collection-element shapes, rather
  than assuming they do.

## Fix Design

Split by whether the fact is structured or prose.

**Derived (no drift possible).** Add a line to the renderer's resource entry,
generated from `RegistryResource.sendable`:

> May be handed to another thread with `thread::transfer`. *(or)* Stays on the
> thread that opened it.

This is the same bit `is_builtin_sendable_resource_type` reads, so it cannot
disagree with the compiler — which is the property bug-522 lacks. Where the bit
is `false`, the reason is per-resource and does belong in prose; add an optional
`unsendable_reason` field carrying the sentence each row already has as a Rust
comment (`process/mod.rs:229`, `audio/mod.rs:406`, `canvas/mod.rs:962`).

**Prose, written once, linked eleven times.** The record-field and
collection-element shapes are uniform across all eleven resources, so they
belong in one place. Two candidates: a new short section in `mfb man variable`
(the narrative topic that already owns the model), or the resource section of
`mfb spec` §14. **Recommend `mfb man variable`**, because the type pages can
link it and it is where a reader already goes for "how do values work here".
The section must cover the two sharp edges the spike found: `List OF RES T`
rather than `List OF T`, and the bracket-positional construction form for a
record with a resource field.

Then delete or define "move-only" on the two audio rows.

Rejected: adding three booleans to `RegistryResource`. Record-field and
collection-element are not per-resource facts — they are true for all eleven —
so per-row storage invites eleven chances to get a uniform answer wrong.

Rejected: leaving it to `mfb man variable` alone with no type-page change. The
reader's entry point is the type page; a fact that is only reachable from
elsewhere is the state we are already in.

## Phases

### Phase 1 — audit (no behavior change)

- [ ] Land `spikes/api-review/bug-523-res-shapes-undocumented/` (done).
- [ ] Read `mfb man variable` and `mfb spec` §14 and record what they *already*
      say about resources in records and collections. The fix depends on this
      and must not assume it.
- [ ] Confirm the record-field named-argument limitation is real and general,
      not an artefact of the one fixture
      (`tests/rt-behavior/resources/record-res-field-export-rt/` says it also
      affects resource-free records, predating plan-114). Record the verdict —
      if it is general, it is a *separate* gap and this bug only documents it.
- [ ] Confirm all eleven `sendable` bits against the tables in bug-522.

Acceptance: the existing coverage in `variable`/§14 is written down; the
named-argument verdict is recorded.
Commit: —

### Phase 2 — the derived line

- [ ] Add `unsendable_reason` to `RegistryResource`, populated for the five
      non-sendable rows from the reasons already in their comments.
- [ ] Render the transferability line in `src/cli/man.rs` from `sendable`.

Acceptance: all eleven `mfb man <pkg> types` pages state transferability;
the text for the six sendable ones agrees with bug-522's table by construction.
Commit: —

### Phase 3 — the shared prose

- [ ] Add the record-field / collection-element section to `mfb man variable`,
      with a runnable example for each, using only permitted vocabulary.
- [ ] Link it from the resource entry in the type-page renderer.
- [ ] Define or delete "move-only" on the two audio rows.

Acceptance: `scripts/man-run-examples.sh` compiles and runs the new examples;
`scripts/man-census.sh --memory-scope` reports 0 unclassified hits.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --no-fail-fast` — the renderer change touches `src/cli/man.rs`
      and `RegistryResource`, so this is not doc-only and does not qualify for a
      scoped run.
- [ ] `cargo check --all-targets` at the end.
- [ ] Re-render all eleven pages and read them.

Acceptance: full suite green; every resource page answers the three questions.
Commit: —

## Validation Plan

- Regression test: the new `mfb man variable` examples, run by
  `scripts/man-run-examples.sh`; a renderer test asserting the transferability
  line matches `sendable` for all eleven.
- Runtime proof: `spikes/api-review/bug-523-res-shapes-undocumented/`.
- Doc sync: eleven `RegistryResource` rows, `src/cli/man.rs`,
  `src/docs/man/variable/`, and `mfb spec` §14 if Phase 1 finds it stale.
- Full suite: `cargo test --no-fail-fast` + `scripts/test-accept.sh`.

## Open Decisions

- `mfb man variable` vs. a new `mfb man resources` topic as the home for the
  shared prose. **Recommend `variable`** — it already owns the value model, and
  a new topic splits the answer across two pages.
- Whether to also fix the *user-declared* resource pages. **Recommend a
  follow-up**: the wording lands here first and is reused, rather than
  designing for both at once.

## Summary

The risk is in scope discipline, not correctness: it is easy to turn this into
a rewrite of the value model across eleven pages. The design deliberately
splits it — one derived line the registry cannot contradict, and one shared
prose section linked eleven times — so the only per-resource content added is
the five reasons that already exist as Rust comments.

## Resolution (2026-09-05, `593d68965`)

**Which side was wrong: the docs — and two of them were worse than silent.**
`process`'s package description said a handle "cannot be a field of a record",
and `mfb spec stdlib transports` said "No transport handle may be stored in a
record field". Both are false. `mfb spec language resource-management` §15.4
permits a `RES` field and uses one as its own example
(`TYPE Holder { handle AS RES fs::File STATE Cursor }`);
`tests/rt-behavior/resources/record-res-field-export-rt` carries such a record
across a package boundary; and a program built for this fix puts a
`tcp::Listener` in a record field and reads its bound port back through it. Both
statements are corrected.

### Phase 1 verdicts

- **What `mfb man variable` / §14 already covered.** They own the copy/alias
  model and cover it well — `RES` bindings, aliasing through a parameter,
  closing through either name, the automatic scope-exit close. Neither said
  anything about a handle *in a record* or *in a collection*. The new section
  fills exactly that gap and adds no second explanation of the model.
- **The record-field named-argument limitation is not a resource fact.**
  `Plain[label: "x", n: 3]` is a parse error for a resource-free record too:
  MFBASIC record literals are positional, full stop. So it is not a separate gap
  to file, and the spike's comment claiming a resource-specific limitation is
  corrected.
- **All 11 `sendable` bits** re-checked against bug-522's tables; they match.

### The design, and why it is split

- **Uniform → written once.** The record-field and collection-element shapes are
  true of all eleven, so they are one preamble under `## Resources`, rendered on
  every types page, together with the `List OF RES <Type>` marker rule — which
  until now was taught only by `TYPE_RESOURCE_REQUIRES_RES`, i.e. only after
  guessing wrong. It links `mfb man variable` rather than restating the model.
  Eleven pages did not each grow three paragraphs, which was the report's stated
  risk.
- **Structured → derived.** Transferability is rendered from
  `RegistryResource.sendable`, the same bit the compiler reads, so it cannot
  disagree with it. That is the property bug-522 lacked. The *reason* a handle
  stays put is per-resource, so it is prose — a new `unsendable_reason` field
  populated for the five non-sendable rows from the sentences already sitting
  beside them as Rust comments.

### "move-only"

Deleted from both `audio` descriptions rather than defined. It was an undefined
term appearing on no other resource, and the fact under it — the handle is not
copied — is now stated once, for all eleven, in the preamble, in permitted
vocabulary.

### Validation

`man-census.sh --memory-scope` reports 0 unclassified hits; the new prose uses
only *alias*, *copy* and *value*. Both new `mfb man variable` examples were
compiled and run: the record-field one writes through `log.handle` and the file's
contents prove the write landed; the collection one holds two handles.

Note `scripts/man-run-examples.sh` reaches package pages only, not guide topics,
so `variable`'s examples were verified by hand — a gap that belongs to bug-472
("man examples are never compiled"), not here, and is recorded rather than
papered over.

Pin: `every_resource_page_states_transferability_from_the_sendable_bit` renders
each types page and asserts the line against the bit for all eleven, requires an
`unsendable_reason` on every non-sendable row (so a new resource cannot render
the bare fact by omission), and asserts the census is 11.

### Open Decisions — resolved

- **`mfb man variable` vs. a new `mfb man resources` topic** → `variable`, as
  recommended. It already owns the value model, and the type pages link it.
- **User-declared resources** → still a follow-up, as recommended. The wording
  lands here first; a `RESOURCE … THREAD_SENDABLE` declaration renders no type
  page of its own.
