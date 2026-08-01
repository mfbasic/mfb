# bug-394: minor doc-comment / diagnostic-message text inaccuracies (goal-07 batch)

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Other (documentation/diagnostic text vs behavior)

Status: Open
Regression Test: none (comment/message-only; no behavioral test — the diagnostic
item (1) can carry a message-substring assertion if desired).

A batch of small, same-class human-readable-text defects surfaced during the
goal-07 full-source review: a user-facing diagnostic and three internal doc
comments whose text contradicts the code they annotate. None has any runtime or
codegen effect; each is a one-line/one-paragraph edit. Batched per the goal's
"batch trivial same-class findings" rule. Kept distinct-root-cause items
(bug-395/396/397) in their own documents.

References: found during goal-07; files cited inline below.

## Items

### (1) `src/ast/doc_items.rs:15` — DOC "no header" message omits RESOURCE (user-facing)
The `DOC_BAD_HEADER` "no header line" diagnostic reads:
`"DOC block has no header line; expected FUNC, SUB, TYPE, UNION, ENUM, or PACKAGE."`
— it omits `RESOURCE`, yet the parser accepts a `RESOURCE` header (`doc_items.rs:28`)
and the *sibling* bad-keyword message (`doc_items.rs:34`) correctly lists
`FUNC, SUB, TYPE, UNION, ENUM, RESOURCE, or PACKAGE`. The message tells the user
RESOURCE is not a valid DOC header when it is.
- Fix: add `RESOURCE` to the list at line 15.

### (2) `src/builtins/collections.rs:120` — stray/misattributed doc fragment
`unary_callback_member` (line 132) is preceded by an orphaned sentence fragment:
`/// The bare native name for a `collections.<member>` native-member call, e.g.`
that dangles on "e.g." and describes a *different* function — `native_member_bare`
(line 144), which already carries its own correct, complete doc. The stray line
misattributes `unary_callback_member`'s purpose.
- Fix: delete the stray line 120 (the real doc for `unary_callback_member` begins
  at line 121, "Whether a native ... call takes a unary callback ...").

### (3) `src/builtins/net.rs:283` — comment contradicts the line it annotates
The comment "Overloaded on `Socket|UdpSocket` — ... overloaded calls must return
`None` ... (bug-173 D)." sits directly above `BIND_UDP => Some("String, Integer")`.
`BIND_UDP` (`bindUdp`) is NOT overloaded on `Socket|UdpSocket` and returns `Some`;
the comment actually describes the *absent* `SET_READ_TIMEOUT` / `SET_WRITE_TIMEOUT`
entries (which fall through to `_ => None` and are asserted `None` at lines
663-664). A maintainer is misled into thinking `bindUdp` is the overloaded/None case.
- Fix: move the comment to annotate the `_ => None` fall-through (or reword it to
  reference the timeout setters), not `BIND_UDP`.

### (4) `src/arch/x86_64/encode/emitter.rs:222` — doc comment on the wrong function
The doc comment at lines 222-228 is attached to `enc_push_reg`, but its main
paragraph describes the register-to-register ALU "MR form" (`op reg64, reg64`) —
which is `alu_rr` (line 247, currently undocumented). Readers of `enc_push_reg`
get an ALU-encoding explanation; readers of `alu_rr` get nothing.
- Fix: move the MR-form paragraph to `alu_rr` and give `enc_push_reg` a correct
  one-line doc.

### (5) `src/arch/riscv64/v128.rs:1155` — comment claims ops are scalarized that actually panic
`rvv_arm`'s fallthrough comment says "Everything else (FRint*, FCvtasV, wide
integer shifts, AbsV/Cnt8bV/Addv8bV, SshlV/UshlV) is left to the scalar arm."
FRint*/FCvtasV/AbsV are genuinely handled by `scalarize_v128`, but `SshlV`/`UshlV`
are NOT — `scalarize_v128` has no arm for them and hits
`other => panic!("rv64 v128: op {} not yet scalarized")` (line 807), the exact
fail-loud pinned by the `unhandled_v128_op_panics` test. The comment also names
`Cnt8bV`/`Addv8bV`, which are not in the `is_v128` set at all. No wrong bytes are
emitted (the panic is a correct fail-loud); the comment misleads a maintainer into
thinking register-shift v128 ops are scalarization-supported on rv64.
- Fix: reword the comment to list only the ops `scalarize_v128` actually handles,
  and note `SshlV`/`UshlV` are unsupported (fail-loud), dropping the non-`is_v128`
  names.

### (6) `src/cli/help.rs:197` and `:78` — help advertises the wrong default output filename (user-facing)
`DOC_HELP` (line 197) and `PKG_HELP` (line 78) both state
`--out <file>  Path to the generated HTML file (default: index.html)`, but the
actual default is `doc.html` (`src/cli/doc.rs:46` and `src/cli/pkg.rs:1792`, both
`.unwrap_or_else(|| PathBuf::from("doc.html"))`). `mfb doc --help` tells the user
to look for `index.html`; the file is written as `doc.html`. (Runtime does print
`Wrote documentation to doc.html`, which keeps this LOW.)
- Fix: change both help strings to `default: doc.html`.

### (7) `src/cli/resolve.rs:337` — stale source-line citation in a comment
The doc on `is_registry_dependency` says "`add_package_from_file` copies the ident
out of the `.mfp` *header* (`src/cli/pkg.rs:566`)". pkg.rs:566 is now inside
`run_remove` (an unrelated chain); `add_package_from_file` lives at pkg.rs:1106 and
copies the ident at :1140. The citation drifted.
- Fix: repoint the citation to `src/cli/pkg.rs:1140` (or drop the line number).

### (8) `src/cli/build/native_libs.rs:36` — stale doc contradicts code (security-adjacent)
`emitted_link_targets`'s doc says a Linux app-mode build "emits a single glibc
binary ... must be checked for glibc only". The code (lines 47-59) returns BOTH
`Libc::Glibc` and `Libc::Musl` for every Linux build (per plan-56-B; the inner
comment 48-50 and the test at mod.rs:1956-1963 confirm `app == console`). The
outer doc is stale and security-adjacent — it describes which vendored blobs are
hash-verified; a maintainer "restoring" glibc-only app resolution per this doc
would put the glibc blob in the musl AppImage.
- Fix: rewrite the doc to state both flavors are always resolved for Linux.

### (9) `src/cli/build/native_libs.rs:270` — stale "unimplemented pre-51" doc
`resource_output_dirs`'s doc says "the `LinuxApp` arm depends on plan-51-A's AppDir
existing; until then a Linux `--app` build never reaches this path (Linux app mode
is unimplemented pre-51)." plan-51..66 (Linux app/AppImage) are complete/merged, so
the arm is now live on every Linux `--app` build (tested at mod.rs:1836-1843).
- Fix: drop the "unimplemented pre-51 / never reaches" claim.

### (10) `src/rules/table.rs:717` — retired-code comment contradicts a live reuse
The comment says `// 2-203-0102 (TYPE_INLINE_TRAP_ON_INLINED_BUILTIN) retired in
plan-26-C: ... The code is not reused.` But `2-203-0102` IS reused: line 640
assigns it to `TYPE_INSTANTIATION_TOO_DEEP`, and the embedded spec confirms it
(`src/docs/spec/diagnostics/01_rule-codes.md:99,:373`). The identical boilerplate is
correct for the parallel `2-208-0007` case (line 1467, genuinely skipped) — a
copy-paste comment error.
- Fix: correct the line-717 comment to reflect that `2-203-0102` is reused by
  `TYPE_INSTANTIATION_TOO_DEEP` (or remove the "not reused" claim).

### (11) `src/unicode/runtime_tables.rs:108` — unchecked `stage1` index (latent panic, not text)
`property_for_codepoint(codepoint: u32)` indexes `tables.stage1[(codepoint >> 8) as
usize]` with no bounds guard; `stage1.len()` is 4352 (valid for `codepoint` ≤
0x10FFFF), so any `codepoint > 0x10FFFF` panics OOB. Latent only: the fn carries a
documented `#[allow(dead_code)]` (bug-326-D4) with no production caller (the emitted
runtime does its own lookup); reachable only if a future caller passes an
out-of-range `u32`. (Included here as a batched LOW latent nit; it is a bounds
guard, not a text fix.)
- Fix: clamp/guard `codepoint > 0x10FFFF` (return the default property) before the
  index, matching the runtime's own behavior.

### (12) `src/target/shared/code/codegen_utils.rs:22-23` — stale register-legend comment
The legend for `lower_sort_string_list_helper` says `x11 = data region base
(entries base + count * entry size)`, but the code (lines 44-48, and its own inline
comment) computes the data-region base from `COLLECTION_OFFSET_CAPACITY`
(`entries base + capacity * entry size`) — correct for a grown list (capacity >
count). A maintainer trusting the "count" legend could reintroduce a real defect
(short data region → name-pointer reads land in the wrong region → wrong sort).
- Fix: change the legend to "capacity * entry size".

### (13) `src/target/shared/code/builder_fixed_math.rs:399-404` — stale CORDIC doc paragraph
`emit_fixed_sincos`'s doc opens with a paragraph describing a bare CORDIC-rotation
primitive ("On entry `cosr` holds the inverse gain and `sinr` is zero; on exit
`cosr ~= cos(z0)`…"), but `emit_fixed_sincos` also does `theta*2/pi` range reduction
and k-mod-4 quadrant selection; the correct one-line description follows. Stale
leftover from the bug-332-E1 CORDIC merge (the sibling `emit_cordic` doc at ~227-236
has the same artifact — its first sentence describes only vectoring mode of what is
now a two-mode loop).
- Fix: drop/rewrite the stale first paragraph on both `emit_fixed_sincos` and
  `emit_cordic` to match current behavior.

### (14) `src/target/shared/code/builder_simd_fixed_math.rs:118` — stale "x0 digit counter" comment
`emit_fixed_sqrt_vector`'s header comment says "Uses `x0` as the (shared) digit
counter and physical `v1..v7`…", but the code (`:156`) allocates the digit counter
via `self.allocate_register()` (an allocator-placed vreg per plan-34-B, per its own
inline comment at :154-155). The "x0 digit counter" half is stale; "physical v1..v7"
is still accurate.
- Fix: update the header comment to say the digit counter is an allocator vreg.

### (15) `src/target/shared/code/os/env.rs:35` — doc paragraph on the wrong function
`env_lock_fns`'s doc (lines 35-46) opens with three lines describing a *different*
function — "Acquire the env/pwd lock: `pthread_mutex_lock(...)`. Emitted at helper
entry, after incoming `String*` arguments have been saved into vregs…" — which
describes `emit_env_lock` (line 48, itself undocumented). The `env_lock_fns` text
only begins at line 38.
- Fix: move the acquire-and-emit paragraph to `emit_env_lock`; leave `env_lock_fns`
  its name-lookup description.

### (16) `src/target/win_x86_64/app/mod.rs:52` — wrong Consolas-metrics comment
`const TUI_CELL_W: usize = 8; // px per cell (matches the Consolas metrics we
request)` — but no Consolas font is requested; `emit_term_on` selects
`GetStockObject(SYSTEM_FIXED_FONT = 16)` (:1147). The hardcoded 8×16 cell metrics are
not guaranteed to match the system fixed font's glyph advance (cosmetic grid-align
risk), and the comment's rationale is factually wrong.
- Fix: correct the comment to reference `SYSTEM_FIXED_FONT`; separately consider
  measuring the actual font metrics for cell sizing (cosmetic, out of scope here).

### (17) `src/builtins/audio_mml.mfb:4` — "whitespace-separated" but splits on space only
The header (lines 4-5) says "A track is a whitespace-separated string of tokens;
every token is separated by whitespace", but `__audio_mmlTokens` (line 272) splits
only on the literal space: `strings::split(mml, " ")`. Tab/newline-separated tokens
are NOT split (the run becomes one token → ErrInvalidArgument). The `audio::play`
man page correctly says "splits on the space character", so the source header is the
outlier.
- Fix: change the header to "space-separated" to match the code and man page.

### (18) `src/target/linux_gtk/term_draw.rs:388` — wrong surface-size comment
`emit_term_init_helper`'s doc says the font cell is measured "via a throwaway 1x1
image surface", but the code (:406-409) creates an 8×8 surface
(`cairo_image_surface_create(ARGB32, 8, 8)`). Surface size doesn't affect the
measured font extents, so cosmetic — but the stated dimension is wrong.
- Fix: change "1x1" to "8x8" in the comment.

## Goal

- Each message/comment above matches the behavior of the code it annotates.

### Non-goals

- No behavioral / signature / codegen change. Purely text.

## Blast Radius

- Four isolated text sites, listed above. No shared code path; no consumer depends
  on the exact text except item (1), which is user-facing diagnostic output (a
  message-substring test, if any exists for `DOC_BAD_HEADER`, should be updated to
  include RESOURCE).
