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

## Goal

- Each message/comment above matches the behavior of the code it annotates.

### Non-goals

- No behavioral / signature / codegen change. Purely text.

## Blast Radius

- Four isolated text sites, listed above. No shared code path; no consumer depends
  on the exact text except item (1), which is user-facing diagnostic output (a
  message-substring test, if any exists for `DOC_BAD_HEADER`, should be updated to
  include RESOURCE).
