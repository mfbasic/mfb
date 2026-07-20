# bug-336: the man pipeline cannot see three packages, ships two renderings, and guards only half the corpus

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (documentation)

Status: Fixed (2026-07-19)
Regression Test:
- `docs::man::tests::man_citations_resolve` (S12) — every `[[path:Symbol]]` in
  the corpus must name a file that exists and a symbol present in it. It caught
  four dangling citations on its first run, all fallout from bug-321 moving
  functions into `linux_common`.
- `builtins::tests::documented_builtins` (S3) — now reads `.md` as well as
  `.txt` and skips `package`/`types`, so the two `call_param_names` guards cover
  all 460 function pages instead of the 225 legacy ones.

A cluster of *structural* defects in the `mfb man` pipeline — the corpus layout, the
generator scripts, and the guard tests — found during the cleanup review. None of these
is a wrong statement about the compiler (those live in bug-337); each is a hole in the
machinery that is supposed to keep man pages honest, and each is silent. The pipeline
skips three packages without a word, the guard tests assert against a subset that shrinks
every time a page is migrated, and the two driver scripts disagree about where a
package's source lives.

The single correct behavior a fix produces: **every user-visible built-in surface has
exactly one page, in one format, that `scripts/update_man.sh` can regenerate and a CI
check can verify.**

References:

- `AGENTS.md:53-60` — designates `.ai/man_*_template.md` as the templates and
  `scripts/update_man.sh` / `scripts/update_man_package.sh` as the authoritative home
  for the authoring rules. The divergence in S8 is therefore load-bearing, not cosmetic.
- `.ai/man_template.md`, `.ai/man_type_template.md`, `.ai/man_package_template.md`
- Found during the cleanup review (Agent 20 man-page sweep; Agent 17 builtins sweep;
  Agent 18 MFBASIC-stdlib sweep), base `25c38ba1`.
- Content drift (pages that contradict the implementation) is filed separately as
  bug-337; the two are independent and can land in either order, except that S1's
  migration should follow S2 (see Fix Design).

## Current State

Counts below were re-measured against the worktree at base `25c38ba1`.

| Measurement | Value |
| --- | --- |
| Legacy plain-text `.txt` pages | 226, all under `src/docs/man/builtins/`, across 13 packages |
| Migrated Markdown `.md` pages | 259 total; 233 under `builtins/` (24 `package.md` + 209 function/type pages) |
| `[[provenance]]` citations in `.txt` pages | **0** |
| `[[provenance]]` citations in `.md` pages | 2084 occurrences / 530 unique |
| Packages `scripts/list_functions.py` reports | 21 — `collections`, `filters`, `testing`, `errorCode` are absent |
| Non-package `.md` pages the `mod.rs` guard tests examine | **0 of 209** |

Both renderings ship in the same binary today:

```
$ mfb man math round          $ mfb man fs open
NAME                          OPEN
  round - nearest intege…     ════
                              
SYNOPSIS                      Open a file with an explicitly named access mode…
  math::round(value AS F…     
                              Synopsis
PACKAGE                       ────────
  math                        
                                fs::open(path AS String, mode AS String) AS File
```

## Items

### S1 — 226 legacy `.txt` pages across 13 packages; two renderings in one binary

- `src/docs/man/mod.rs:185` (`is_markdown_page`) selects the renderer per page by
  sniffing content, so the format is a per-file property with no migration deadline.
- 13 packages are still `.txt`: `collections` (38), `filters` (6), `http` (4), `io`
  (15), `json` (5), `math` (35), `money` (3), `net` (23), `strings` (37), `term` (17),
  `testing` (12), `thread` (12), `vector` (19). Every one of those directories also
  carries a migrated `package.md`, so 13 packages render their overview in the boxed
  Markdown style and their function pages in the flat legacy style.
- The legacy pages carry **zero** `[[provenance]]` citations. Migration is therefore
  not a formatting exercise: it is the only mechanism by which those 226 pages acquire
  traceability, and it is the precondition for the S9 CI check to cover them.
- `scripts/update_man.sh:105-107` already knows the intended end state ("Write the page
  as Markdown … If a legacy plain-text `<dir>/${fname}.txt` exists, delete it (git rm)").
  The migration has simply never been run to completion.

### S2 — `collections`, `filters`, and `testing` are invisible to `list_functions.py`, so `update_man.sh` silently skips all 56 of their pages

- `scripts/list_functions.py:132-136` collects the package's function names by scanning
  the **body** of `is_<stem>_call` / `is_<stem>_function` for uppercase Rust identifiers
  and resolving them through `build_const_map` (`:41-50`), which matches only
  `const NAME: &str = "…";`. Line `:147` (`if functions or constants:`) then drops any
  package that yielded nothing — with no diagnostic.
- `collections`: `src/builtins/collections.rs:92-95` (`is_collections_call`) delegates to
  `is_collections_function` / `is_native_member`, whose bodies (`:79-87`) reference the
  slice constants `FUNCTIONS` (`:20-39`) and `NATIVE_MEMBERS` (`:47-68`). Neither is a
  `&str` const, so `resolve()` returns an empty list. All 39 members vanish.
- `testing`: there is no `is_testing_call` at all — the predicate is
  `src/builtins/testing.rs:44` (`is_expect_call`), the one module in the tree using a
  private naming convention. `idents_in_fn` finds no matching function and returns `[]`.
- `filters`: no `src/builtins/filters.rs` exists, and `filters` appears nowhere in
  `src/builtins/mod.rs`. It is a man-corpus-only package name; its 6 pages document
  predicates that actually live in `general.rs`.
- Confirmed by running it: `python3 scripts/list_functions.py` prints 21 package
  headers, and `collections`, `filters`, `testing`, and `errorCode` are not among them.
- Consequence: `scripts/update_man.sh:55-58` builds its entire work list from that
  script's output, so `./scripts/update_man.sh collections` exits 1 ("No built-in
  functions or types found for package 'collections'"). **The three packages the tool
  cannot see are exactly the three that are still 100% legacy `.txt`.** That is not a
  coincidence — it is cause and effect.

### S3 — the man-corpus guard tests filter on `.txt`, so 209 `.md` pages are unchecked, and the size assertion hides it

- `src/builtins/mod.rs:542-567` (`documented_builtins`): line `:557-559` skips any page
  whose extension is not `txt`, and line `:566` asserts `names.len() > 100`.
- The two guards that consume it — `no_named_argument_alias_repeats_across_positions`
  (`:569-606`) and `overloaded_param_name_tables_are_well_formed` (`:608-…`) — therefore
  validate `call_param_names` for the 226 legacy pages and for **none** of the 209
  migrated ones.
- The hole is invisible because 226 > 100: the assertion passes today and would keep
  passing until fewer than 100 `.txt` pages remain. Completing S1's migration would
  eventually turn this into a hard failure — but only after the guard had already
  stopped covering most of the corpus. The metric is inverted: progress on migration
  degrades the guard.
- Fix: drop the extension filter (accept `txt` and `md`), skip the `package` stem, and
  replace the `> 100` floor with an exact-count or per-package assertion.

### S4 — `errorCode` has no man page at all, while nine shipped pages teach `IMPORT errorCode`

- `src/docs/man/mod.rs:29-61` (`PACKAGE_ORDER`) has no `errorCode` row, and no
  `src/docs/man/builtins/errorCode/` directory exists — yet `src/builtins/mod.rs:39`
  admits `"errorCode"` as a built-in import and `src/builtins/errorcode.rs:15` includes a
  build-time-generated constant table derived from
  `src/docs/spec/diagnostics/02_error-codes.md`.
- Observed:

  ```
  $ mfb man errorCode
  error: unknown package `errorCode`
  Available packages: tour, types, flow, errors, link, general, collections, …
  ```

- Nine man pages already write `IMPORT errorCode` in example code:
  `builtins/testing/expectTrap.txt`, `builtins/fs/openWithin.md`, `errors/package.md`,
  `tour/package.md`, and all five `tour/0{1..5}_*.md` tour pages. A reader who follows
  the tour and then runs `mfb man errorCode` is told the package does not exist.
- Root cause is S2's shape: `src/builtins/errorcode.rs:23` exports
  `is_errorcode_constant`, not `is_errorcode_call`, so `list_functions.py` cannot see it
  either.

### S5 — `general` ships zero function pages for 18 always-in-scope builtins

- `src/docs/man/builtins/general/` contains only `package.md`.
- `list_functions.py` *does* see the package and reports 18 functions: `error`, `len`,
  `typeName`, `toString`, `toInt`, `toFloat`, `toFixed`, `toByte`, `toMoney`,
  `toScalar`, `isNumeric`, `isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`,
  `isEmpty`, `isNotEmpty` (`src/builtins/general.rs:4-30`, `:199-300`).
- Observed: `mfb man general` ends at the Errors table with no `FUNCTIONS` section
  (contrast `mfb man money`, which appends one). These are the only builtins callable
  with no `IMPORT`, so they are the most likely thing a new user looks up.
- Six cross-references in `src/docs/man/unicode/package.md:41-43` point at `general`
  function pages that do not exist; two of them (`find`, `mid`) are no longer `general`
  functions at all — they moved to `collections`/`strings`
  (`src/builtins/collections.rs:65-67`).

### S6 — `http` documents 4 of 14 functions and none of its 4 types; `strings::toBytes` has no page but is cross-referenced from 9

- `http`: `list_functions.py` reports 14 functions (`read`, `write`, `server`,
  `serverSSL`, `handleRequest`, `route`, `responseDefault`, `ok`, `status`, `json`,
  `withHeader`, `bytes`, `respondFile`, `respondPath`) and 4 types (`Response`,
  `Request`, `RequestPart`, `Route`). `src/docs/man/builtins/http/` holds four legacy
  pages — `bytes.txt`, `respondFile.txt`, `respondPath.txt`, `withHeader.txt` — plus
  `package.md`. Ten functions and all four types are undocumented; `package.md`'s
  non-template `## Server` section (see S7) is doing the missing pages' job.
- `strings::toBytes`: declared at `src/builtins/strings.rs:33` and resolved at `:137`
  / `:158` (`String → List OF Byte`). No `src/docs/man/builtins/strings/toBytes.*`
  exists — it is the one gap in an otherwise complete 37-page directory. It is
  referenced 14 times across 9 pages (`crypto/aes256GcmSeal.md`,
  `crypto/chacha20Poly1305Seal.md`, `crypto/ed25519Verify.md`, `encoding/package.md`,
  `encoding/utf8Encode.md`, `encoding/utf8EncodeBytes.md`, `encoding/utf8EncodeInts.md`,
  `tls/write.md`, `http/bytes.txt`) — it is the `String` ↔ `List OF Byte` seam the whole
  `encoding` package is built on.

### S7 — type documentation lives in three different homes; the type-page loop cannot see Rust-declared types

- Only five `types` pages exist: `crypto/types.md`, `datetime/types.md`,
  `audio/types.md`, `vector/types.md`, and the legacy `json/types.txt`.
- Visible-but-missing: `list_functions.py` reports exported record types for `http`
  (4) and `net` (1); neither has a `types` page, so `scripts/update_man.sh:161-208`
  would generate one on the next run and nobody has run it.
- Structurally invisible: `list_functions.py:155-160` scans only `*_package.mfb` for
  `EXPORT TYPE`, so record types declared in Rust are unreachable by the type-page loop
  no matter how often it runs. Two packages declare record types with fields there —
  `src/builtins/net.rs:88-95` (`Address`, `Datagram`, `DatagramText`) and
  `src/builtins/term.rs:68-74` (`TermColor`, `TermSize`).
- Those two are documented in two *different* wrong places: `net`'s fields are prose
  inside `src/docs/man/builtins/net/package.md:35-40`, while `term`'s two records are
  hoisted out of the package entirely into the global
  `src/docs/man/types/package.md:105-111`. `.ai/man_type_template.md` mandates one
  consolidated page per package.
- (`csv`, `fs`, `thread`, `tls` declare only opaque resource handle types with no
  fields — `src/builtins/fs.rs:3`, `thread.rs:3-4`, `tls.rs:12-13` — so they are
  correctly out of scope for a `types` page.)

### S8 — `regex/language.md` is a topic page inside a function directory, and the build has no way to say so

- `build.rs:278-291` collects every non-index page in a package directory into
  `DocPackage::pages`; the renderer then lists them all under `FUNCTIONS`.
- Observed:

  ```
  $ mfb man regex
  FUNCTIONS
    find               Locate the first regular-expression match…
    findAll            Locate every non-overlapping regular-expression match…
    language           How to write patterns for the regex package.
    match              Test whether a regular expression matches anywhere…
    replace            Replace every non-overlapping regular-expression match…
  ```

- `language` is not a function. There is no mechanism to express "topic page inside a
  package", which is also why `flow/`, `types/`, and `tour/` exist as free-standing
  pseudo-packages whose pages match no template at all.

### S9 — package-overview template conformance: 5 of 24 add forbidden sections, 3 omit the mandatory Errors section

- `.ai/man_package_template.md` permits exactly: title, summary, `## Synopsis`,
  `[## Imports]`, `## Description`, `## Errors` — and
  `scripts/update_man_package.sh:76,83-84` states it as a rule ("The page ends after
  Errors. Do not add Examples, See also, or a function list").
- Forbidden extra sections: `audio` (`Platform availability`, `Members`, `See also`),
  `crypto` (`Security notes`, `See also`), `http` (`Server`), `money` (`Functions`,
  `See Also`), `testing` (`See Also`).
- Missing the mandatory `## Errors`: `audio`, `money`, `testing`.
- `money` is the visible failure. `src/docs/man/builtins/money/package.md:44-48`
  hand-writes a `## Functions` list, and `mfb man money` then appends the generated one,
  so the three functions print twice in a single page — exactly what the script's rule
  exists to prevent.
- Per-function `.md` pages are otherwise near-perfect; the only structural deviation is
  S8's `regex/language.md`.

### S10 — 51 legacy pages have no PARAMETERS section and 20 have no ERRORS section — concentrated in the packages the pipeline cannot see

- Measured across all 226 `.txt` pages: 51 lack `PARAMETERS`, 20 lack `ERRORS`.
- Distribution of the 51: `collections` 38 of 38, `filters` 4 of 6, `http` 4 of 4,
  `net` 4 of 23, `json` 1 of 5.
- `collections` and `filters` together account for 42 of the 51, and both are
  S2-invisible: the pages the tool cannot refresh are the pages that never got the
  sections the template has required since. This item is largely *subsumed* by S1+S2 —
  it should resolve as a side effect and is listed here as the acceptance signal, not as
  separate work. The `http`, `net`, and `json` remainder is genuine independent debt.

### S11 — the two driver scripts disagree about where a package's source lives, and duplicate their shared rules three ways

- `scripts/update_man.sh:92` tells the authoring agent, unconditionally: "Read
  `src/builtins/${module}.rs` to understand the function's signature, overloads,
  parameter types, return type, and error behavior." It never mentions
  `src/builtins/<pkg>_package.mfb`.
- For the 13 packages implemented in MFBASIC — `audio`, `collections`, `crypto`, `csv`,
  `datetime`, `encoding`, `http`, `json`, `money`, `net`, `regex`, `strings`, `vector` —
  the `.rs` file is a registration shim (name constants, `is_*_call`, return-type
  tables) and the actual signatures, defaults, and error paths live in the `.mfb`. An
  agent following the instruction literally documents the shim.
- `scripts/update_man_package.sh:34-43` does it correctly: it special-cases `filters`
  → `general.rs`, guards each candidate with `[[ -f ]]`, and **adds**
  `src/builtins/${pkg}_package.mfb` when present. The two scripts are the authoritative
  rules home per `AGENTS.md:58-60`, and they disagree.
- They also disagree about which packages exist. `update_man_package.sh:19-21`
  enumerates the man directory, so it will happily generate a `filters/package.md`;
  `update_man.sh` derives its list from `list_functions.py`, which has never emitted a
  `filters::` function, so it can never generate a `filters` function page.
- Shared prose is copy-pasted rather than factored: 19 lines are byte-identical between
  the two prompts, the ~9-line Provenance paragraph appears three times
  (`update_man.sh:125-130` and `:200-206`, `update_man_package.sh:90-96`), the
  "renderer's supported Markdown subset" list appears three times, and the ~17-line
  Errors-table rules block appears twice (`update_man.sh:132-148`,
  `update_man_package.sh:98-115`). A rule fixed in one place stays broken in the others.

### S12 — nothing resolves `[[provenance]]` citations, so a citation can name a symbol that does not exist

- 530 unique citations across the `.md` corpus and no checker. `update_man.sh:127-128`
  instructs "Grep-confirm the symbol exists before citing", which is an instruction to a
  language model, not a gate.
- A ~30-line resolver run over the corpus finds exactly one dangling **symbol**
  citation: `src/docs/man/builtins/net/package.md:40` cites
  `[[src/builtins/net.rs:record_fields_for_type]]`; the function is `builtin_type_fields`
  at `src/builtins/net.rs:88`. (Filed as a content item in bug-337; recorded here because
  the missing gate is the structural cause.)
- The same scan finds six citations in the malformed *file-only* form with no
  `:Symbol` — `link/package.md` ×2, `crypto/package.md`, `audio/types.md`,
  `thread/package.md`, and three `bits/*.md` pages citing another man page rather than
  source. All name files that exist, so they are template deviations rather than
  dangling, but they are undetectable today for the same reason.
- Three apparent hits are false positives the checker must exclude: `[[:alpha:]]` POSIX
  classes in `regex/language.md` (×2) and a `[["a", "b"]]` list literal in
  `csv/stringify.md`.
- A checker of this shape would also have caught bug-337's tls error code, where the
  citation `[[error_constants.rs:ERR_TIMEOUT_CODE]]` resolves but names a constant whose
  value contradicts the number printed beside it.

## Outcome (2026-07-19)

Every item is closed. The corpus is **484 Markdown pages and zero `.txt`**; all
31 packages and all 460 function pages render.

### What the migration actually found

S1 was framed as a formatting exercise. It was not — the 225 legacy pages were
substantially **wrong about the compiler**, and the migration is worth reading as
a correctness audit. A representative sample of what came out, each verified
against source and, where possible, by compiling and running a probe:

- **Documented functions that do not exist.** `thread`'s `t.result` accessor was
  taught on seven pages; it was removed from the language and a `.result` member
  access is rejected with `TYPE_THREAD_RESULT_REMOVED`. `testing`'s two trap
  pages built their only examples on `parseInt`, which exists nowhere in the
  tree. (This is the same shape as bug-337-D4's `net::toAddress`.)
- **A rule stated backwards.** `testing/expectTrap` claimed an infallible or
  inline-compiled call "is rejected at compile time; wrap it in a FUNC/SUB". The
  inline-builtin restriction was retired by plan-26-C and a fixture exists
  proving `expectNTrap(len(xs))` compiles. The prescribed workaround was
  superstition.
- **Missing and invented error codes.** `math`'s `log`/`log10`/`tan` pages
  invented an `ErrFloatInf` their kernels do not declare (confirmed: those calls
  return finite values). `money::round` was missing a real `ErrOverflow`.
  `term::on`, `terminalSize`, and both colour getters were all marked "No
  errors" while each can raise. Five `io` pages omitted `ErrInvalidContext`.
- **Behaviour documented as the opposite of the implementation.** `term::clear`
  was said to fill with the current background colour and to leave the cursor
  alone; it zero-fills and homes the cursor. `io::readLine` was said not to
  alter the terminal mode and to echo; it clears `ECHO`, and `io::input` is the
  one that echoes. `io::flush` was said to request a host sync; it is
  deliberately drain-only.
- **Pervasively non-compiling examples.** Nearly every legacy page's example
  failed to build — top-level statements with no enclosing `SUB`, missing
  `IMPORT`, `collections::` members called bare after they moved namespace,
  `Fixed` examples without the `F` suffix, `END FOR` instead of `NEXT`.

### Claims in this document that were wrong

- **S5 is stale.** `general/` does not contain "only `package.md`" — all 18
  function pages already existed as Markdown. What was real was `filters`: a
  man-corpus-only pseudo-package whose six pages duplicated `general`
  predicates. It is deleted, and `general` keeps the canonical pages.
- **S10's count.** 51 pages lacking `PARAMETERS` was right, but it resolves as a
  side effect of S1 exactly as predicted; every one of the 460 function pages now
  has both `Parameters` and `Errors`.
- **S12's "1 dangling citation".** The resolver found **five** — the one named
  here plus four `linux_x86_64` paths that bug-321 invalidated after this
  document was written. Which is the argument for the test over the sweep.

### Decisions worth recording

- **S7 fixed structurally, not by hand.** `list_functions.py` now reads
  `builtin_type_fields` from the Rust sources, so `net`'s and `term`'s
  Rust-declared record types are visible to the type-page loop rather than
  invisible to it. Their fields had been living as prose in two package pages and
  in the *global* `types` pseudo-package; both now point at the package's own
  `types` page.
- **S9 without deleting content.** `crypto`'s "Security notes" (nonce discipline)
  and `http`'s "Server" section are real, load-bearing prose. They were demoted
  to `###` subsections inside Description rather than removed to satisfy the
  template's section list.
- **S11 by extraction.** The shared authoring rules now live in one file,
  `scripts/man_rules.sh`, sourced by both drivers — including a single
  `man_package_sources` so the two cannot disagree about where a package lives
  again. Both scripts also cited `mod.rs` for the `ERR_*_CODE` constants; they
  are defined in `error_constants.rs`.
- **S8 structurally, not by a hardcoded list.** A page whose `## Synopsis` has no
  `::` qualifier is a package **topic** page and lists under `TOPICS`. That is
  what `regex/language.md` is, and it no longer appears beside `find` and
  `replace` as though it were callable.
- **S2 turned out to have a fourth blind spot.** Fixing `list_functions.py` was
  not enough: `update_man.sh`'s own `module_of` routed the twelve bare `expect*`
  names to `general`, so `update_man.sh testing` still matched nothing. Both are
  fixed.

### Verification

- Corpus: **484 Markdown pages, zero `.txt`**. All 31 packages and all 460
  function pages render (`mfb man <pkg>` / `mfb man <pkg> <fn>`, swept).
- `cargo test` 3101 passed; `scripts/artifact-gate.sh` 1189 goldens, 0 diffs;
  macOS acceptance 1014/1014.
- **Every self-contained example compiles.** An independent sweep extracted all
  1012 `## Examples` blocks and built each one: 964 self-contained blocks
  compile, 48 are genuine fragments (a `TCASE` body, a worker `ISOLATED FUNC`, a
  block importing the second package a `thread` example needs), and **2 fail
  because of a compiler bug, not a documentation bug** — filed as bug-361. Those
  two pages are left as-is: they are valid MFBASIC, and mangling them to dodge a
  codegen defect would be documenting around it. They now serve as its repro.
- Before this work, 627 of those blocks did not compile.

### A process failure worth recording

A late agent ran `git checkout -- src/docs/man` to undo its own partial pass. The
tree was **not** clean at that moment, so the restore silently reverted work that
was not its own: the `man_citations_resolve` test, four citation fixes, and all
five S9 package-page corrections. It was caught only because the citation test
was re-run and had vanished.

This is exactly what `AGENTS.md` forbids — "never run tree-wide `git checkout`/
`reset`/`restore`; only touch and commit files you changed this session". The
rule exists because a shared tree makes an undo indistinguishable from a
deletion. Everything was reconstructed and re-verified, but the only reason it
was noticed is that a *test* disappeared; a reverted prose edit would have been
invisible.

## Goal

- `scripts/update_man.sh <pkg>` succeeds for every package that has a man directory,
  including `collections`, `filters`, `testing`, and `errorCode`.
- Zero `.txt` pages remain under `src/docs/man/`; `mfb man` has one rendering.
- The `mod.rs` guard tests cover every page in the corpus, with a count assertion that
  fails when pages go missing rather than one that passes on a shrinking subset.
- Every `[[path:Symbol]]` citation resolves, enforced in CI.
- Every package overview matches `.ai/man_package_template.md` exactly.

### Non-goals (must NOT change)

- Any compiled-program behavior. This document is entirely documentation and tooling.
- The `mfb man` command surface (`mfb man <pkg> [topic]`), except to add the missing
  `errorCode` row to `PACKAGE_ORDER`.
- The *content* corrections in bug-337 — a migration that silently rewrites a wrong
  claim into prettier Markdown makes bug-337 harder to audit, not easier. Migrate the
  format; fix the claims under their own bug.
- `INTERNAL_CALLS` (`scripts/list_functions.py:24-29`) must keep excluding
  `tls::closeListener` and `crypto::generateP*Raw` from generated pages. (That those
  three crypto entries are nevertheless user-callable is bug-337's problem, and the
  right fix there is a resolver guard, not a man page.)

## Blast Radius

- `scripts/list_functions.py` — S2, S4, S7. Every downstream consumer inherits its
  blind spots; any audit built on it is incomplete by construction.
- `scripts/update_man.sh` — S1 (the migration tool), S7, S11. Blocked on S2.
- `scripts/update_man_package.sh` — S9, S11. Not blocked on S2 (it enumerates
  directories).
- `src/builtins/mod.rs:542-567` — S3. Test-only; no shipped behavior.
- `src/docs/man/mod.rs:29-61,185` — S4 (`PACKAGE_ORDER` row), S1 (`is_markdown_page`
  becomes dead once the migration completes).
- `build.rs:278-291` — S8. Needs a topic-page concept or a naming convention.
- 226 `.txt` files + 24 `package.md` files — the edit surface.
- Unaffected: `src/docs/spec/**` (a separate corpus with its own rules), and every
  `.rs` file outside `src/builtins/mod.rs`'s test module.

## Fix Design

**The ordering is not optional.** `scripts/update_man.sh` is the migration tool for S1,
and it cannot see `collections`, `filters`, or `testing` — the three packages that are
100% legacy `.txt`. Migrating them by hand, or fixing `list_functions.py` afterwards,
both mean the 56 pages that most need regeneration are the only ones produced outside
the pipeline. **S2 lands first.**

For S2, prefer widening `build_const_map` and the predicate-name search over editing the
builtins to suit the script:

- Teach `build_const_map` to also resolve `const NAME: &[&str] = &["a", "b", …];` slice
  constants, which makes `collections` fall out (`FUNCTIONS` + `NATIVE_MEMBERS`).
- Add `is_expect_call` to the predicate-name list at `list_functions.py:133-135`, or
  rename `testing.rs`'s predicate to `is_testing_call` for consistency with its 25
  siblings — the latter is preferable and is a two-line change.
- `filters` has no Rust module at all. Either give it a real one, or (recommended)
  accept that it is a documentation-only grouping of `general.rs` predicates and add the
  same `filters` → `general.rs` special case `update_man_package.sh:35-38` already has.
  Rejected: deleting the `filters` package and folding its 6 pages into `general` — that
  is a user-visible `mfb man` surface change and belongs in its own decision.
- `errorCode` exports constants, not calls. `list_functions.py:142-146` already
  special-cases `math`'s `is_math_constant`; generalize that to any module exposing an
  `is_<stem>_constant` predicate, which picks up `errorcode.rs:23` and removes the
  `math`-only hack in the same edit.

Verify S2 by running `python3 scripts/list_functions.py` and confirming 25 package
headers with `collections` (39 members), `filters`, `testing`, and `errorCode` present,
*before* invoking `update_man.sh` on anything.

For S9's CI check, the resolver used to produce S12's numbers is ~20 lines and belongs
in `scripts/` with a test wrapper:

```python
# scripts/check_man_citations.py — walk src/docs/man/**/*.md, resolve every [[…]].
import re, sys, pathlib
FALSE_POSITIVE = re.compile(r'^:\w+:$|^"')          # [[:alpha:]], [["a", "b"]]
bad = []
for page in pathlib.Path("src/docs/man").rglob("*.md"):
    for m in re.finditer(r"\[\[([^\]]+)\]\]", page.read_text()):
        cite = m.group(1)
        if FALSE_POSITIVE.search(cite):
            continue
        if ":" not in cite:
            bad.append((page, cite, "no :Symbol"))
            continue
        path, symbol = cite.rsplit(":", 1)
        target = pathlib.Path(path)
        if not target.is_file():
            bad.append((page, cite, "file missing"))
            continue
        src = target.read_text()
        if symbol.isdigit():
            if int(symbol) > len(src.splitlines()):
                bad.append((page, cite, "line out of range"))
        elif not re.search(rf"\b{re.escape(symbol)}\b", src):
            bad.append((page, cite, "symbol not found"))
for page, cite, why in bad:
    print(f"{page}: [[{cite}]] — {why}", file=sys.stderr)
sys.exit(1 if bad else 0)
```

Against the current tree this reports 1 dangling symbol (S12) and 6 file-only citations;
fix those, then wire it into the acceptance suite so it starts green. Do **not** relax
the file-only case to "valid" — the template requires `path:Symbol`, and a file-only
citation is unfalsifiable, which is the whole point of the check. Rejected alternative:
extending `documented_builtins()` in Rust to do this. It would run under `cargo test`
but reading `src/docs/man/**` from a unit test is what created S3's brittleness; a
standalone script keeps the corpus check independent of the builtins module.

Note that migrating a page from `.txt` to `.md` shifts `mfb man` output for that page
entirely (flat text → boxed, ruled, tabled). Any golden capturing `mfb man` output for a
migrated package will churn wholesale; that churn is expected and is the intended extent.

## Phases

### Phase 1 — unblock the pipeline (no page edits)

- [ ] S2: widen `list_functions.py` (slice consts, `is_expect_call`/rename,
      `filters` special case, generalized `is_<stem>_constant`). Confirm 25 packages.
- [ ] S3: make `documented_builtins()` extension-agnostic, skip `package`, and replace
      the `> 100` floor. Confirm the two guards now run over 435 pages and still pass —
      if they fail, the failures are real bug-337 findings and belong there.
- [ ] S12: land `scripts/check_man_citations.py`; record its current output in this file.

Acceptance: `./scripts/update_man.sh collections` no longer exits 1; the guard tests
enumerate the whole corpus; the citation checker runs (red is acceptable at this point).
Commit: —

### Phase 2 — coverage gaps

- [ ] S4: add the `errorCode` row to `PACKAGE_ORDER` and generate its package + constant
      pages.
- [ ] S5: generate the 18 `general` function pages; fix the six dead cross-references in
      `unicode/package.md:41-43` (dropping `find`/`mid`, which moved packages).
- [ ] S6: generate the 10 missing `http` function pages, `http/types`, and
      `strings/toBytes`.
- [ ] S7: generate `http/types` and `net/types`; teach the type loop about Rust-declared
      record types; relocate `term`'s two records out of `types/package.md:105-111`.
- [ ] S9: bring `audio`, `crypto`, `http`, `money`, `testing` package overviews back to
      the template; delete `money/package.md:44-48`'s duplicate function list.
- [ ] S12: fix the dangling `net/package.md:40` citation and the six file-only ones;
      the checker goes green.

Acceptance: every package in `PACKAGE_ORDER` has a page for every member
`list_functions.py` reports; `check_man_citations.py` exits 0.
Commit: —

### Phase 3 — migration and consolidation

- [ ] S1: run `scripts/update_man.sh <pkg>` per package for the 13 legacy packages;
      `git rm` each `.txt` as its `.md` lands. 226 pages — land package by package.
- [ ] S10: confirm the 51 missing-PARAMETERS / 20 missing-ERRORS pages resolve as a side
      effect; hand-fix any residue in `http`/`net`/`json`.
- [ ] S8: decide and implement the topic-page mechanism in `build.rs:278-291`; stop
      listing `regex::language` as a function.
- [ ] S11: factor the shared prompt prose into one file both scripts source; correct
      `update_man.sh:92` to name the `_package.mfb` source when one exists.
- [ ] Delete `is_markdown_page` (`src/docs/man/mod.rs:185`) and the legacy renderer once
      zero `.txt` pages remain.

Acceptance: `find src/docs/man -name '*.txt' | wc -l` is 0; `mfb man` has one rendering;
full acceptance suite green with the expected man-output golden churn and nothing else.
Commit: —

## Validation Plan

- Regression tests: the extension-agnostic `documented_builtins()` (S3) and
  `scripts/check_man_citations.py` wired into acceptance (S12).
- Runtime proof: `mfb man errorCode`, `mfb man general`, `mfb man strings toBytes`,
  `mfb man http read`, and `mfb man money` (function list printed once) all resolve;
  `mfb man math round` renders in the boxed style.
- Tooling proof: `python3 scripts/list_functions.py` reports 25 packages;
  `./scripts/update_man.sh testing` completes.
- Doc sync: `AGENTS.md:53-60` if the rules move out of the driver scripts (S11).
- Full suite: `scripts/test-accept.sh` plus the doc/spec render checks.

## Open Decisions

- **`filters`** — keep it as a documentation-only grouping with a `list_functions.py`
  special case (recommended, no user-visible change), vs. dissolving it into `general`
  (cleaner model, but changes `mfb man` output and breaks 6 page paths). (S2)
- **`testing`'s predicate name** — rename `is_expect_call` → `is_testing_call` to match
  its 25 siblings (recommended), vs. teaching the script the exception. (S2)
- **Topic pages** — a `topics/` subdirectory per package, vs. a leading-underscore or
  front-matter marker `build.rs` recognizes. (S8)

## Summary

The engineering risk is concentrated in Phase 3's 226-page migration, which is mechanical
but high-volume and shifts `mfb man` output wholesale for 13 packages. Phases 1 and 2 are
small, high-leverage, and are what make Phase 3 tractable at all: until
`list_functions.py` can see `collections`, `filters`, and `testing`, the migration tool
cannot touch the packages in the worst shape. Nothing here changes compiled-program
behavior; the value is that after it lands, a wrong man page becomes detectable by CI
rather than by a human reading 485 files.
