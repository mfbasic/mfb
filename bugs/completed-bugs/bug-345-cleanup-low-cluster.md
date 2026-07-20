# bug-345: cleanup LOW residue — stale comments, misleading names, file ordering, mechanical lint clusters

Last updated: 2026-07-18
Effort: medium (1h–2h)
Severity: LOW
Class: Other (cleanup)

Status: FIXED (2026-07-19) — A-C in commit 4791f665a, D in the follow-up commit
Regression Test: none (no behavior change; `scripts/artifact-gate.sh` byte-identity is the guard)

The LOW-value residue from the cleanup review that does not belong to any themed
cleanup document (bug-321 … bug-344): comments that contradict the code they sit
on, identifiers that name something other than what they hold, items filed in the
wrong place in their file, and one machine-applicable lint sweep. Direct successor
to bug-300, same itemized shape.

Nothing here changes program semantics or emitted bytes. **Every item in this
document is expected to leave `scripts/artifact-gate.sh` byte-identical with zero
golden churn** — that is the acceptance criterion, not a caveat. If any edit does
shift a byte, the edit is wrong; do not resync goldens to accommodate it.

References:

- Cleanup review index `/tmp/cleanup-findings/index.md` (agents 01–22).
- bug-300 (`bugs/bug-300-docs-deadcode-low-cluster.md`) — the same cluster shape;
  E11 there already owns the stale AArch64 comment on
  `linux_x86_64/code.rs:784-785`, so it is deliberately absent below.

## Current State

All 28 items below were re-verified against the working tree at `b12213d2`; every
`path:line` was opened or grepped. Lint counts in section D are measured from a
full `cargo clippy --all-targets` run, not quoted from the review index.

Two review-index claims did **not** survive verification and are corrected here:

- Agent 22 #7 noted `src/audit/collect/dependencies.rs:77` as an already-resolved
  `items_after_test_module` site. **It is live.** `mod tests` opens at `:77` and
  closes at `:160`; `collect_packages` — the module's primary `pub(super)` entry
  point — is defined at `:162` and runs to EOF at `:220`. Clippy still flags it.
  All four sites are real.
- Agent 02 #13 scoped "vreg locals still named after physical registers" to
  `private/unicode.rs + builder_strings*.rs + builder_search.rs`. Only
  `private/unicode.rs` holds (17 sites); the three `builder_strings*.rs` files and
  `builder_search.rs` have zero `let xN = self.temporary_vreg()`. Item B1 is
  narrowed accordingly.

---

## A. Stale comments that contradict the code

### A1 — `(was out-of-pool xN)` regalloc archaeology, 13 sites
- `builder_collection_compare.rs:13,73,298,399`; `builder_strings_builtins.rs:1409,1632,2286,2595`;
  `builder_collection_layout.rs:1802,1831`; `builder_strings_package.rs:154`;
  `builder_conversions.rs:177,312` (all under `src/target/shared/code/`).
- Each narrates the completed plan-34 vreg migration ("was out-of-pool x2/x3/x4,
  which fall back to…"). The physical registers named no longer appear anywhere in
  those functions.
- A reader looking for `x2` finds nothing and cannot tell whether the note is a
  live constraint or history.
- Fix: delete the parenthetical; keep any surviving *reason* clause (e.g. the x86
  ABI-collision note at `builder_collection_layout.rs:1831`) restated in role terms.

### A2 — entry codegen comments name AArch64 physicals in ISA-neutral token code
- `src/target/shared/code/entry_and_arena.rs:45,134,650,659,666`.
- The code emits `abi::SCRATCH[n]` / `abi::RET[n]` tokens (plan-34-D forbids a
  physical register in shared lowering, enforced by the panic at
  `codegen_utils.rs:539`), but the comments read `x9`/`x10` are free scratch",
  `x11 = entry cursor`, `x15 = argv[i]`.
- On x86-64 and riscv64 those names are simply false; the file is compiled for all
  three.
- Fix: restate as roles — "SCRATCH[0] = entry cursor", etc.

### A3 — `linux_gtk/mod.rs` "SCAFFOLD STATUS" header whose own body says shipped
- `src/target/linux_gtk/mod.rs:10-12` (and the `scaffold` echo at
  `linux_gtk/mod.rs:54`, `linux_aarch64/code.rs:147`).
- The header is labelled `SCAFFOLD STATUS (plan-05)` and three lines later says the
  notes "describe the **implemented** main-thread contract". plan-05 is complete
  and archived; the GTK backend ships on linux-aarch64 and linux-x86_64.
- Fix: drop the SCAFFOLD label and the two `scaffold` echoes; keep the contract prose.

### A4 — `linux_gtk/mod.rs` module doc points at `macos_aarch64/app.rs`, a directory
- `src/target/linux_gtk/mod.rs:3,166,200,204` — four cites of `macos_aarch64/app.rs`.
- `src/target/macos_aarch64/app.rs` does not exist; it is `macos_aarch64/app/`
  (mod.rs, app_io.rs, bootstrap.rs, term_view.rs). The specific constants cited
  (`INPUT_MODE_*`, `STR_EXIT_PREFIX`, `STR_STDERR_PREFIX`) live in `app/mod.rs`.
- Fix: repoint the four cites at `macos_aarch64/app/mod.rs`.
- Related, same header: `:8` sends the reader to
  `linux_aarch64/plan.rs::app_mode_imports`, which is a 3-line delegation
  (`linux_aarch64/plan.rs:404-407`) back to `linux_gtk::app_mode_imports` at
  `linux_gtk/mod.rs:684` — in this very file. Cite the real one.

### A5 — `plan_lower` doc states the opposite of what the function does
- `src/target/linux_x86_64/mod.rs:461-462`; `src/target/linux_riscv64/mod.rs:426-427`.
- Both say the backend "reuses the AArch64 backend's `plan` lowering verbatim", but
  `plan` resolves to each backend's **own** `pub(crate) mod plan` (declared at `:11`
  in each file). The AArch64 backend has no such wrapper at all — it calls
  `plan::lower_module` directly (`linux_aarch64/mod.rs:314,384,400,413,432`).
- Fix: delete both wrappers and call `plan::lower_module` directly like aarch64, or
  correct the doc to "this backend's own object-plan lowering".

### A6 — `normalize_c_int_result` claims to be "the single owner" of an invariant it does not own
- `src/target/shared/code/fs_helpers_atomic.rs:11-16` (helper body at `:17`).
- The doc says "This is the single owner of the invariant: it lives at the
  comparison seam so a newly added `int`-returning platform wrapper cannot
  reintroduce the class." In the same file it is bypassed 7× by a direct
  `abi::sign_extend_word(return_register(), return_register())` at
  `:125,163,498,961,1199,1500,1736`, and 47 more times across
  `fs_helpers_io.rs`, `link_thunk.rs`, `net/{mod,io,poll}.rs`,
  `tls/{macos,openssl}.rs`, `audio/{macos,alsa}.rs` (55 direct call sites total).
- Two backends cite it as the authoritative seam (`linux_aarch64/code.rs:450`,
  `linux_riscv64/code.rs:466`), so the false claim propagates.
- Fix: either route the 7 in-file bypasses through the helper and downgrade the doc
  to "the seam for the `fs` atomic helpers", or drop the sole-ownership sentence.
  Do not silently leave both.

### A7 — `lower_stdout_drain` header describes behavior bug-208 inverted
- `src/target/shared/code/io_helpers.rs:7-9` vs the body at `:87-109`.
- The header: "on failure the advanced window is written back (`OUT_PTR`/`OUT_FILLED`
  point past the bytes already sent)". bug-208 does the opposite — it explicitly
  refuses to advance `OUT_PTR`, slides the unflushed tail back down to the buffer
  base, and stores `OUT_PTR = base` (`:107`). The body's own comment at `:89-95`
  says so.
- Dangerous shape: the header still cites bug-97, which the reader will take as
  current.
- Fix: rewrite `:7-9` to the bug-208 contract; keep the bug-97 sentence as the
  historical reason for persisting the window at all.

### A8 — `os.rs` module header covers 6 of 16 lowered calls
- `src/target/shared/code/os.rs:1-11` vs the dispatch at `:164-181`.
- Documents `getEnv`/`getEnvOr`/`hasEnv`/`setEnv`/`unsetEnv`/`environ`. The dispatch
  has 16 arms: also `name`, `arch`, `pid`, `cpuCount`, `hostName`, `userName`,
  `executablePath`, `resourcePath`, `args`. The header's "Each is a small runtime
  helper wrapping a libc primitive" is false for `os.name`/`os.arch`
  (compile-time constants) and `os.resourcePath` (build-mode dependent).
- Fix: extend the list; qualify the "wrapping a libc primitive" claim.
- Note: the review index also flagged a cite of a never-defined `ERRNO_EINVAL`.
  **Dropped** — `os.rs:18` mentions EINVAL only in prose, no symbol is cited.

### A9 — `link_locator.rs` doc references a function that exists nowhere
- `src/target/shared/code/link_locator.rs:8-11`; restated at `link_thunk.rs:145`.
- "This replaces `link_thunk`'s old `library_filename()` soname guess". Grep for
  `library_filename` across `src/` returns exactly these two comments — the function
  is gone.
- Fix: drop the archaeology from `link_locator.rs` (keep at most one line at
  `link_thunk.rs:145` if the rationale is still load-bearing).

### A10 — two macOS app comments document the *absence* of an attribute
- `src/target/macos_aarch64/app/mod.rs:225-227` and `:365-367`.
- Both read "bug-176 E dropped the stale `#[allow(dead_code)]` and the …note".
  Grepping `allow(dead_code)` under `src/target/macos_aarch64/` returns **only these
  two comments about its removal** — the attribute is not there.
- Fix: keep the "consumed by X" clause (that is useful); delete the clause about
  what was deleted.

---

## B. Naming that describes something other than what is there

### B1 — vreg locals still named after the physical registers they replaced
- `src/target/shared/code/private/unicode.rs` — 17 `let xN = self.temporary_vreg()`
  bindings (`:226,227,326,358,359,446,447,514,515,521,…`), each immediately
  re-bound as `let xN = xN.as_str()`.
- The allocator colours these per-ISA; nothing pins `x6`. Sole remaining file (see
  Current State — the strings/search files named in the index are clean).
- Fix: rename to roles (`cp`, `lead`, `cursor`, …).

### B2 — two scratch-allocation idioms and two `scratchN` schemes in one subsystem
- `builder_collection_mutate.rs:1239-1250,3571-3575` use `let sN = temporary_vreg()`
  (17 in the file) while the same file uses `let scratchN = …` 137 times;
  `builder_collection_queries.rs:267-273` uses `scratchN` while `:110-117` uses the
  older `self.allocate_register()?` with role names (`count`, `index`, `entry`).
- Three conventions for one concept inside two sibling files; a reader cannot tell
  whether `allocate_register` vs `temporary_vreg` is a meaningful distinction.
- Fix: standardize on `temporary_vreg` + role names; if `allocate_register` is
  genuinely different, document the difference at its definition.

### B3 — `store_string_pointer` used to spill Integers
- Defined `builder_strings_package.rs:368`. Misused at
  `builder_strings_builtins.rs:2153` (`strings_lr_count`), `:2275`
  (`strings_repeat_times`), `:2419` (`strings_pad_width`), `:2731`
  (`strings_grapheme_at_index`) — all Integer values.
- The name asserts a type the helper never checks; it is a generic
  register→stack-slot spill.
- Fix: rename `spill_to_slot` and move it to the primitives module (39 call sites,
  mechanical).

### B4 — `emit_dlopen_libssl_macos` does not open libssl
- `src/target/shared/code/tls/macos.rs:1762`, called 5×.
- The body forwards to `emit_dlopen_at(.., MACLIB_SYMBOL, ..)`; `MACLIB_SYMBOL` is
  `"_mfb_tls_maclib"` (`tls/macos.rs:4`) = Network.framework. There is no OpenSSL on
  the macOS TLS path at all.
- Fix: rename `emit_dlopen_maclib`.

### B5 — `GOT_OFF` means bytes in one audio backend and frames in the other
- `audio/macos.rs:1542` — `const GOT_OFF: usize = 88; // bytes accumulated so far`.
- `audio/alsa.rs:164` — `const GOT_OFF: usize = 144;` used as a frame count
  (`:1320` "frames read", `:1470` "reload for readi math").
- Same identifier, same subsystem, different unit. Anyone porting a fix between the
  two backends will convert wrongly.
- Fix: `BYTES_GOT_OFF` / `FRAMES_GOT_OFF`.

### B6 — `LoadCommandPlan` (a Mach-O concept) is the ELF writer's struct name
- `src/os/linux/object.rs:16,29,151,558`.
- ELF has program headers, not load commands. `SectionPlan.segment` on the same
  path holds `"PT_LOAD"` (`:116,125,152`), so the field is already speaking ELF
  while the type is speaking Mach-O.
- Fix: `ProgramHeaderPlan` on the Linux side (the shared-extraction work in bug-335
  will want a neutral name anyway — coordinate).

### B7 — `LR_OFFSET` is a locals-region *size*, not a link-register offset
- `src/target/shared/code/datetime.rs:26` (`= 88`, passed at `:165`) and
  `term.rs:20` (`= 64`, passed at `:334`).
- Both are passed as the `local_size` parameter of
  `finalize_vreg_body_with_locals` (`codegen_utils.rs:529-533`), which rounds it to
  16 and reserves a buffer. Contrast `runtime_helpers.rs:730`, where `LR_OFFSET = 0`
  genuinely is the store offset of the link register (`:744`, `:1021`) — so the same
  name means two different things in the same directory.
- `datetime.rs:21-22`'s frame comment ("The saved link register sits at the top")
  describes the pre-vreg frame and no longer holds.
- Fix: rename to `LOCALS_SIZE` in `datetime.rs`/`term.rs`; refresh the frame comment.

### B8 — test-module names diverge four ways in the two app runtimes
- `linux_gtk/mod.rs:883` `identity_tests`, `:961` `import_tests`;
  `macos_aarch64/app/mod.rs:728` `bug53_release_tests`; plain `mod tests` in
  `linux_gtk/bootstrap.rs:703`, `macos_aarch64/app/bootstrap.rs:944`,
  `macos_aarch64/app/term_view.rs:1491`.
- `bug53_` ages worst: it names a fixed bug rather than the subject under test, so
  the module cannot absorb a second release-path test without lying.
- Fix: `mod tests` for whole-file suites, `<subject>_tests` for topic modules;
  rename `bug53_release_tests` → `release_tests` (keep the bug cite in a doc comment).

### B9 — sibling files in `net/` disagree on the visibility spelling
- `net/io.rs` (8×) and `net/poll.rs` (2×) use
  `pub(in crate::target::shared::code)`; `net/mod.rs` (5×) uses `pub(super)`.
- Not purely cosmetic: from `net/mod.rs`, `pub(super)` *is*
  `pub(in ...::code)`, but from `io.rs`/`poll.rs` `pub(super)` would mean `pub(in
  net)`. The long form is the correct one in the children; the short form in the
  parent happens to coincide.
- Fix: spell `pub(in crate::target::shared::code)` in all three, with a one-line
  note at `net/mod.rs` explaining why the short form is not used.

### B10 — `binary_repr` has five encoder conventions against one decoder convention
- Encoders: `Table::encode(&self)` methods (`sections.rs:17,359,458,532,607,682`);
  a free `encode_native_library_table` (`sections.rs:718`); `encode_doc_table`
  living in **reader.rs** (`reader.rs:21`); `BinaryRepr::encode_manifest/
  encode_exports/encode_globals/encode_functions` (`writer.rs:988,1012,1034,1052`);
  and a bare `BinaryRepr::encode` (`writer.rs:934`).
- Decoders: uniformly free `read_*` functions in `reader.rs`
  (`:575,600,841,928,954,983,1022,1039,1056,1079`).
- Consequences: names don't pair (`TypeTable::encode` ↔ `read_type_entries`), and
  `read_string_pool` returns `Vec<String>` while every sibling returns its table
  type, so callers re-wrap by hand.
- Fix: one convention — free `encode_<section>` / `read_<section>` pairs, adjacent,
  each returning its table type. `NATIVE_LIBRARY_TABLE` already has both halves
  adjacent; follow it. (Coordinate with bug-335.)

---

## C. File ordering

### C1 — submodule declarations buried mid-file, 4 sites
- `regalloc/mod.rs:384-385` (of 405 lines); `net/mod.rs:865-866` (of 869);
  `linux_gtk/mod.rs:786-788` (of 996); `macos_aarch64/app/mod.rs:558-560` (of 792).
- Each file puts its entire implementation before revealing that it has children.
- Fix: move the `mod` block to the top, under the module doc. (bug-334 owns the same
  fix for `code/mod.rs` and `builder_collection/mod.rs`; do these four with it or
  after it, same commit shape.)

### C2 — items stranded below an inline `mod tests`, 4 sites (all live)
- `audit/collect/dependencies.rs:77` → `collect_packages` at `:162-220`;
  `cli/resolve.rs:673` → `print_lock_diff` at `:1030`;
  `doc.rs:636` → `STYLE` at `:1053`;
  `link_thunk.rs:1515` → `store_field`/`load_field`/`marshal_struct_in`/
  `narrow_signed_bits`/`marshal_struct_out` at `:1617,1631,1648,1747,1772`
  (~390 lines of production code below the tests in a 2006-line file;
  `marshal_struct_in` is called from `:567`, a thousand lines above its definition).
- Clippy flags all four (`clippy::items_after_test_module`). See Current State: the
  index's claim that `dependencies.rs` was already resolved is wrong.
- Fix: move the stranded items above the test module; add
  `clippy::items_after_test_module` to the deny list so it cannot recur.

### C3 — a constant defined before its dependency, in a file whose header states the opposite rule
- `src/target/shared/code/error_constants.rs:4-5` states: "the layout chains below
  are written in ascending-offset (dependency) order."
- `ENTRY_STACK_SIZE` (`:277`) is `ENTRY_SEED_SCRATCH_OFFSET + 8`, and
  `ENTRY_SEED_SCRATCH_OFFSET` is defined at `:282` — five lines later.
- Fix: swap the two declarations. One-line change; the file's own stated rule is the
  spec.

### C4 — `ToNirJson` declared after its first impl
- `src/target/shared/nir/json.rs:84` (trait) vs `:60` (`impl ToNirJson for NirGlobal`).
- Fix: move the trait above `:60`.

### C5 — `LowerContext` declared 668 lines after its first use
- `src/ir/lower.rs:894` (struct) vs `:226` (first construction). `write_ir` at `:653`
  is a file-writing entry point stranded in the middle of the lowering body.
- Fix: hoist `LowerContext` above `:226`; move `write_ir` to the head or tail of the
  file with the other entry points. (Overlaps bug-342's split of `lower.rs` — if that
  lands first, this is subsumed.)

### C6 — `derive(Clone)` separated from its item by a blank line, 8 sites
- `ir/mod.rs:13,119`; `ir/op.rs:3`; `ir/value.rs:3,12`; `ir/types.rs:3,53,59`.
- Exactly 8 tree-wide, all in `src/ir/`, and inconsistent within each file.
- Fix: delete the blank line. (`clippy::empty_line_after_doc_comments` catches the
  doc-comment variant of this at 2 other sites; this attribute variant is not linted.)

---

## D. Mechanical lint clusters

Land section D as a **separate commit** from A–C so the review of the hand-written
items stays trivial. `cargo clippy --fix --all-targets` handles D1 entirely.

### D1 — 45 auto-fixable lint sites in 6 clusters
Measured from `cargo clippy --all-targets` at `b12213d2` (deduplicated by site):

| Lint | Sites | Where |
| --- | --- | --- |
| `manual_is_multiple_of` | 12 | `arch/aarch64/encode/emitter.rs` (10, consecutive), `os/linux/squashfs.rs`, `os/macos/link/tests.rs` |
| `cloned_ref_to_slice_refs` | 7 | `cli/build.rs` (4), `binary_repr/tests.rs` (3) |
| `doc_lazy_continuation` | 7 | `builder_inplace_assign.rs` (3), `error_constants.rs` (2), `os/macos/link/commands.rs`, `builder_collection_layout.rs` |
| `needless_range_loop` | 7 | `regalloc/analysis.rs` (5), `arch/riscv64/encode/tests.rs`, `peephole.rs` |
| `needless_return` | 6 | `resolver/resolution.rs` (3), `builder_collection_layout.rs` (3) |
| `redundant_pattern_matching` | 6 | `arch/x86_64/encode/tests.rs` (6) |
| `doc_overindented_list_items` | 2 | `crypto_ec.rs` (2) |

- Fix: `cargo clippy --fix --all-targets`, then `cargo fmt` (twice — `repository/` is
  not a workspace member). Inspect the `regalloc/analysis.rs` and `peephole.rs`
  `needless_range_loop` rewrites by hand; the rest are trivially mechanical.
- Expected artifact delta: **none**. All seven clusters are source-level rewrites
  with identical semantics.

### D2 — 49 `excessive_precision` warnings are deliberate; add a scoped allow, do NOT trim
- `builder_simd_float_math.rs` (30), `builder_pow.rs` (19).
- These are the `hi` halves of fdlibm/Remez double-double constants, paired with
  `lo` tails to recombine past double precision. Trimming digits to silence the lint
  would silently degrade `pow`/`exp`/`log`/`sin`/`cos` accuracy — a correctness
  regression dressed as a lint fix.
- The precedent already exists in one of the two files:
  `builder_simd_float_math.rs:1-7` carries `#![allow(clippy::approx_constant)]`
  above a six-line comment explaining exactly this reasoning.
- Fix: add `#![allow(clippy::excessive_precision)]` to both files, each with a
  comment pointing at the paired-`lo`-tail rationale. Extend the existing
  `builder_simd_float_math.rs` block rather than adding a second one.

---

## Verified no-action findings (recorded so they are not re-derived)

### N1 — plan/bug citation hygiene in `src/` is CLEAN
Measured directly at `b12213d2`, resolving against `planning/`,
`planning/old-plans/`, `bugs/`, `bugs/completed-bugs/`, and `bugs/skipped/`:

- **2,972 raw citations** in `src/` (1,909 `plan-N`, 1,063 `bug-N`).
- **48 distinct plan ids**: 47 archived under `planning/old-plans/`; **1 live** —
  `plan-47` → `planning/plan-47-windows-x86_64.md`, cited correctly as
  forward-looking. Zero dangling.
- **225 distinct bug ids**: 223 resolve to `bugs/completed-bugs/`, 1 to
  `bugs/skipped/` (`bug-218`), **zero to an open bug in `bugs/`**.
- One id does not resolve anywhere: **`bug-255`**, cited twice at
  `link_thunk.rs:415,1796`. No document exists in any of the three directories. This
  is the only dangling bug citation in the tree; either restore the doc or reword the
  two comments to state the rule directly (they already do, so deleting the cite is
  fine).

No sweep is warranted. **The one latent trap** is that `bugs/skipped/` is a third
resolution location a reader would not guess from `AGENTS.md`.

- Fix: one line in `AGENTS.md` noting that a `bug-N` cite resolves in `bugs/`,
  `bugs/completed-bugs/`, **or** `bugs/skipped/`, and a `plan-N` cite in `planning/`
  or `planning/old-plans/`.

### N2 — dropped item
`linux_x86_64/code.rs:784-785` (stale AArch64 comment on the x86 variadic emitter,
Agent 10 #23) is **not** included: bug-300-E11 already owns that site and the
comment fix is part of its AL-zeroing fix.

---

## Goal

- Every comment cited above agrees with the code it annotates.
- Every identifier cited above names what it holds.
- Every file cited declares its structure before its implementation, and no
  production item sits below a test module.
- `cargo clippy --all-targets` reports zero warnings in the seven D1 clusters, and
  the 49 `excessive_precision` warnings are silenced by a documented allow with the
  constants untouched.
- `AGENTS.md` names all three bug-resolution directories.

### Non-goals (must NOT change)

- Any emitted byte. No item here is permitted to shift a golden. If a rename or a
  reorder changes `.ncode`/`.nplan`/`.nobj`/binary output, stop — the change is wrong.
- The `excessive_precision` constants themselves. Trimming a digit to satisfy the
  lint is explicitly forbidden (D2).
- The structural refactors owned by bug-321…bug-344. Where an item overlaps one
  (C1/C5/B6/B10), do the minimal local fix or defer — do not start the split here.
- `normalize_c_int_result`'s behavior (A6): either route the bypasses through it or
  fix the doc; do not change what the helper emits.

## Blast Radius

Each item is a localized edit in an independent module; land per item or per
section. The only items with more than a single-file footprint:

- A6 — 55 `sign_extend_word` call sites audited; only the 7 in
  `fs_helpers_atomic.rs` are in scope. The other 48 are a different seam and stay.
- B3 — 39 `store_string_pointer` call sites, all mechanical rename.
- B9/B10 — touch `net/` and `binary_repr/` respectively; both overlap bug-331 and
  bug-335. Sequence after those if they land first.

## Fix Design / Phases

### Phase 1 — hand-written items (A, B, C)
- [ ] Section A (10 comment fixes). No code touched.
- [ ] Section B (10 renames + one visibility spelling). Compiler-checked; no
      behavior change.
- [ ] Section C (6 reorderings + the deny-list entry for
      `items_after_test_module`).

Acceptance: `scripts/artifact-gate.sh` byte-identical; `cargo clippy` shows one
fewer lint class (`items_after_test_module` → 0).
Commit: —

### Phase 2 — mechanical lint sweep (D), separate commit
- [ ] `cargo clippy --fix --all-targets`; `cargo fmt` (second pass in `repository/`).
- [ ] Hand-review the two `needless_range_loop` rewrites in `regalloc/analysis.rs`
      and `peephole.rs`.
- [ ] Add the two `#![allow(clippy::excessive_precision)]` blocks with rationale.

Acceptance: `cargo clippy --all-targets` clean in the seven D1 clusters and silent
on `excessive_precision`; `scripts/artifact-gate.sh` byte-identical.
Commit: —

### Phase 3 — validation
- [ ] `scripts/artifact-gate.sh` — must be byte-clean after **both** commits.
- [ ] `scripts/test-accept.sh` — full acceptance, zero golden churn expected.
- [ ] `AGENTS.md` line for the three bug directories (N1).

Acceptance: full suite green; `git status` shows no golden file modified.
Commit: —

## Validation Plan

- Regression test: none added. The guard is byte-identity —
  `scripts/artifact-gate.sh` (execution-free, ~5 min) after each phase, then
  `scripts/test-accept.sh` for the full acceptance sweep.
- Runtime proof: not applicable; no item reaches a runtime path.
- Doc sync: `AGENTS.md` only (N1). No spec or man page changes — spec drift is owned
  by bug-337/bug-338.
- Full suite: `scripts/artifact-gate.sh` && `scripts/test-accept.sh`.

## Open Decisions

- A6 — route the 7 in-file bypasses through `normalize_c_int_result` (preferred:
  makes the doc true) vs. weaken the doc to "the seam for the `fs` atomic helpers"
  (cheaper, leaves the tree-wide pattern undocumented).
- B10 / B6 — do them here as renames, or defer wholesale into bug-335's
  `binary_repr`/linker restructure. Recommend: defer if bug-335 is scheduled;
  otherwise rename now, since the misnomers actively mislead.
- N1 `bug-255` — restore the missing document vs. delete the two citations.
  Recommend delete; both comments already state the rule in full.

## Summary

28 verified items, all cosmetic: 10 stale comments, 10 misnamed identifiers, 6
ordering nits, and 2 lint clusters (~94 sites, machine-applicable). One review-index
claim was refuted during verification (`dependencies.rs:77` is live, not resolved)
and one narrowed (B1 holds in one file, not five). One item dropped as already owned
by bug-300. Citation hygiene measured and recorded as clean — the only action there
is a one-line `AGENTS.md` note about `bugs/skipped/`.

The engineering risk is near zero and concentrated entirely in the byte-identity
check: nothing here should move a golden, so any golden churn is the signal that an
edit went further than intended.


---

# Resolution (2026-07-19)

All 28 items resolved except **B10**, deliberately deferred. Acceptance held:
`scripts/artifact-gate.sh` byte-identical after **both** commits (1002 tests,
1195 goldens, 0 diffs), full `scripts/test-accept.sh` green with zero golden
churn, `cargo clippy --all-targets` clean.

## Deviations from this document

Four, each a case where the plan did not survive contact with the tree.

- **A5 needed no action.** The `plan_lower` wrappers whose docs claimed to
  "reuse the AArch64 backend's `plan` lowering verbatim" no longer exist; all
  three Linux backends already call `plan::lower_module` directly. Grep for
  `plan_lower` returns nothing.
- **A6 took the *second* Open Decision, not the preferred one.** The doc
  preferred routing the 7 in-file `sign_extend_word` bypasses through
  `normalize_c_int_result` because it "makes the doc true". It does not: the
  same pair is written inline at ~48 further sites that this document
  explicitly scopes out, so the "single owner of the invariant" claim would
  still be false afterward. The doc was downgraded instead — the helper is
  named as one spelling, not a choke point, and warns that a new `int`-returning
  wrapper must apply the extension deliberately. `linux_common/code.rs`'s
  propagating cite was corrected to match. Nothing about what the helper emits
  changed.
- **C2 had 5 live sites, not 4.** `src/audit/collect/project.rs:111` is a fifth,
  new since `b12213d2`. (This document's own Current State correction stands:
  `dependencies.rs:77` was indeed live.) All 5 fixed;
  `items_after_test_module = "deny"` added under `[lints.clippy]` in
  `Cargo.toml`.
- **B10 deferred to bug-335.** Restructuring `binary_repr`'s five encoder
  conventions against one decoder convention *is* the split bug-335 owns, and
  this document's own non-goals forbid starting it here. B6 was done, since it
  is a purely local type rename.

## Two traps worth recording

- **`cargo clippy --fix --all-targets` trimmed the D2 constants.** D2 says
  explicitly not to trim them, but `--fix` does not read prose: it rewrote every
  `hi` half in `builder_pow.rs` and `builder_simd_float_math.rs` to the shortest
  literal that round-trips a lone `f64` (e.g. `6.931_471_805_599_452_862_27e-01`
  -> `6.931_471_805_599_453e-1`), destroying exactly the digits the paired `lo`
  tail recombines against. Both files were reverted and the scoped allows added
  instead. **Add the `#![allow]` blocks BEFORE running `--fix`, not after.**
- **`approx_constant` in `builder_pow.rs` was already breaking `cargo clippy`.**
  It is deny-by-default (correctness group), and only `builder_simd_float_math.rs`
  carried the allow. So `cargo clippy --all-targets` exited non-zero before any
  of this work — a pre-existing failure this document did not record, since its
  D-section counts came from warnings only. `builder_pow.rs` now carries both
  allows with the same rationale.

## A5/A1 note

A1 was specified as 13 `(was out-of-pool xN)` sites. The same defect also
appears as `was hand-pinned xN` / `was physical xN` (6 further sites, 5 in
`private/unicode.rs` and 1 in `builder_value_semantics.rs`); those were cleaned
up with A1/B1 since B1's renames would otherwise have rewritten the physical
register names *inside* those comments into nonsense.
