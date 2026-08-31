# The `mfb man` content standard

What a builtin man page must contain, what it must never contain, and the
memory vocabulary it is allowed to use. Authored by plan-108-A; applied by
plan-108-B–E; certified by plan-108-F.

**Read this before editing any prose field on a registry descriptor.** The
compiler never reads these fields — they are `&'static str` — so no build,
test, or golden can catch a mistake in them. Rendering the page is the only
verification there is.

## 0. Where the content lives

`mfb man` renders builtin pages straight from the clean-room registry
descriptors in `src/codegen/builtins/**` (`src/cli/man.rs:1-15`). There is no
Markdown source for a builtin page; the retired `src/docs/man/builtins` tree
is archived under `planning/old_man/` and is **source material only, never
authority** (see §5).

The editable prose fields, and nothing else:

| Field | Where | Renders as |
|---|---|---|
| `MODULE_INTRO` | package `mod.rs` | the line under the package title |
| `MODULE_DESC` | package `mod.rs` | the package's `Description` section |
| `RegistryFunction.intro` | `func_*.rs` | the line under the function title |
| `RegistryFunction.desc` | `func_*.rs` | the function's `Description` section |
| `RegistryFunction.example` | `func_*.rs` | the function's `Examples` section |
| `Parameter.desc` | `func_*.rs` | the Description column of the Parameters table |
| `RegistryRecord` / prop / union-variant / enum-variant / `RegistryResource` `description` | `mod.rs` | the `mfb man <pkg> types` page |

Everything else on a page — Declaration, Parameters types, Errors, See also —
is **derived from the descriptors the compiler actually executes** and is
correct by construction. Do not try to "fix" a derived table by editing prose;
if a derived table is wrong, the descriptor is wrong, and that is a code bug.

The renderer omits every empty prose section (`man.rs:473,501,509`), which is
why `scripts/man-census.sh` measures rendered output rather than source. A
source grep is not a census: `fs` shows 37 `owns` hits in Rust module docs and
**0** rendered lines.

## 1. Who the page is for

*A man page is written for the MFBASIC developer at the terminal.*

The test: **if a sentence only matters to someone reading compiler source, it
does not belong on the page.** It belongs in `src/docs/spec/**` (if it is part
of the language contract) or `.ai/**` (if it is an implementation invariant).

## 2. What a page MUST contain

- **`intro` — REQUIRED**, one sentence, no trailing prose. It is the line a
  reader sees under the title and in the package's function table. It must be
  *distinct from the first line of `desc`*, not a copy of it: the intro says
  what the function is for, the description says how it behaves.
  (Decision, plan-108-A Phase 2 — previously an Open Decision. 477 of 489
  pages already comply; only `thread`'s 12 do not.)
- What the function does, in MFBASIC terms.
- Parameter semantics for **every** parameter: units, valid range, and what
  happens at zero/empty/negative. A bare parameter name with an empty
  description is an unfinished page — 234 of them exist today.
- Return semantics, including what comes back in the boring case.
- **Every raisable error and the condition that raises it**, consistent with
  the auto-derived Errors table.
- **Sharp edges, stated explicitly**: clamps vs. raises, Unicode scalar vs.
  grapheme, mutation vs. new value, ordering and stability guarantees.
  `strings::mid` raises where `left`/`right` clamp — a page that omits that
  distinction has failed, even if every sentence on it is true.
- **One example that was actually compiled and run** while the page was
  written. Where the environment genuinely cannot run it — a tty, a live
  endpoint, an audio device — compile it, and record the reason per function
  in the letter's ledger. An unrecorded skip is a gap.
- Cross-references to the sibling a developer would otherwise reach for by
  mistake.

## 3. What a page MUST NOT contain

- Registry / descriptor / lowering / monomorph / ABI vocabulary.
- Helper or mangled symbols: `#pkg_…`, `__pkg_…`, `$T` suffixes.
- `Body::`, `abi_inline`, NIR, IR, `.ncode`, regalloc, or any codegen
  mechanics. "Lowers to a single native AArch64 `and` instruction" is a
  compiler fact, not a developer fact.
- Rust implementation details, or the name of any Rust item.
- Plan numbers, bug numbers, or `[[path:symbol]]` citation markers. The last
  are old_man artifacts: strip them on port and never re-derive them.
- **Any C/Rust memory-management vocabulary — see §4. That is a hard,
  mechanically-checked ban, not a style preference.**

The canonical scope failure, and the reason §4 exists:

> "The returned Socket is a borrowed pointer — an alias into the list"
> (`tls::selectRead`)

Every banned word in one sentence, on a page a developer reads to learn how to
accept a connection.

## 4. The memory-vocabulary hard ban

MFBASIC is designed so a developer does not think about memory management on a
day-to-day basis. **The documentation is part of that design.** A page that
explains a handle in terms of borrows and pointers hands the reader a C/Rust
mental model the language deliberately does not require. The ban exists to keep
that model out of the product — not to police wording.

Accuracy is *not* the test. A sentence may be perfectly true and still banned:
precision of that kind belongs in `mfb spec`.

### 4.1 The permitted vocabulary — these four words, and no others

| Word | Means, in MFBASIC terms | Applies to |
|---|---|---|
| **value** | the thing a variable holds; two variables never share one | everything |
| **copy** | assigning or passing gives an independent value; changing one cannot change the other | everything copyable |
| **mutate** | changing a `MUT` binding, or a collection in place | `MUT` bindings, collections |
| **alias** | a second name for the SAME open handle — no copy is made, and closing through either name closes it | `RES` handles ONLY |

`alias` is the resource-only escape hatch. It is the only sanctioned way to say
"this is not a copy", and it may **not** be used for values.

### 4.2 The banned list

The single source is `scripts/man-census.sh`'s `BANNED_CORE`. Print it with:

```
./scripts/man-census.sh --banned-list
```

This document deliberately does not re-type the list, because a second copy is
a copy that drifts. As of 2026-08-30 it covers `borrow`/`borrowed`/`borrows`/
`borrowing`, `pointer(s)`, `ownership`/`owns`/`owned`/`owner(s)`,
`move semantics`/`moved into`/`moves the value`,
`consume`/`consumes`/`consumed`/`consuming`, `free the`/`free its`/`frees`/
`freed`, `heap`, `refcount`/`reference count`/`reference-counted`,
`garbage collect(ed)`, `lifetime(s)`, `dangling`, `deep copy`/`shallow copy`,
`by reference`/`by value`, `RAII`, `escape analysis`, `lexical drop`/
`drop the value`/`drop the handle`, and `allocate`/`allocates`/`allocated`/
`allocating`/`allocation(s)`/`allocator`.

Two matching rules that are part of the definition:

- **Whole-word only.** Unbounded `heap` matches `cheap` — it did, on five
  rendered lines. The script spells the boundaries `[^A-Za-z]` because BSD
  grep has no portable `\b`.
- **Bare `own` is NOT banned.** "fromString builds its own copy of text" is the
  prescribed replacement in §4.3; only `owns`/`owned`/`owner`/`ownership` are
  banned.

`consume`/`consumed` is banned deliberately, and it is the single most common
hit (69 rendered lines). It is Rust move-semantics vocabulary — but there is a
real, developer-visible event underneath it, so the rewrite must **keep the
fact and drop the word**: the handle is either left open (the caller still
closes it) or closed by the call (and cannot be used again). MFBASIC already
has the verb for both: `close`.

There are **three** cases, not two — the third was found by auditing the
parameter descriptions rather than assumed:

1. the handle stays open and the caller still closes it;
2. the call closes the handle and it cannot be used again;
3. **the call is handed the handle, may update it in place, and neither closes
   it nor takes it** — `http`'s `startRead` stream parameters
   ("Passed by reference; `pump` mutates its STATE … and neither consumes nor
   closes it"). Case 3 needs its own sentence because "stays open — the caller
   still closes it" loses the in-place update, and note that its current
   wording also leaks the separately banned `by reference`.

### 4.3 The rewrite table

Deleting a true statement to pass the grep is a **failure**, not a fix. Every
row below preserves the developer-visible fact.

| Banned today | Write instead |
|---|---|
| "Borrowed, not consumed." (parameter desc) | "The handle stays open — the caller still closes it." |
| "consumes its `RES` argument" / "the handle is consumed" | "closes the handle; it cannot be used again" |
| "the caller keeps ownership and must close it" | "the caller still closes it" |
| "the returned Socket is a borrowed pointer — an alias into the list" | "the returned socket is an alias of the one in the list — closing it closes that one" |
| "the list keeps ownership and still closes both" | "the list still closes both" |
| "close moves the value into the call" | "close takes the handle; it is closed afterwards and cannot be used again" |
| "closing the socket never frees the listener's context" | "closing the socket leaves the listener open" |
| "letting a Process drop at scope exit" / "closed by lexical drop" | "letting a Process go out of scope" / "closed when its binding leaves scope" |
| "so the caller owns the returned value unconditionally" | "so the caller always gets a value back" |
| "fromString builds a deep copy of text" | "fromString builds its own copy of text" |
| "costs no allocation" / "allocates a new buffer" | (cut it — allocation is not a developer-visible fact; if the point was speed, say "cheap" or give the complexity) |
| "a resource cannot be stored as a collection element" | (delete — it is also FALSE; `List OF RES tls::Socket` is valid, spec §15.6) |
| "Passed by reference; `finish` neither consumes nor closes it." | "The stream stays open — `finish` reads it and leaves it to you to close." |
| "Passed by reference; `pump` mutates its STATE (`raw`/`closed`/`err`) and neither consumes nor closes it." | "`pump` updates the stream in place and leaves it open." |
| "Borrowed and inspected for readiness only; no data is read." | "The handle stays open — this only tests readiness and reads no data." |
| "Borrowed — it remains open and usable after the call." | "The handle stays open and is usable after the call." |
| "Consumed by the call — the handle is moved and unusable afterward." | "Closed by this call; the handle cannot be used again." |
| "The value is consumed by the call." | "This call closes the handle." |

### 4.4 The two carve-outs

1. **Arithmetic borrow.** `datetime` normalization legitimately borrows a
   second ("a negative nanos value borrows a second") — 15 rendered lines, and
   every `borrow` in `datetime` is this sense. Not a memory claim; keep it.
   `--memory-scope` classifies these automatically and reports them separately;
   they are never counted as unclassified hits.
2. **`mfb spec` and `.ai/**` are untouched.** The spec's §14 memory model is
   the precise language contract and needs its precise words. The ban covers
   the `mfb man` surface only. **If a man page needs that much precision, that
   is the signal it is saying too much** — cut it and link `mfb man variable`.

### 4.5 Link, do not re-explain

There is exactly one page where the memory model is explained end to end:
**`mfb man variable`**. Any package page needing more than one sentence about
copies or handles links it rather than restating the model — and never
restates it in C/Rust terms. Twenty packages carried competing inline
explanations before this rule; that is what made them disagree with each other.

### 4.6 Enforcement

```
./scripts/man-census.sh --memory-scope <pkg>     # one package
./scripts/man-census.sh --memory-scope           # the whole surface
```

Exit status is 0 only when there are no unclassified hits. Every letter runs it
for its own packages before closing and records before/after counts in its
ledger; plan-108-F runs it whole-surface and certifies 0.

## 5. Accuracy

**Every behavioral claim is verified by running a program against the release
binary, or is read off the auto-derived descriptor tables.** Nothing else
counts as verification — least of all that the sentence sounds plausible or
that it was already there.

`planning/old_man/**` is source material, never authority. Behavior may have
changed since it was retired, and its `[[path:symbol]]` citations pre-date the
plan-102/103 code motion — sampled stale. Port the prose, re-verify the claim,
strip the citation.

The canonical accuracy failure:

> the `mfb man process` overview's claim that a resource "cannot be stored as
> a collection element"

It is false. `List OF RES tls::Socket` is valid (spec §15.6) and `tls::poll`'s
own signature takes one. The real rule is that the element type must be spelled
`RES` (a bare `List OF tls::Socket` is rejected with
`TYPE_RESOURCE_REQUIRES_RES`) and that a **record field** may never hold a
resource. A claim that has been sitting on a page for a year is not thereby
true.

A second live example, filed as bug-465: `mfb man tcp read` claims `tcp::read`
returns an empty list at EOF and ships a drain example looping on
`len(chunk) = 0`. Both transports actually **raise** `ErrConnectionClosed`. Fix
the page; do not "fix" the code to match the page.

## 6. Scope discipline

- **Prose fields only.** Several builtins files carry byte-significant MFBASIC
  bodies ("Body byte-significant … do not reformat", e.g.
  `src/codegen/builtins/collections/func_sort_by.rs:2`). Never touch a body, a
  descriptor type, an error table, or any non-prose code. Check every commit's
  `git diff` shows string-literal prose changes only.
- No renderer changes; no registry schema changes; no new fields.
- **No wording churn on accurate, in-scope prose.** This is an audit, not a
  rewrite. A page that passes §2, §3, §4 and §5 is left byte-for-byte alone.
- If the accuracy pass uncovers a real code bug — the doc says X, the code does
  Y, and Y is wrong — that is a found bug. Fix it or file it per AGENTS.md.
  Never paper over it in prose.
- `tests/cli_man_summary_plain.rs` pins some rendered summary text. If you
  correct a summary that test pins, update the pinned text in the same commit.

## 7. The four-step per-package workflow

1. **Accuracy pass** — check every prose claim against the implementation and
   against probe programs; fix or excise. Where the page is empty, port from
   old_man and re-verify each ported claim. Compile and run the example as part
   of writing it.
2. **Scope pass** — apply §3; rewrite internals-speak into developer terms or
   delete it. Run `scripts/man-census.sh --memory-scope <pkg>` and drive it to
   0 using §4.3. A claim that survives only in banned words is a claim that
   belongs in `mfb spec`: cut it and link `mfb man variable`. Record the
   before/after counts.
3. **Cross-model review** — one non-interactive Codex CLI run per package
   (a different vendor's model — stronger independence than another Claude
   tier):
   `codex exec -C <repo> -s workspace-write '<review prompt>'`.
   The prompt must instruct it to render `mfb man <pkg> --all` and
   `mfb man <pkg> types`; to **independently verify** every factual claim
   against the code and by compiling and running MFBASIC probe programs under
   `/tmp`; and to flag (a) inaccuracies with evidence, (b) leakage per §3 —
   including §4 quoted verbatim, with the instruction to flag any sentence that
   would teach a borrow/ownership mental model *even without using a banned
   word* — and (c) missing developer-critical information. Output is a
   structured findings list (function / claim / verdict / evidence) captured
   into the letter's ledger with `codex --version` and the model it reports.
   The reviewer never commits; `git status` must be clean of reviewer edits
   after each run.
4. **Apply** — triage on the main thread. Confirmed → fix. Rejected → record
   **with the disproving command**. Re-render the touched pages and re-run the
   census for the package.

## 8. Verification instruments

This is a documentation plan. **No compiler test gates apply** — prose fields
are strings the compiler never reads, so `artifact-gate`, the cargo suite and
`test-accept` can neither catch a doc error nor fail on one. Do not run them
for doc-only work.

What to run instead:

```
mfb man <pkg>                      # overview
mfb man <pkg> <func>               # one page
mfb man <pkg> --all                # every page in the package
mfb man <pkg> types                # records / unions / enums / resources
./scripts/man-census.sh [pkg...]           # fill state
./scripts/man-census.sh --functions [pkg]  # per-page rows
./scripts/man-census.sh --memory-scope [pkg]
```

Plus the release binary itself, for every probe program and every example.
