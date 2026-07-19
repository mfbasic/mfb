# bug-338: embedded specification (`src/docs/spec/**`) has drifted from the implementation in ~50 places

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (documentation)

Status: Fixed (2026-07-19)
Regression Test: `src/docs/spec/mod.rs` — `spec_links_resolve` and
`spec_citations_resolve`, both landed and both green. They caught real breakage on
their first run: every C9 file citation, which is how the ten stale paths below
were found rather than hand-listed.

`.ai/specifications.md` makes the embedded spec **the single source of truth for
every externally observable compiler/language/format/ABI contract**, version-locked
to the binary, and requires every contract change to update the owning topic in the
same commit ("part of the Hard Completion Gate, not optional cleanup"). That gate
has leaked. A two-sided sweep of `src/docs/spec/**` against `src/**` found **51
confirmed drifts across 8 spec packages** — each one a spec line and an
implementation line, both personally read.

Nothing here changes compiled-program behavior, which is why the severity is LOW.
But the spec is written to be *reimplementable*, and several items actively mislead
a reader on security-relevant behavior:

- Three linker topics state the Linux ELF header is `ET_EXEC` at base `0x400000`,
  and two say emphatically **"There is no `ET_DYN`/PIE path."** The dynamic encoder
  has emitted `ET_DYN` at base `0` since bug-186. A reader auditing ASLR posture
  from the spec reaches the **inverted** conclusion.
- `--unsigned` — the flag that relaxes the audit-1 PKG-01 supply-chain check — is
  implemented and in `mfb build --help` but absent from the CLI reference.
- The audit JSON renderer is documented as escaping bidi/format overrides. It does
  not; a package name carrying U+202E survives the JSON path verbatim.
- A reader decoding a `List OF Scalar` from `memory/05_collections.md` **mis-strides**:
  the element type-id table omits `Scalar` entirely, and `Scalar` is the only
  element type that is neither 1- nor 8-byte aligned.

The single correct behavior a fix produces: every one of the 51 cited spec lines
states what the implementation actually does, and a `cargo test` failure — not a
human reading a checklist — is what catches the next one.

References:

- `.ai/specifications.md` — the spec-currency rule, the citation convention
  (`[[src/file.rs:Symbol]]`), the single-source-of-truth rule, and the manual
  verification step this bug proposes to automate (`:56-58`).
- `src/docs/spec/mod.rs:107-139` — the existing spec tests (discovery/lookup/summary
  only; no link or citation guard).
- `src/builtins/errorcode.rs:47` (`table_matches_registry`) — the precedent: the
  error-code registry *does* have a mechanical drift guard.
- Found during the `cleanup-review` sweep (agents 02–19); this document is the
  consolidated spec-drift cluster.
- House style: `bugs/bug-300-docs-deadcode-low-cluster.md`.

## Current State

Verified drifts by topic (both sides confirmed at the cited `path:line`):

| Spec package | Topics touched | Verified drifts |
| --- | --- | --- |
| `language` | `02`, `18`, `19` | 7 |
| `memory` | `04`, `05`, `08`, `09`, `03`/`06` | 5 |
| `linker` | `01`, `02`, `03`, `06`, `07`, `08`, `09` | 9 |
| `app` | `01`, `02`, `03`, `04` | 11 |
| `tooling` | `01`, `04`, `05`, `07`, `08` | 12 |
| `architecture` | `01`, `02`, `06`, `09`, `18`, `spec.md` | 5 |
| citations (tree-wide) | 5 topics | 2 (+13 paths, counted in `linker`/`memory`) |
| **Total** | | **51** |

Mechanical baseline, measured on this worktree:

- `[[...]]` citations: **1758 total, 13 broken** (11 file paths that do not exist —
  every one a file that became a directory; 2 bare symbols with no file component).
- `./mfb spec <pkg> <topic>` cross-links: **765 total, 0 unresolved.** The link half
  of the proposed guard is purely preventive; the citation half finds 13 today.

Reproduce both counts with the scanner in Phase 1.

## Root Cause

Two independent causes, and the fix must address both.

**1. No mechanical guard.** `.ai/specifications.md:56-58` asks a *human* to "confirm
that every `./mfb spec`/`./mfb man` link target and `[[…:Symbol]]` citation resolves"
on every spec change. Nothing enforces it. `src/docs/spec/mod.rs:107-139` tests only
package discovery, topic lookup, and summary-line extraction. The contrast is
one directory away: the error-code registry in
`src/docs/spec/diagnostics/02_error-codes.md` is *build input*, and
`src/builtins/errorcode.rs:47` (`table_matches_registry`) fails the build if the
table and the generated constants disagree — that registry has not drifted. The 13
broken citations and every stale offset table below are the predictable result of the
difference.

**2. One structural duplication.** `src/docs/spec/architecture/01_commands.md:5-79`
is a **second full copy** of the CLI surface that `tooling/07_cli-reference.md` owns
— exactly what the conventions forbid ("Each fact has one canonical topic. Other
topics give a short summary and a link — never a second full copy"). It drifted the
way a second copy always does; see E-F1.

The remaining items are ordinary per-change misses: a landed fix (bug-78, bug-186,
bug-203, bug-240, bug-119, plan-52-B, plan-41, plan-29, plan-51, plan-55) updated the
code and not the owning topic.

## Items

Ranked within each group by consequence: a reader who cannot decode a value, or who
draws an inverted security conclusion, comes before one missing a section number.

### Group A — `language/`

#### A1 — `18_builtin-functions.md` claims "exactly sixteen" always-in-scope builtins; there are **eighteen**
- `src/docs/spec/language/18_builtin-functions.md:10` ("These **sixteen** names are
  the *only* callables…") and `:36` ("All sixteen except `error` are **overridable**")
  vs `src/builtins/general.rs:42-63` (`is_general_call`).
- `is_general_call` matches 18 names. The spec table at `:14-26` lists 16 and omits
  **`toMoney`** and **`toScalar`** (`general.rs:11-12,53-54`), both of which are
  unqualified, import-free, and overridable (`is_overridable` = every name but
  `error`).
- This is a **false completeness claim** on the language's smallest and most-read
  surface: the sentence says "the *only* callables", so a reader concludes `toMoney`
  needs an `IMPORT`.
- Fix: add both rows; change both counts to eighteen.

#### A2 — grammar has no production at all for `CSTRUCT`, `DOC`, or `TESTING`
- `src/docs/spec/language/19_grammar.md:13` (`linkDecl = "LINK" string "AS" ident
  { nativeFuncDecl } "END" "LINK"`) vs `src/ast/items.rs:651` — the parser accepts
  `CSTRUCT` inside a `LINK` block (`parse_cstruct`, `items.rs:677-742`), and its own
  error text at `items.rs:663` reads "A LINK block may only contain native FUNC and
  **CSTRUCT** declarations."
- `DOC` appears in `19_grammar.md` only as prose in the header comment at `:5`; the
  parser builds `Item::Doc` at `src/ast/parser.rs:176-180`.
- **`TESTING` appears nowhere in the file** (zero matches), yet `TESTING`/`TGROUP`/
  `TCASE` are parsed at `src/ast/parser.rs:168-170` → `src/ast/testing.rs:8,36,90`.
- Fix: add `cstructDecl` to `linkDecl`, and `docBlock` + `testingBlock` productions
  to `declaration`.

#### A3 — `abiSlot` grammar omits the `IN` and `INOUT` directions
- `19_grammar.md:49` — `abiSlot = ( ident | "return" ) [ "OUT" ] nativeType ;` vs
  `src/ast/items.rs:1151-1159`, which matches `INOUT`, then `OUT`, then an optional
  `IN` keyword (plan-50-E).
- Two facts the grammar hides, both load-bearing: the match **order** (`INOUT` shares
  a prefix with `IN`, and the code comment at `items.rs:1147-1148` says so), and that
  `IN` is a **keyword** (`self.match_keyword(Keyword::In)`) while `OUT`/`INOUT` are
  contextual identifiers (`match_identifier_ci`) — a real lexical distinction.
- Fix: `abiSlot = ident [ "IN" | "OUT" | "INOUT" ] nativeType ;` plus a note on the
  keyword/identifier split.

#### A4 — grammar still admits the removed `"return"` ABI slot name
- `19_grammar.md:44` (`nativeFree`), `:49` (`abiSlot`), `:53` (`abiReturn`) each
  admit `( ident | "return" )` vs `src/ast/items.rs:1200` (`parse_abi_slot_name`),
  which is `self.consume_identifier(...)` and nothing else.
- The code's **own doc comment** at `items.rs:1196-1199` records the deletion:
  "plan-50-H deleted the `return` special case… `return` carries no meaning here and
  is simply a keyword the parser will not accept as a name." The comment at
  `19_grammar.md:50-52` compounds this by *explaining* the removed behavior.
- Converges with `bug-300-E4`, which cites the same three lines — this document
  supersedes that item.
- Fix: all three productions become plain `ident`; delete the `:50-52` comment.

#### A5 — grammar requires parens and a return type on `FUNC`/`SUB`; the parser makes both optional
- `19_grammar.md:71` (`funcDecl … "(" [ params ] ")" returnType`) and `:73`
  (`subDecl … "(" [ params ] ")"`) vs `src/ast/items.rs:65-76` — the parameter list
  is `if self.match_kind(TokenKind::LParen) { … } else { Vec::new() }` — and
  `items.rs:79-89`, where the return type is taken only `if … self.match_keyword(
  Keyword::As)`.
- So `FUNC f AS Integer` and `SUB main` both parse; the grammar rejects them.
- Fix: `funcDecl = … [ "(" [ params ] ")" ] [ returnType ] …`.

#### A6 — `constPin` grammar omits the `SIZEOF <CSTRUCT>` form
- `19_grammar.md:33` — `constPin = "CONST" ident "=" expr ;` vs
  `src/ast/items.rs:1223-1236` (`parse_const_pin`), which special-cases
  `SIZEOF <CStructName>` **before** falling through to `parse_expression`.
- `expr` does not cover it: `SIZEOF` takes a **CSTRUCT type name**, not a value
  (`consume_identifier("SIZEOF requires a CSTRUCT name.")`), and lowers to a synthetic
  `Expression::Unary { operator: "SIZEOF", … }`.
- Fix: `constPin = "CONST" ident "=" ( "SIZEOF" ident | expr ) ;`.

#### A7 — `02_lexical-structure.md` has two sections numbered 2.3
- `src/docs/spec/language/02_lexical-structure.md:58` (`## 2.3 Scalar literals`) and
  `:66` (`## 2.3 \`DOC\` blocks`); `:70` then resumes at `## 2.4`.
- Fix: renumber `DOC` blocks to 2.4 and the following section to 2.5.

### Group B — `memory/`

#### B1 — `05_collections.md` element type-id table omits `Money` (8) and `Scalar` (9), and `Scalar`'s alignment
- `src/docs/spec/memory/05_collections.md:76-88` (the `keyType`/`valueType` table:
  0, 2, 3, 4, 5, 6, 7, then jumps to 20/21/22) vs
  `src/target/shared/code/error_constants.rs:809` (`COLLECTION_TYPE_MONEY = 8`) and
  `:813` (`COLLECTION_TYPE_SCALAR = 9`), both produced by
  `src/target/shared/code/type_utils.rs:71-72` (`collection_type_code`).
- Neither word appears anywhere in the topic.
- **This is the worst decoding hazard in the cluster.**
  `src/target/shared/code/builder_collection_layout.rs:60-73`
  (`collection_payload_alignment`) gives `Scalar` a **4-byte payload at alignment 4**
  — the only element type that is neither the 1-byte (`Boolean`/`Byte`/`String`) nor
  the 8-byte (`Integer`/`Float`/`Fixed`/`Money`/pointer) case the table's shape
  implies. A reader decoding a `List OF Scalar` from this spec mis-strides every
  element.
- Also worth a note: the neighbouring `package/04` uses a **different** id space
  (wire ids) with nothing recording that the two spaces differ.
- Fix: add both rows, add an alignment column (or an explicit `Scalar` = 4-byte/align-4
  note), and cross-reference the wire-id space.

#### B2 — `04_arenas.md` says resources are excluded from scope-drop frees; drop now reclaims STATE and both buffers
- `src/docs/spec/memory/04_arenas.md:314-316` — "Three classes of value are
  **excluded** from scope-drop frees… **resources** (a move-only handle to the single
  arena-global instance, reclaimed by its own close op)" — vs
  `src/target/shared/code/builder_codegen_primitives.rs:1588-1660`.
- Since plan-52-B Phase 2 the drop path `arena_free`s the resource's **output
  buffer, read buffer, and `STATE` payload**, nulling each pointer word as it goes,
  and skips `RESOURCE_MOVED_BIT`. Only the 80-byte record survives, deliberately, as
  the tombstone holding the closed flag. The code's doc comment is explicit: "Runs at
  drop, never at close… Close releases the OS handle; drop reclaims memory."
- `language/15` already documents the close/drop split correctly, so `memory/04` is
  the stale side.
- Fix: replace the resource bullet with the close/drop split and the tombstone rule.

#### B3 — `09_closures.md` describes the pre-bug-78 per-evaluation `FunctionRef` allocation
- `src/docs/spec/memory/09_closures.md:13` ("The object is allocated with
  `arena_alloc(16, 8)` from the constructing scope's arena"), `:23-24` ("builds the
  16-byte object with `code = <symbol>` and `env = 0` (the `x31`/zero register is
  stored into the env word)"), and `:127-129` ("Because each *evaluation* of a
  `FunctionRef` or `Closure` allocates a fresh 16-byte object… repeatedly **creating**
  new function values inside a loop accumulates arena memory") vs
  `src/target/shared/code/builder_values.rs:357-375`.
- The `NirValue::FunctionRef` arm **allocates nothing**. It loads the address of a
  **static BSS descriptor** (`closure_descriptor_symbol`,
  `src/target/shared/code/error_constants.rs:258-264`) via `DataAddrHi`/`DataAddrLo`.
  The code comment states the motive verbatim: "Load its address instead of
  arena-allocating a fresh descriptor on every evaluation, so a lambda in a loop no
  longer grows the arena (bug-78)."
- The `:127-129` loop-accumulation caveat is therefore **wrong for `FunctionRef`** and
  now applies only to a capturing `Closure`.
- Fix: rewrite `:13` and `:23-24` for the static-descriptor path; scope the loop
  caveat to capturing closures.

#### B4 — the 80-byte resource record has no layout section anywhere, though another topic defers to one
- `src/docs/spec/memory/03_heap-values.md` has sections for `Standalone String`
  (`:21`), `Record` (`:39`), `Error`/`ErrorLoc` (`:85`), `Result` (`:108`), and
  `Union` (`:126`) — and **none** for the resource record.
- `src/docs/spec/memory/06_native-calling-convention.md:92-93` sends the reader there:
  "The byte layouts behind a `Reference` are owned by `./mfb spec memory heap-values`."
- `src/docs/spec/language/17_native-libraries.md:141` compounds it by citing "a real
  80-byte resource **record**" as if the layout were documented.
- The layout exists and is fully constrained in
  `src/target/shared/code/error_constants.rs:652-712`:
  `RESOURCE_RECORD_SIZE_BYTES = 80` (`:696`), `FILE_OFFSET_FD = 0`,
  `FILE_OFFSET_CLOSED = 8` (a **u64 flag set** — bit 0 `closed`, bit 1 `moved`, 62
  spare, per the comment at `:712-715`), `FILE_OFFSET_STATE = 16`, plus the plan-14-B
  output-buffer and read-buffer fields, all held in place by `const _: () = assert!`s.
- Fix: add a `## Resource Record` section to `03_heap-values.md`.

#### B5 — `08_program-startup.md` documents a partial arena clear, names two constants that do not exist, and omits the closure-descriptor init step
Three confirmed sub-drifts in one topic.
- **Partial clear.** `:24-28` says the shim "explicitly clears the header words it
  depends on… arena-state offsets `0`/`8`/`16`/`24`, the free-list head at offset
  `48`…, the cleanup-failure audit triple at `64`/`72`/`80` (only when…)". The code at
  `src/target/shared/code/entry_and_arena.rs:128-142` zeroes **the whole range** in a
  loop from `ARENA_STATE_REGISTER` to `+ ARENA_STATE_SIZE`, and its comment explains
  why the totality is load-bearing: it "must stay in lockstep with the thread-spawn
  child-state zeroing… both zero exactly `ARENA_STATE_SIZE`, so growing the state
  (e.g. quick bins) can never leave a field as garbage in one path but not the other."
- **`ENTRY_ARGC` does not exist.** `:95` names it as the seed scratch slot; the
  symbol has zero occurrences in `src/`. The real one is `ENTRY_SEED_SCRATCH_OFFSET`
  (`entry_and_arena.rs:206-212`).
- **`ENTRY_ERROR_SEPARATOR` does not exist.** `:134` names it beside
  `ENTRY_ERROR_PREFIX`/`ENTRY_ERROR_NEWLINE`; zero occurrences in `src/`.
- **Missing step.** The closure-descriptor initializer runs from the entry, once,
  **before the LINK branch** (`entry_and_arena.rs:303-312`, calling
  `CLOSURE_DESC_INIT_SYMBOL` = `_mfb_closure_desc_init`, wired at
  `src/target/shared/code/mod.rs:752-768`). The word "closure" appears in this topic
  only at `:175`/`:183` in an unrelated register-naming context — the startup step is
  entirely absent from the documented sequence.
- Fix: describe the full-range clear (and why totality matters), correct the two
  constant names, and insert the closure-descriptor init step before the LINK branch.

### Group C — `linker/`

#### C1 — all three per-arch topics say `ET_EXEC` at base `0x400000`, and two deny a PIE path; the encoder has emitted PIE at base 0 since bug-186
- `src/docs/spec/linker/07_linux-aarch64.md:22-24`,
  `08_linux-x86_64.md:25-27`, `09_linux-riscv64.md:28-31` — all three state "image
  base `0x400000` … The ELF header is `ET_EXEC`". `08:27` and `09:31` add, in bold,
  **"There is no `ET_DYN`/PIE path."**
- `src/os/linux/link/elf.rs:255-257` writes `e_type = 3` (`ET_DYN`) with the comment
  "ET_DYN: a position-independent executable the loader maps at a random base
  (bug-186). macOS is already PIE; this brings Linux in line." The base is
  `src/os/linux/link/mod.rs:17` — `const DYN_IMAGE_BASE: u64 = 0`.
- **Highest-consequence item in the cluster.** ASLR posture is exactly what a reader
  consults a linker spec for, and the spec asserts the inverse of the truth in bold.
- Precision required in the fix: `mod.rs:16` records that "The static (import-less)
  path keeps `IMAGE_BASE` / `ET_EXEC` for now" — so the old claim is still true for
  the *static* path. The defect is asserting it universally and denying the dynamic
  PIE path.
- Fix: document both paths (dynamic → `ET_DYN` @ `DYN_IMAGE_BASE = 0`; static →
  `ET_EXEC` @ `IMAGE_BASE = 0x400000`); delete both "no ET_DYN/PIE path" sentences.

#### C2 — `DT_FLAGS_1 = DF_1_NODELETE` documented where the code emits `DT_FLAGS = DF_BIND_NOW` (both tag and flag wrong)
- `src/docs/spec/linker/07_linux-aarch64.md:64` and `08_linux-x86_64.md:51` list
  `DT_FLAGS_1 = 8 (DF_1_NODELETE)`; `09_linux-riscv64.md:98` says the tags are
  "identical to aarch64 (… `DT_FLAGS_1 = DF_1_NODELETE` …)".
- `src/os/linux/link/elf.rs:841` — `put_dynamic(&mut bytes, 30, 8)`. Tag **30 is
  `DT_FLAGS`**, not `DT_FLAGS_1` (which is `0x6ffffffb`); within `DT_FLAGS`, value
  `8` is **`DF_BIND_NOW`**, not `DF_1_NODELETE`.
- Both halves of the documented pair are wrong, and the semantics are unrelated
  (eager binding vs. un-unloadability) — another security-relevant misstatement.
- Fix: `DT_FLAGS = 8 (DF_BIND_NOW)` in all three topics.

#### C3 — `DT_RUNPATH` and `PT_GNU_RELRO` are emitted but appear in none of the three enumerations
- Zero occurrences of `RUNPATH` or `RELRO` anywhere in `src/docs/spec/linker/`.
- `PT_GNU_RELRO` is emitted at `src/os/linux/link/elf.rs:355-362` (bug-263 / LNK-03:
  "the loader `mprotect`s this range back to R"), and the header comment at `:222`
  enumerates the real program-header set: "PHDR, INTERP, LOAD(text), LOAD(data-rw),
  DYNAMIC, **GNU_RELRO**, GNU_STACK, NOTE".
- `DT_RUNPATH` is emitted at `elf.rs:589,656`, with `:821` noting "DT_RUNPATH is tag
  29, not DT_RPATH (15)". The topics document 7 program headers; 8–9 are emitted.
- `PT_GNU_RELRO` is the RELRO hardening mitigation — again security-relevant, and
  again invisible to a spec reader.
- Fix: add both to all three enumerations, with the vendoring condition on `DT_RUNPATH`.

#### C4 — `06_macos-aarch64.md` load-command and dylib lists are incomplete and the `__DATA_CONST` condition is wrong
- **Load commands.** `:39-48` lists the base set plus "one `LC_LOAD_DYLIB` per
  imported library" and "`LC_DYLD_INFO_ONLY` when a `__DATA_CONST` is present". It
  omits three commands the writer emits: `LC_DYLD_CHAINED_FIXUPS` (`0x80000034`) and
  `LC_DYLD_EXPORTS_TRIE` (`0x80000033`), emitted on the **non**-`__DATA_CONST` path at
  `src/os/macos/link/macho.rs:134-144`, and `LC_RPATH`, emitted per vendor search
  path at `macho.rs:154-156` (plan-46-D §4.3) and counted at `macho.rs:393`.
- **`__DATA_CONST` condition.** `:22` and `:33-34` say `__DATA_CONST` is present
  "only if imports or initializers". The real predicate is
  `src/os/macos/link/macho.rs:50` — `has_imports || has_init || **has_rodata**` (the
  same at `commands.rs:462`). Any string constant forces one (bug-187); the spec omits
  that third disjunct.
- **Dylibs.** `:68-75` lists 6 (`libSystem`, `Network`, `AppKit`, `Foundation`,
  `libobjc`, `libz`). `src/os/macos/link/mod.rs:372-393` (`dylib_path`) resolves **9**
  — the three audio frameworks `AudioToolbox`, `CoreAudio`, and `CoreFoundation`
  (plan-33-B §5) are missing.
- Fix: three list/condition corrections in one topic.

#### C5 — `03_import-selection.md` is the declared owner of flavor soname selection and is false on x86-64
- `src/docs/spec/linker/03_import-selection.md:69-81` hardcodes the aarch64 flavor
  table: `libc.musl-**aarch64**.so.1`, and "`thread::start` imports `pthread_create`
  from `libpthread.so.0` on glibc but from `libc.musl-aarch64.so.1` on musl".
- `07` and `08` both defer to `03` for this model, so `03` is the single source of
  truth — and it is wrong on 1 of the 3 Linux targets:
  - `src/target/linux_x86_64/plan.rs:24-28` — the musl soname is
    `libc.musl-**x86_64**.so.1`.
  - `src/target/linux_x86_64/plan.rs:31-34` — `fn libpthread()` returns `self.libc()`
    **unconditionally** ("On musl and modern glibc, pthread lives in libc"), so on
    glibc/x86-64 `pthread_create` comes from `libc.so.6`, **not** `libpthread.so.0`.
    Contrast `src/target/linux_aarch64/plan.rs:22-27`, which does branch on flavor.
- Same topic, second drift: `:56` states "`io::print` / `io::write` require `write`".
  On x86-64 `write` is a raw syscall and is imported by nobody — asserted by the
  backend's own test `write_is_never_imported`
  (`src/target/linux_x86_64/plan.rs:493-505`, which lists `io.print` and `io.write`
  explicitly).
- Fix: make the flavor table per-arch and split the `write` bullet by ISA.

#### C6 — `03_import-selection.md` says a `LINK` call is `CallKind::Import`; that variant is never produced
- `:102-104` — "A call to a `LINK` function is a `CallKind::Import` referencing the
  internal thunk symbol — an internal `branch26`, not an external relocation."
- `src/target/shared/plan/mod.rs:114-120` — the `Import` variant carries
  `#[allow(dead_code)]` and the doc "**Never produced**: `import_symbols` is always
  empty (bug-139.2), so no call is ever classified as an import. The variant is
  retained only because `src/os/` object emitters still enumerate it in an exhaustive
  match." LINK routing goes through thunk symbols
  (`src/target/shared/nir/lower.rs:38-50`) and is classified `Local`.
- The spec's *conclusion* (internal branch, no external relocation, no dynamic
  dependency) is correct; only the variant name is wrong — which matters because the
  claim carries a `[[…:CallKind]]` citation inviting the reader to check it.
- Fix: `CallKind::Local`.

#### C7 — two linker topics still say Linux app mode emits `<name>.out`; it emits an `.AppImage`
- `src/docs/spec/linker/07_linux-aarch64.md:15-17` and `08_linux-x86_64.md:17-19` —
  "An app-mode build (`mfb build --app`) emits a single glibc binary,
  `build/<project>.out`".
- `src/target/linux_aarch64/mod.rs:343-352` (and `linux_x86_64/mod.rs` likewise) calls
  `os::linux::write_linked_appdir`, whose contract is
  `src/os/linux/mod.rs:73` — "Seal `build/<name>.AppDir` into
  `build/<name>.AppImage` (plan-51-C §4.4)".
- Both topics predate plan-51. `app/spec.md` and `tooling/07` are already correct, so
  these two are the stale side. Note the likely origin: the stale wording also sits in
  a source comment at `src/target/linux_aarch64/mod.rs:303-305`, 36 lines above the
  `write_linked_appdir` call that contradicts it — worth fixing in the same pass.
- Fix: both topics; and the two source comments.

#### C8 — `02_object-plan.md` documents 2 of the 4 accepted target strings
- `src/docs/spec/linker/02_object-plan.md:37` (`target "macos-aarch64" |
  "linux-aarch64"`) and `:70` ("`target` matches the platform (`"macos-aarch64"` /
  `"linux-aarch64"`)").
- `src/os/linux/object.rs:178-186` (`NativeObjectPlan::validate`) accepts
  `linux-aarch64`, `linux-x86_64`, **and** `linux-riscv64`, with the comment "The
  object/ELF plan is ISA-neutral (an ELF container is ELF); accept any Linux target".
  With the macOS validator that is 4 accepted strings against 2 documented; `08`
  already contradicts `02`, and riscv64 is unmentioned in `02` entirely.
- Fix: list all four and state the ISA-neutrality rationale.

#### C9 — 13 `[[…]]` citations do not resolve (11 stale file paths + 2 bare symbols)
Swept mechanically over all 1758 citations; every one below confirmed against the
filesystem. Eleven name a file that has since become a **directory**:

| Topic | Line | Cited path | Real |
| --- | --- | --- | --- |
| `linker/01_pipeline.md` | 18 | `src/ir.rs` | `src/ir/` |
| `linker/01_pipeline.md` | 18 | `src/target/shared/nir.rs` | `src/target/shared/nir/` |
| `linker/01_pipeline.md` | 18 | `src/target/shared/plan.rs` | `src/target/shared/plan/` |
| `linker/01_pipeline.md` | 18 | `src/arch/aarch64/encode.rs` | `src/arch/aarch64/encode/` |
| `linker/01_pipeline.md` | 18 | `src/os/link.rs` | `src/os/{macos,linux}/link/` |
| `linker/04_symbols-and-relocations.md` | 6 | `src/target/shared/nir.rs` | `src/target/shared/nir/` |
| `linker/06_macos-aarch64.md` | 5 | `src/os/macos/link.rs` | `src/os/macos/link/` |
| `linker/07_linux-aarch64.md` | 5 | `src/os/linux/link.rs` | `src/os/linux/link/` |
| `threading/05_worker-and-package-functions.md` | 22 | `src/target/shared/nir.rs:function_symbol` | `src/target/shared/nir/` |
| `threading/05_worker-and-package-functions.md` | 22 | `src/target/shared/nir.rs:symbol_fragment` | `src/target/shared/nir/` |
| `memory/08_program-startup.md` | 204 | `src/target/shared/code/abi.rs` | `src/target/shared/abi.rs` — this one **never existed** at the cited path |

`linker/01_pipeline.md:18` alone carries **five** broken citations. Two more are bare
symbols with no file component, in violation of `.ai/specifications.md:34-36`
(`[[src/file.rs:Symbol]]`): `memory/08_program-startup.md:209` cites
`[[run_register_allocation]]` and `[[finalize_vreg_body_with_locals]]` — the only 2
of 1758. `linker/09_linux-riscv64.md:8` gets the directory convention right, so the
correct form is already in the tree.

- Fix: mechanical, and Phase 1's scanner produces the exact list.

### Group D — `app/`

#### D1 — the Linux GTK char grid is `u32`/cell since bug-203, so every offset after `ST_TERM_CHARS` and `STATE_SIZE` are wrong in two topics
- `src/docs/spec/app/02_linux-runtime.md:182-188` (the offset table) and `:193-207`
  (the byte-range block, headed "`_mfb_gtkapp_state` layout (139456 bytes, align 8)")
  vs `src/target/linux_gtk/mod.rs:113-123`.
- Recomputed from the source constants (`TERM_MAX_COLS = 160`, `TERM_MAX_ROWS = 48`
  at `mod.rs:137-138`; `ST_TERM_CHARS = ST_TERM_CELL_H + 8 = 1216`; each of the six
  grid arrays is `160 * 48 * 4 = 30720` bytes):

| Symbol | Spec | Actual | Δ |
| --- | --- | --- | --- |
| `ST_TERM_CHARS` | 1216 | 1216 | 0 |
| `ST_TERM_FG` | 8896 | **31936** | +23040 |
| `ST_TERM_BG` | 39616 | **62656** | +23040 |
| `ST_TERM_SNAP_CHARS` | 70336 | **93376** | +23040 |
| `ST_TERM_SNAP_FG` | 78016 | **124096** | +46080 |
| `ST_TERM_SNAP_BG` | 108736 | **154816** | +46080 |
| `STATE_SIZE` | 139456 | **185536** | **+46080** |

  The `46080` delta is exactly the two char arrays × 7680 cells × 3 bytes: the spec
  still sizes `chars`/`snapChars` at `u8[160*48]` while the code sizes them like the
  `fg`/`bg` arrays. `mod.rs:104-112` records the reason: one byte per cell "split a
  multi-byte glyph across cells and drew each fragment as tofu (bug-203). 4 bytes
  covers every code point."
- The same stale claim, second topic: `src/docs/spec/app/04_term-backend.md:340`
  (`| ST_TERM_CHARS | u8[160*48] glyph bytes (live/back) |`) and `:347-348` ("The char
  array is 1 byte/cell (not a unichar) — ASCII-oriented, unlike the macOS u32
  glyph") — the latter is now flatly false and contradicts `04`'s own shared cell
  model at `:63-64`, which correctly says `glyph (u32 unichar…)`.
- Fix: recompute both tables and the byte-range block; delete the "1 byte/cell"
  paragraph.

#### D2 — `03_console-io.md` Linux state offsets are stale by two slots and contradict `02_linux-runtime.md`
- `src/docs/spec/app/03_console-io.md:114` (`ST_LINE_LEN` = 64), `:115`
  (`ST_LINE_BUF` = 72), `:136` ("Linux stores it at `ST_INPUT_MODE` (offset 56)") vs
  `src/target/linux_gtk/mod.rs:72,74,76` — `ST_INPUT_MODE = 72`, `ST_LINE_LEN = 80`,
  `ST_LINE_BUF = 88`.
- Every value is short by 16 bytes: `ST_ARGC` (56) and `ST_ARGV` (64) were inserted
  ahead of them for bug-240 (`mod.rs:63-69`). Offset 56 — which `03` labels
  `ST_INPUT_MODE` — is now `ST_ARGC`, so a reader following `03` reads argc as the
  input mode.
- **The two app topics disagree with each other**: `02_linux-runtime.md:197-198` has
  the correct `72 … mode`, `80 … lineLen`, `88 .. 1112 lineBuf`. Per the
  single-source-of-truth convention, `02` owns the layout and `03` should summarize
  and link rather than restate.
- Fix: correct `03`'s three values, or better, replace its table with a pointer to `02`.

#### D3 — `02_linux-runtime.md` "Documented divergences" describes a pre-fix scaffold and contradicts its own body
- `src/docs/spec/app/02_linux-runtime.md:301-305` claims "**No main-thread marshal
  for the transcript-active path**" and "the GTK transcript path is structurally
  present but **not exercised** on that path".
- `src/target/linux_gtk/app_io.rs:500-506` builds the chunk and calls
  `g_idle_add(_mfb_gtkapp_append_idle, chunk)` on **every** transcript write — the
  main-thread marshal the section says is absent.
- The same document says so **correctly** 20 lines earlier, at `:276-281`: step 2 of
  the write helper is "copy the bytes … into a `malloc` chunk `[0]=len(u64),
  [16..]=bytes`, then `g_idle_add(_mfb_gtkapp_append_idle, chunk)`."
- So the section is not merely stale — it is internally contradictory, and a reader
  cannot tell which half to trust. It describes the scaffold as it stood before the
  bug-204 fix.
- Fix: delete or rewrite the two bullets at `:301-305`.

#### D4 — Linux app mode is spec'd as aarch64-only though x86_64 is implemented and wired
- `src/docs/spec/app/02_linux-runtime.md:3-4` — "when `mfb build --app` targets
  `linux-aarch64`, the backend emits a GTK4 `_main` bootstrap…".
- `src/target/linux_x86_64/mod.rs:199-202` — `fn supports_app_mode(&self) -> bool {
  true }`, commented "GTK4 app mode (plan-05-linux-app.md), shared with linux-aarch64
  via `target::linux_gtk`" — and `src/target/linux_x86_64/plan.rs:413-417` forwards
  `app_mode_imports` to the same shared `linux_gtk` module.
- `linker/08_linux-x86_64.md` and `tooling/07_cli-reference.md` both already list
  `linux-x86_64` as app-capable, so `app/02` is the outlier.
- Fix: scope the topic to "linux-aarch64 or linux-x86_64" throughout.

#### D5 — `TermView` synthesizes 7 ObjC methods; two app topics say 5
- `src/docs/spec/app/04_term-backend.md:108` ("**Five methods** are added, then
  `objc_registerClassPair`") with a 5-row table at `:111-117`; and
  `src/docs/spec/app/01_macos-runtime.md:78-81`, which names the same five.
- `src/target/macos_aarch64/app/bootstrap.rs:233-260` adds **seven**: the five listed
  plus `mfbClear:` (`:233-240`, "main-thread grid clear (bug-165)") and
  `setFrameSize:` (`:254-260`, "the live-window-resize hook: recompute rows/cols and
  realloc the grid (plan-35-D)").
- `04` is self-inconsistent about it: `:194` devotes a whole section to
  `setFrameSize:` while `:108` says it is not one of the added methods.
- Fix: both topics — count and table rows.

#### D6 — the console shadow-grid layout omits the out buffer (which dominates the block size) and the trailer slack
- `src/docs/spec/app/04_term-backend.md:83-91` gives the block layout as header
  (`0`–`32`), `40 back cells rows*cols cells`, `... front cells rows*cols cells`, and
  stops.
- `src/target/shared/code/term_grid.rs:15-25` (the module doc that owns the layout)
  has a **third** region: `... out buffer rows*cols * OUTBUF_PER_CELL (escape-stream
  scratch)`. `OUTBUF_PER_CELL = 64` (`term_grid.rs:56`) against `CELL_SIZE = 16`
  (`:45`), so the out buffer is **64 bytes/cell against 32 for back+front combined**
  — it is the majority of the block, and a reader sizing the allocation from the spec
  undershoots by ~2×.
- Also omitted: `TRAILER_SLACK = 64` (`term_grid.rs:63`), reserved past the exact
  `rows*cols*OUTBUF_PER_CELL` "so the fixed trailing reset/CUP/cursor sequence
  (~24 bytes) … has headroom even on a near-saturating repaint (bug-175 G)".
- Fix: add both regions and the 16-byte cell size to the spec's layout block.

#### D7 — `02_linux-runtime.md` says the worker gets `argc=0/argv=NULL`; it gets the real ones
- `:109-110` — "If `spec.language_entry_accepts_args`, it passes `argc=0/argv=NULL`;
  argv is **not plumbed through to the worker**."
- `src/target/linux_gtk/bootstrap.rs:304-309` — under exactly that condition, the
  worker shim loads `ST_ARGV` into `x1` and `ST_ARGC` into `x0`.
- The same topic documents the mechanism correctly at `:140-141`
  (`| 56 | ST_ARGC | process argc, for an arg-accepting entry |`), so this is a third
  internal contradiction in `app/02`.
- Fix: rewrite `:109-110`.

#### D8 — `02_linux-runtime.md` says the app-mode `io::flush` returns OK with no work; it posts a redraw
- `:287-288` — "The app-mode `io` flush helper returns OK immediately without a
  marshaled drain."
- `src/target/linux_gtk/app_io.rs:545-560` (`emit_app_io_flush_helper`) gates on
  `emit_gtk_term_active_gate` and, while TUI mode is on, calls
  `g_idle_add(TERM_REDRAW_IDLE_SYMBOL, 0)` — i.e. it **presents the frame**. The
  "returns OK immediately" description holds only for the TUI-off path.
- Fix: qualify by TUI state.

#### D9 — `02_linux-runtime.md` says term init blanks the char grid to spaces; it blanks to 0
- `:243` — "…and blanks the char grid **to spaces**."
- `src/target/linux_gtk/term_draw.rs:364-365` — "clears to 0 rather than `' '` —
  `memset` writes whole bytes, so `' '` over u32 cells would pack FOUR spaces per
  cell; the draw skips 0 (bug-203)." `src/target/linux_gtk/mod.rs:110-112` states the
  same invariant: "A blank cell is 0, not `' '`."
- A direct consequence of the same bug-203 change as D1, missed in the prose.
- Fix: "blanks the char grid to 0 (not `' '`)".

#### D10 — the app id is sanitized on Linux and used verbatim on macOS; no topic records the divergence
- `src/target/linux_gtk/mod.rs:860-875` (`gtk_app_id`) replaces every byte that is not
  `[A-Za-z0-9_]` with `_` — "Every other byte — `-`, `.`, a space, or any non-ASCII
  scalar — becomes `_`" — then prefixes `dev.mfbasic.`.
- `src/os/macos/link/mod.rs:208-221` (`app_info_plist`) interpolates the project name
  **verbatim** into `dev.mfbasic.{name}`.
- So for any project name containing `-`, `.`, or a space the two platforms produce
  **different bundle/app identifiers**, and neither `app/01` nor `app/02` mentions it.
- Fix: document the sanitization rule in `app/02` and the divergence in whichever
  topic owns app identity.

### Group E — `tooling/`

#### E1 — the `resources` manifest section is fully implemented and validated but absent from the manifest schema
- `src/docs/spec/tooling/01_project-manifest.md` — **zero** occurrences of
  "resources"; the field table at `:27-44` has no row for it, and the
  "only these fields are validated" sentence at `:71-77` enumerates
  `name`/`version`/`mfb`/`entry`/`author`/`url`/`icon`/`kind`/`mode`/`sources` and
  says the rest "are **not** schema-checked here".
- Both halves are false. `src/manifest/mod.rs:111` calls `validate_resources`
  (defined `:401`) and `:115` calls `validate_libraries` (defined `:531`) — so two
  validators the sentence denies exist do run, and `resources` is a first-class,
  schema-checked section (plan-55-A) consumed at `src/cli/build.rs:1787`.
- Worse, another topic **cross-references into the gap**:
  `src/docs/spec/stdlib/14_os.md:169-172` documents `os::resourcePath` as returning
  "the … path of a resource the build copied out of the project's manifest
  `resources` section (see `./mfb spec tooling project-manifest`)" — sending the
  reader to a page that never mentions the word.
- Fix: add the `resources` row + a *Resource Entries* subsection; correct the two
  "only … are validated" sentences to include `validate_resources` and
  `validate_libraries`.

#### E2 — `--unsigned` is implemented and in `--help` but absent from the CLI reference
- `src/cli/build.rs:196` parses it; `:101-105` documents it as the opt-in for
  "building against unsigned dependencies"; `:1234` is the refusal it relaxes
  ("package `{name}` is unsigned but its source is not local; pass --unsigned to
  allow it"); `src/main.rs:168` advertises it in `mfb build --help`.
- `src/docs/spec/tooling/07_cli-reference.md` — zero occurrences of `--unsigned`
  (the word "unsigned" appears only in signature-type prose at `:305,314,345,386`).
- `src/docs/spec/package/12_verifier-rules.md:51` **does** mention it, so the CLI
  reference is the sole gap — on the one flag that weakens a supply-chain check
  (audit-1 PKG-01). This is the worst kind of documentation gap: the security
  control is documented, the escape hatch is not.
- Fix: add the flag row with its exact relaxation semantics.

#### E3 — the audit JSON renderer is documented as escaping what it does not, and the text renderer as escaping less than it does
- `src/docs/spec/tooling/04_audit-format.md:31-35` — "Both renderers escape untrusted
  strings. The text renderer replaces **every control character** … with
  `\u{XXXX}`… The JSON renderer escapes **the same characters** as `\u00xx`."
- Wrong in **both directions**:
  - Text renderer **does more**. `src/audit/text.rs:11-13` forwards to
    `src/terminal_safe.rs:25-41`, whose `is_terminal_unsafe` is
    `ch.is_control() ||` a bidi/format set — `U+061C`, `U+200B..200F`,
    `U+202A..202E`, `U+2060..2064`, `U+2066..2069`, `U+FEFF` (bug-210). The spec
    understates it as controls-only.
  - JSON renderer **does less**. `src/audit/json.rs:72-86` (`write_string`) escapes
    only `"`, `\`, `\n`, `\r`, `\t`, and `c < 0x20`. It escapes **no** C1 control and
    **no** bidi override.
- Concretely: a crafted package name containing **U+202E (RIGHT-TO-LEFT OVERRIDE)**
  is neutralized on the text path and passes through the JSON path **verbatim** —
  the opposite of what the spec promises to a downstream consumer rendering that JSON.
- Fix: describe each renderer's real character class separately; consider filing the
  JSON under-escaping as its own defect.

#### E4 — the audit TEXT format is entirely unspecified though the topic claims to own "two output formats"
- `src/docs/spec/tooling/04_audit-format.md:6-7` — this topic "owns the
  reimplementable detail of its **two output formats** and its analysis model".
  `:20-22` lists `--format text` (the **default**) and `--format json`.
- The topic's headings are `Invocation and Exit Status`, `JSON Document Shape`,
  `Object schemas`, `Finding Catalogue`, `Analysis Model`, `See Also`. There is **no
  text-format section at all**.
- `src/audit/text.rs:19-258` renders a fixed multi-section report plus four
  `lockfile_state` strings, none of it specified. The default output of a shipped
  command is undocumented while the doc asserts it is documented.
- Fix: add a `## Text Report Shape` section, or narrow the `:6` claim to JSON and say
  the text form is unstable.

#### E5 — the documented fallible-call table omits an entire arm
- `src/docs/spec/tooling/04_audit-format.md:253-260` gives the rule as exactly two
  arms: `fallible builtin packages: fs, io, json, net, thread, tls, http` /
  `otherwise: callee ∈ fallible-user-function set`.
- `src/audit/collect/source.rs:630-639` (`is_fallible_call`) has **three**: the
  package match, then `is_fallible_builtin(callee)` (`:645-684`) — a 30-entry list of
  specific `crypto.*` and `datetime.*` builtins added by bug-96 precisely because "a
  coarse package match would over-report" — then the user set.
- A reader reimplementing the audit from the spec produces different output on any
  program calling `crypto.ed25519Sign`, `datetime.parse`, etc.
- Scope note (see "Dropped leads"): the sibling **resource-producer** table (`:266-276`)
  and **capability-inference** table (`:236-247`) were both checked line-by-line
  against `resource_producer` (`source.rs:688-702`) and `builtin_capability`
  (`source.rs:539-588`) and **match**. Only this one arm is missing.
- Fix: add the `is_fallible_builtin` arm and its list (or a pointer to it).

#### E6 — `08_auditability.md` omits two shipped capabilities and lists one unimplemented analysis as shipped
- `src/docs/spec/tooling/08_auditability.md:26-28` — capabilities are "filesystem,
  network, process, environment, clock, randomness, or native-library access".
  Missing: **terminal** and **threads**, both shipped —
  `src/audit/collect/source.rs:545-546` maps `io` → `terminal` and `thread` →
  `threads`, and `src/audit/collect/findings.rs:197-198` emits `AUDIT-PERM-TERMINAL`
  and `AUDIT-PERM-THREADS`. `tooling/04` catalogues them correctly, so `08` is the
  stale side.
- `:29-32` lists "Confusing identifier similarity in dense or security-sensitive
  code" among what the audit reports. A grep of `src/audit/` for
  `similar`/`confusab` returns **zero** hits — the analysis is not implemented, and
  the surrounding prose is written in the indicative.
- Fix: add the two capabilities; move the similarity paragraph to a clearly-marked
  future-work note or delete it.

#### E7 — `05_fmt.md` contradicts itself and `fmt.rs` on the whitespace guarantee
- `:5-6` — "Everything else — **intra-line spacing**, string contents, comments,
  blank lines, and `DOC`/`LINK` block bodies — is preserved **byte-for-byte**."
- The same document then describes two transformations that are not byte-for-byte
  preservation: `:129` ("The **first** physical line is **trimmed** and re-indented")
  and `:130-131` ("**Continuation** lines keep their original leading whitespace
  (**only trailing whitespace is stripped**)"), matching `src/fmt.rs:89,98,601,613`.
- `:41` — "there are **no tabs** in output" — cannot hold simultaneously with
  `:130-131`, since a continuation line's preserved leading whitespace may contain tabs.
- `:242` — LINK blocks are re-indented "with **all text and casing preserved**" —
  but `src/fmt.rs:613` emits a hardcoded uppercase `"END LINK"`
  (`out.push(format!("{}END LINK", indent_str(base, width)))`), while `END FUNC` is
  passed through verbatim. The two block kinds are treated inconsistently and the spec
  documents only one behavior.
- Fix: state the real guarantee (leading/trailing whitespace is normalized per the
  first-line/continuation rule; everything else preserved), and either preserve
  `END LINK` casing or document the exception.

#### E8 — `mfb fmt` has no handling for `TESTING` blocks and neither does the spec
- `src/fmt.rs:402-431` (the keyword→`Op` mapping) has arms for `If`, `Case`, `End`,
  `Next`/`Wend`/`Loop`, `Func`, `Sub`, `For`, `While`, `Do`, `Type`, `Union`, `Enum`,
  `Match`, `Trap` — and **no `K::Testing` arm**, so `TESTING` falls through to
  `_ => None` at `:430`. The `Block` enum has no `Testing` variant either.
- `src/docs/spec/tooling/05_fmt.md` never mentions `TESTING`; its `Block` variant list
  at `:147-148` enumerates the same 12 the code has.
- So the spec is *accurate* about the formatter and both are wrong about the language
  — `TESTING`/`TGROUP`/`TCASE` are parsed constructs (`src/ast/testing.rs`). Recorded
  here for the spec side; the formatter defect (an `END TESTING` popping an empty
  stack, flattening committed acceptance sources) is a **separate live bug** and must
  be filed and fixed on its own, not papered over by a spec sentence.
- Fix (this bug): once the formatter handles `TESTING`, document the block. Do **not**
  document the current flattening as intended behavior.

#### E9 — eight-plus per-command `--help` screens exist; the spec documents a two-tier surface
- `src/docs/spec/tooling/07_cli-reference.md:23-27` — "The usage block is
  **two-tier**: the top-level screen advertises only the common `pkg` … and `repo` …
  commands; `mfb pkg --help` and `mfb repo --help` list each family in full".
- `src/main.rs` defines **eleven** help constants: `INIT_HELP:80`,
  `INIT_PKG_HELP:88`, `PKG_HELP:96`, `REPO_HELP:120`, `BUILD_HELP:153`,
  `TEST_HELP:182`, `FMT_HELP:196`, `AUDIT_HELP:208`, `DOC_HELP:217`, `MAN_HELP:228`,
  `SPEC_HELP:241` — a per-command surface, not a two-tier one.
- Fix: describe the real per-command help surface and list which commands have one.

#### E10 — `mfb man --all` is undocumented and the documented arity rule contradicts it
- `src/docs/spec/tooling/07_cli-reference.md:65` gives the syntax as
  `mfb man [package] [function]` — no flags — and `:416-418` states the rule: "zero
  args print the package index, one arg a package's function/topic listing, two args a
  single function page; an unknown package/function **or more than two args exits 2**".
- `src/cli/man.rs:9,13` parse `--all`; `:23-24` handle `mfb man --all` (whole manual),
  `:33-34` handle `mfb man <pkg> --all`, and `:41-42` reject only the
  `--all`-plus-function combination. So `mfb man io --all` is valid and reads, under
  the documented rule, as a spec violation.
- Same passage, second drift: `:416` says `show_man` "is **not width-aware**".
  `src/cli/man.rs:3` imports `detect_terminal_width`, and `:65`, `:111`, and `:119`
  all use it. Only the `--width` *flag* is absent.
- Fix: add `[--all]` to the syntax and the arity rule; replace "not width-aware" with
  "width-aware, but exposes no `--width` flag".

#### E11 — `icon` is documented as macOS-only though Linux app builds render it
- `src/docs/spec/tooling/01_project-manifest.md:68` — "`icon` is **macOS-only** — a
  Linux/GTK app build **ignores it**." (Also `:37` describes it as "the macOS app
  icon".)
- `src/os/linux/appdir.rs:65-86` renders it at **every** `HICOLOR_SIZES` entry into
  `usr/share/icons/hicolor/<N>x<N>/apps/`, plus a root copy at `:86-87`. The code's
  own comment is explicit that this is a behavior change: "a project `icon` that is
  present but not 1024×1024 now **fails a Linux build** that previously ignored it —
  intended".
- So the spec is not merely stale: it tells a user their icon is inert on Linux when
  a malformed one now **fails the build**.
- `src/docs/spec/tooling/07_cli-reference.md:168-171` already documents this correctly
  — the two `tooling` topics contradict each other.
- Fix: rewrite `:68`; keep `07` as-is.

#### E12 — `--indent`'s 256 ceiling is undocumented
- `src/docs/spec/tooling/05_fmt.md:262` — "Must be a **non-negative integer**; a bad
  value errors with exit `2`."
- `src/cli/fmt.rs:72` — `const MAX_INDENT: usize = 256;` — enforced at `:78` with the
  message "mfb fmt --indent must be between 0 and {MAX_INDENT} (got `{value}`)".
- Fix: "an integer in `0..=256`".

### Group F — `architecture/`

#### F1 — `01_commands.md` is a stale second copy of the CLI surface `tooling/07` owns (structural)
- `src/docs/spec/architecture/01_commands.md:5-46` (commands + every `mfb build`
  flag) and `:59-79` (fmt + "Other commands") restate what
  `src/docs/spec/tooling/07_cli-reference.md` owns. `.ai/specifications.md:29-32`
  forbids exactly this: "Each fact has one canonical topic. Other topics give a short
  summary and a `./mfb spec <package> <topic>` link — **never a second full copy**."
- It drifted the way a second copy always does. Against `src/main.rs:264-511`, whose
  dispatch arms are `help`/`--help`/`-h`, `--version`/`-V`, `init`, `init-pkg`,
  `build`, `test`, `pkg`, `repo`, `machine`, `key`, `org`, `token`, `audit`, `man`,
  `spec`, `doc`, `fmt`, this topic **never mentions**:
  - `mfb test` (`main.rs:334`),
  - `--version` / `-V` (`main.rs:267`),
  - `-q`/`--quiet` and `-v`/`--verbose` (`src/cli/build.rs:208,212`),
  - `--app-debug` (`src/cli/build.rs:191`),
  - `--unsigned` (`src/cli/build.rs:196` — cf. E2),
  - four top-level commands: `machine` (`main.rs:390`), `key` (`:409`), `org`
    (`:428`), `token` (`:447`).
  Its `:73` also lists `mfb repo register|auth` while `src/cli/repo.rs` dispatches
  `register`, `auth`, `trust`, `link`, and `rotate` (`:34,48,63,84,131`).
- **This is the structural fix of the cluster.** Do not merely refresh the lists —
  that recreates the duplication and the next drift. Cut the topic down to its
  *unique* content (build modes and the `buildMode` → artifact mapping, `:35-42`,
  `:48-56`) and replace both enumerations with a pointer to
  `./mfb spec tooling cli-reference`.

#### F2 — `06_native.md` documents an abstraction the default path bypasses, a heuristic that was deliberately removed, and omits a helper family
Three confirmed drifts in one topic.
- **`AllocationStrategy` is bypassed.** `:315` and `:327-337` present it as the
  swappable seam ("The allocation method is a swappable `AllocationStrategy`, selected
  by the `--regalloc <name>` build flag… Further strategies (graph-coloring) slot in
  without touching the rewrite pass or the register model"). In
  `src/target/shared/code/regalloc/mod.rs:242-258`, only the `BumpAndReset` arm goes
  through the trait (`BumpAndReset.assign(&AllocInput { … })`); the
  **`LinearScan` arm — the default — never calls `assign` at all** and runs its own
  per-class passes. The spec asserts an extensibility property the default path
  disproves.
- **Removed heuristic still documented.** `:133-136` documents the "Thread `.result`
  member access" rule as a live helper-detection input.
  `src/target/shared/runtime/usage.rs:245-252` deleted it: "**No `.result`
  heuristic**: `Thread.result` was removed from the language
  (`TYPE_THREAD_RESULT_REMOVED`), so every surviving `.result` is a user record/enum
  field access — declaring the Thread helper for it was a pure false positive that
  rejected valid programs (bug-119)."
- **`audio` missing from the helper-family list.** `:56-69` lists 13 families from
  `crypto` to `tls`. `src/target/shared/runtime/mod.rs:4-18` — `enum RuntimeHelper` —
  has 14, and **`Audio` is variant #1**, ahead of `Crypto`.
- Fix: three edits; the `AllocationStrategy` one should say what is actually true
  (one trait-based reference strategy, one bespoke default) rather than describing an
  aspiration.

#### F3 — `18_math-kernels.md` names 2 backends for a regression test where 3 exist and x86 has none
- `src/docs/spec/architecture/18_math-kernels.md:22-25` — the no-libm determinism
  contract is "verified by the `no_libm_math_imports` regression test in **both**
  `[[src/target/macos_aarch64/plan.rs]]` and `[[src/target/linux_aarch64/plan.rs]]`".
- The test exists in **three** places — `src/target/linux_aarch64/plan.rs:434`,
  `src/target/linux_riscv64/plan.rs:430`, and the macOS one — and in **none** for
  `linux_x86_64`.
- So the spec both undercounts the guard and, more importantly, conceals that the
  bit-identical-across-targets claim ("a math result is bit-identical on macOS,
  Linux-glibc, and Linux-musl") is unguarded on x86-64, the most common Linux target.
- Fix: correct the count **and** add the missing x86-64 test — this is the one item in
  the cluster whose honest fix is a code change, not a doc change.

#### F4 — `linker/01_pipeline.md` and `architecture/spec.md` still describe an aarch64-only code path
- `src/docs/spec/linker/01_pipeline.md:6` cites only `[[src/target/macos_aarch64/]]`
  and `[[src/target/linux_aarch64/]]`; `:14` and `:31` both say the code-lowering
  stage produces "concrete **aarch64** instructions".
- `src/docs/spec/architecture/spec.md:49` — "encoded **aarch64** image".
- Three ISAs ship (`src/arch/{aarch64,x86_64,riscv64}`), and the sibling `spec.md`
  files already name all three, so these are the last aarch64-only statements.
- (`linker/01_pipeline.md:18` additionally carries five of the broken citations in C9.)
- Fix: "concrete native instructions"; enumerate the three backends.

### Group G — `architecture/` frontend

#### G1 — the architecture spec claims syntaxcheck emits no semantic rules; it emits 42
- `src/docs/spec/architecture/02_frontend.md:231-234` — syntaxcheck "does **not**
  check the relocated semantic rules (operator/argument/return typing, member access,
  constructor arity, match exhaustiveness, resource ownership, literal ranges, …);
  those live only in the IR semantic verifier."
  `src/docs/spec/architecture/09_modules.md:12` restates it absolutely: "**All**
  semantic rules live in the IR semantic verifier."
- Extracting every rule name passed to `SyntaxChecker::report`
  (`src/syntaxcheck/mod.rs:2441`) across `src/syntaxcheck/*.rs` yields **42 distinct
  rules**. Three of them fall squarely inside categories the spec explicitly lists as
  relocated away:
  - **argument typing** — `TYPE_CALL_ARGUMENT_MISMATCH`,
  - **arity** — `TYPE_CALL_ARITY_MISMATCH`,
  - **resource ownership** — `TYPE_COLLECTION_OWNERSHIP_VIOLATION`.
  The rest include the whole `NATIVE_*` family (11 rules), the `TESTING_*` family
  (6), and the inline-TRAP/lambda/isolation rules.
- The distinction the spec *means* is real and is machine-readable: `report` carries a
  `debug_assert!(!crate::ir::RELOCATED_TO_IR_VERIFY.contains(&rule), …)`
  (`mod.rs:2447-2449`). Syntaxcheck may not emit a **relocated** rule; it emits plenty
  of non-relocated semantic ones.
- Fix: restate both sentences in terms of `RELOCATED_TO_IR_VERIFY`, and generate the
  "still in syntaxcheck" list from the source rather than asserting emptiness.

### Group H — citations naming symbols that do not exist

(File-path breakage is C9. These are the symbol-level ones, which the proposed guard
deliberately does **not** cover — see Fix Design.)

#### H1 — `language/12_collections.md:70` cites `src/ir/verify/mod.rs:is_comparable_with_seen`
- No such symbol in that file. `src/ir/verify/mod.rs:2138` defines
  `is_comparable_**seen**`; the `is_comparable_with_seen` spelling exists in a
  *different* module, `src/syntaxcheck/types.rs:297`.
- Both the file and the symbol are wrong, and the claim it backs (which types are
  comparable) is one a reader would want to check.

#### H2 — `package/14_compact-summary.md:128` cites `src/ir/verify/mod.rs:verify_semantics`
- That file defines no `verify_semantics`; the name exists only as a re-export,
  `src/ir/mod.rs:175` — `pub use verify::check as verify_semantics`. The real symbol
  in the cited file is `check`.
- **This is the load-bearing one.** The citation is the sole proof offered for the
  sentence "the complete semantic verifier — the same one used on the project's own
  source-lowered IR — runs over the merged package IR before native lowering", i.e.
  the claim that importing a package cannot bypass semantic verification. A reviewer
  grepping the cited file finds nothing.
- (Weaker than the sweep's original "NO SUCH SYMBOL" reading — the name does exist,
  one module up. Recorded accurately here.)

#### H3 — `package/03_metadata-encoding.md:181` cites `repository/src/abi.rs:from_project`
- The file exists; `from_project` does **not**. Its functions are `parse_vendor_blobs`,
  `read_native_vendor_locators`, `table_string`, `parse_abi_index`, `abi_index_json`,
  `read_section_table`, `read_string_pool`, `read_abi_exports`, and four `read_uN`
  helpers — all *readers*. The ABI-index *producer* the claim describes lives on the
  compiler side, in `src/binary_repr/`.

## Outcome (2026-07-19)

Every group is closed, and the two drift guards are in place and green. What
changed relative to this document:

### The guards found more than the sweep did, and less

`spec_citations_resolve` failed on its first run with **ten** broken file
citations — `src/ir.rs`, `src/target/shared/nir.rs` (×2 plus two symbol-level),
`src/target/shared/plan.rs`, `src/arch/aarch64/encode.rs`, `src/os/link.rs`,
`src/os/macos/link.rs`, `src/os/linux/link.rs`, and
`src/target/shared/code/abi.rs`. All are modules that grew into directories
(`x.rs` → `x/mod.rs`) or moved. That is C9, found mechanically rather than by
hand — which is the point of the guard.

`spec_links_resolve` needed two parser exclusions to be honest rather than
noisy: a `See Also` line reads `./mfb spec linker — description`, so the em-dash
terminates a target, and `./mfb spec language *` is prose meaning "the language
topics", not a link. Both are recorded in the test.

The symbol half of a citation is deliberately **not** guarded. H2 is exactly why:
`verify_semantics` does exist — as a re-export one module up from the file cited —
so a naive symbol grep would have called a real re-export a dangling citation
while missing that the *file* was the wrong one to cite.

### Claims that were already fixed

Re-checking each item against the tree found five that had landed since the sweep:

- **A3, A4, A5 (partially)** — `19_grammar.md` already carried
  `abiSlot = ident [ "IN" | "OUT" | "INOUT" ] nativeType`, already recorded
  plan-50-H's deletion of the `return` slot name, and already made the parameter
  list optional. Only the **return type** was still written as mandatory, which
  is the half that remained.
- **E3 (JSON half)** — bug-283-A1 already routed `audit::json::write_string`
  through `is_terminal_unsafe`, so the JSON renderer no longer under-escapes and
  the U+202E hazard described here is closed. The spec's *text*-renderer
  description was the stale half, understating it as controls-only.
- **E5** — `04_audit-format.md` already documented all three arms of
  `is_fallible_call`, including the per-builtin sets.
- **F3's code half** — `no_libm_math_imports` now exists in **all four** backends,
  `linux_x86_64` included. Only the spec's "in both" count was wrong; there was
  no missing test to add.

### Corrections to the document's own numbers

- **D6** — `OUTBUF_PER_CELL` is **72**, not 64. bug-313 raised it, recording that
  64 was the exact worst case with zero margin and only for coordinates below
  1000. The conclusion holds and is stronger: the out buffer is still the
  majority of the block.
- **D1** — the recomputed offsets in the table are correct; verified by compiling
  a probe against the real constants rather than by re-deriving them
  (1216 / 31936 / 62656 / 93376 / 124096 / 154816, `STATE_SIZE` 185536).

### The structural item

F1 was fixed by **deletion**, not refresh: `architecture/01_commands.md` went from
82 lines to 61, keeping only what it uniquely owns (the build modes and the
`buildMode` → artifact mapping) and pointing at `tooling/07_cli-reference.md` for
the command surface. Refreshing the lists would have recreated the duplication
that produced the drift, and the topic now says so in its own text.

## Goal

- Each of the 51 cited spec lines states what the implementation does, verified
  against the same `path:line` recorded here.
- `cargo test --bin mfb spec` fails if any embedded page's `./mfb spec <pkg> <topic>`
  cross-link or any `[[…]]` file component does not resolve.
- `architecture/01_commands.md` contains no second copy of the CLI command or flag
  enumerations.
- A regression test exists for the `no_libm_math_imports` gap on `linux_x86_64` (F3).

### Non-goals (must NOT change)

- **Any compiled-program behavior.** Every item except F3's missing test is a
  documentation edit. In particular: do **not** "fix" C1 by reverting to `ET_EXEC`,
  C2 by changing the emitted `DT_FLAGS`, or D1 by reverting the `u32` cell — the code
  is right and the spec is wrong in all three.
- **The tempting wrong fix, named explicitly:** do not resolve E8 by writing a spec
  sentence blessing `mfb fmt`'s current flattening of `TESTING` blocks. That is a live
  formatter defect; it gets its own bug and its own fix.
- **Symbol-level citation checking must stay manual.** The guard checks file
  components only (see Fix Design). Do not weaken it into a substring grep that would
  pass on a comment mentioning the symbol.
- The `[[ ]]` citation syntax, `PACKAGE_ORDER`, the `NN_slug.md` discovery rule, and
  the rendered output of every page that is not edited.
- The error-code registry path (`build.rs` + `table_matches_registry`) — the model,
  not a target.

## Blast Radius

Found by actually scanning, not from memory.

**In scope — the 51 items above**, spanning 30 spec files across 8 packages:
`language/{02,18,19}`, `memory/{03,04,05,08,09}`, `linker/{01,02,03,04,06,07,08,09}`,
`app/{01,02,03,04}`, `tooling/{01,04,05,07,08}`, `architecture/{01,02,06,09,18,spec.md}`,
`package/{03,14}`, `threading/05`, `stdlib/14` (the dangling pointer in E1).

**Mechanically found by the Phase 1 scanner** (subset of the above, no judgement
needed): the 13 citations in C9, plus H1–H3's file components where wrong.

**Latent, same hazard, out of scope:**

- `src/docs/man/**` — the man corpus has the same class of drift (the sweep filed it
  separately; `mfb man` pages are not `mfb spec` pages and have their own owner).
- The ~1745 citations that resolve today. The guard covers them going forward; none
  needs an edit now.
- Symbol components of all 1758 citations. Only the 3 in H were checked by hand; a
  full symbol audit is a separate, larger task and is explicitly not attempted here.

**Unaffected:**

- `src/docs/spec/diagnostics/02_error-codes.md` — build input, already guarded by
  `table_matches_registry`, and the sweep found it clean.
- `src/rules/table.rs` — independently verified clean by the sweep (233 rule names,
  zero orphans, 230 codes matching `diagnostics/01_rule-codes.md`).
- All 765 `./mfb spec` cross-links — currently 100% resolving.

## Fix Design

**The root-cause fix is Phase 1, and it comes first.** Forty-plus symptom edits with
no guard reproduces this document in a year. `.ai/specifications.md:56-58` already
*asks* for exactly the check being automated; the error-code registry
(`table_matches_registry`) is the in-tree proof that the mechanical form works and the
manual form does not.

**Phase 1 — the drift guard.** Two `#[cfg(test)]` tests in `src/docs/spec/mod.rs`,
beside the existing discovery tests at `:107-139`:

1. `spec_links_resolve` — for every embedded page, extract every
   `./mfb spec <package> [<topic>]` reference and assert it resolves against the
   in-memory `PACKAGES` table via the existing `package()`/`topic()` lookups. This
   needs **no filesystem access**, so it works from the embedded corpus alone.
   Baseline: 765 references, 0 failures — purely preventive.
2. `spec_citations_resolve` — for every `[[…]]` in every embedded page, take the
   component before the first `:` and assert (a) it is non-empty and repo-rooted
   (catching the 2 bare symbols) and (b) it exists on disk. Baseline: 1758
   citations, **13 failures** — this test fails on the current tree and passes after
   Phase 2, which makes it a genuine regression test rather than a tautology.

   Two design points. First, the on-disk check needs `CARGO_MANIFEST_DIR`, not
   `include_str!`, since the page text does not carry the paths — acceptable in a
   `#[cfg(test)]` guard, which always runs from the source tree. Second, a citation
   whose path is a **directory** must be accepted when it has no `:Symbol` suffix
   (`[[src/syntaxcheck/]]` is a valid, intentional form) and **rejected** when it does
   (`[[src/target/shared/nir.rs:function_symbol]]` where `nir` is a directory) — that
   distinction is what catches the H2/threading-05 class.

**Explicitly out of scope for the guard: symbol-level checking.** Verifying that
`[[src/ir/verify/mod.rs:is_comparable_with_seen]]` names a real item requires parsing
Rust, and a naive `contains("fn is_comparable_with_seen")` grep would be both
brittle and defeatable by a comment. `.ai/specifications.md` should be amended to say
the guard covers links and file paths mechanically and symbol names remain a human
check — turning a vague "confirm everything resolves" into a small, actually-performable
review step.

**Phases 2–4 — the symptom edits**, ordered by consequence, not by file:

- **Phase 2 (user-misleading first):** C1, C2, C3 (linker/ASLR), E2 (`--unsigned`),
  E3 (audit escaping), B1 (`Scalar` stride), D1/D2 (GTK offsets), E11 (icon).
- **Phase 3 (structural):** F1 — cut `architecture/01_commands.md` down to build
  modes + the `buildMode`→artifact mapping and replace both enumerations with a link
  to `tooling/07`. This is the one edit that *removes* text, and the one that prevents
  a recurrence of its own item.
- **Phase 4 (the remainder):** everything else, plus F3's missing x86-64 test.

**Rejected alternatives:**

- *A CI grep instead of a `#[cfg(test)]` test.* Rejected: it would not run in
  `cargo test`, would not be version-locked to the embedded corpus, and would diverge
  from the `table_matches_registry` precedent the tree already follows.
- *Auto-rewriting stale `[[…]]` paths.* Rejected: `src/os/link.rs` maps to **two**
  real directories and `src/target/shared/code/abi.rs` never existed at all — the
  correct target requires judgement per citation.
- *Deleting `architecture/01_commands.md` entirely.* Rejected: it owns real unique
  content (build modes, `buildMode`→artifact). Cut it down, do not remove it.
- *Fixing D2 by correcting `03_console-io.md`'s three numbers.* Weakly rejected in
  favor of replacing the table with a link to `02`, which owns the layout — correcting
  the copy leaves the duplication that caused the drift.

**Expected output shift:** none in any generated artifact. `mfb spec <package> --all`
renders differently for the edited pages, and no golden covers spec prose.

## Phases

### Phase 1 — the drift guard (no doc edits)

- [ ] Add `spec_links_resolve` to `src/docs/spec/mod.rs` tests. Confirm it passes at
      765/765 on the current tree (preventive baseline).
- [ ] Add `spec_citations_resolve` to the same module, with the directory/symbol rule
      above. **Confirm it fails on the current tree with exactly the 13 citations in
      C9**, listed in the assertion message.
- [ ] Record the two baselines (1758 citations / 765 links) in the test's doc comment
      so a future reader can tell growth from regression.
- [ ] Amend `.ai/specifications.md:56-58`: links and citation **paths** are
      mechanically guarded; citation **symbols** remain a human check.

Acceptance: `cargo test --bin mfb spec` fails for the 13 documented citations and for
no other reason; the link test passes.
Commit: —

### Phase 2 — user-misleading and security-relevant items

- [ ] C1, C2, C3 — `linker/{07,08,09}`: real `ET_DYN`/base-0 dynamic path vs the
      `ET_EXEC` static path; `DT_FLAGS = DF_BIND_NOW`; add `DT_RUNPATH` and
      `PT_GNU_RELRO`.
- [ ] E2 — add `--unsigned` to `tooling/07_cli-reference.md`.
- [ ] E3 — split the two renderers' escaping in `tooling/04_audit-format.md`; open a
      separate defect for the JSON under-escaping of bidi overrides.
- [ ] B1 — add `Money`/`Scalar` rows and the `Scalar` 4-byte/align-4 note to
      `memory/05_collections.md`.
- [ ] D1, D2 — recompute both GTK tables from `src/target/linux_gtk/mod.rs`; point
      `app/03` at `app/02` rather than restating.
- [ ] E11 — rewrite `01_project-manifest.md:68`.
- [ ] A1 — sixteen → eighteen, plus the `toMoney`/`toScalar` rows.

Acceptance: each edited line matches the cited implementation line; the Phase 1
citation test still passes.
Commit: —

### Phase 3 — the structural fix

- [ ] Cut `architecture/01_commands.md` to build modes + the `buildMode`→artifact
      mapping; replace `:5-46` and `:59-79` with a `./mfb spec tooling cli-reference`
      pointer.
- [ ] Verify no other topic duplicates the CLI surface (grep for `mfb build --`
      across `src/docs/spec/`).
- [ ] Confirm `tooling/07` covers everything the cut text carried that is still true.

Acceptance: `01_commands.md` contains no command or flag enumeration; nothing
`tooling/07` lacked is lost.
Commit: —

### Phase 4 — the remainder + regenerate

- [ ] Remaining Group A (A2–A7), B (B2–B5), C (C4–C9), D (D3–D10), E (E1, E4–E10,
      E12), F (F2, F4), G (G1), H (H1–H3).
- [ ] F3: add the missing `no_libm_math_imports` test to
      `src/target/linux_x86_64/plan.rs` and correct the spec's backend count.
- [ ] `cargo build` (regenerate the embedded table), `cargo test --bin mfb spec`, and
      `mfb spec <package> --all` for all 12 packages — confirm no leaked `[[` markers.
- [ ] Full acceptance suite.

Acceptance: full suite green; both guard tests pass; every item above has an edit and
a re-verified `path:line`.
Commit: —

## Validation Plan

- **Regression tests:** `spec_links_resolve` and `spec_citations_resolve` in
  `src/docs/spec/mod.rs`. The citation test is failing-then-passing (13 → 0), which
  is what makes this cluster's root-cause fix testable at all.
- **Runtime proof:** `mfb spec <package> --all` for each of the 12 packages —
  renders with no leaked `[[` markers, and the edited claims read correctly against
  the cited source.
- **Numeric spot-checks:** re-derive `STATE_SIZE = 185536` from
  `src/target/linux_gtk/mod.rs` constants (D1) and re-count `is_general_call`'s 18
  arms (A1) after editing, rather than trusting this document.
- **Doc sync:** this bug *is* the doc sync. `.ai/specifications.md` gains the guard
  note (Phase 1).
- **Full suite:** the project's acceptance run, plus `cargo test --bin mfb spec`.

## Open Decisions

- **E4 — audit TEXT format.** Specify it fully (a `## Text Report Shape` section
  covering the 14 sections and 4 `lockfile_state` strings) — *recommended*, since the
  topic claims to own it and text is the **default** format — vs. narrowing the `:6`
  claim to JSON and declaring the text form unstable. (§E4)
- **E7 — `END LINK` casing.** Change `src/fmt.rs:613` to preserve the author's casing,
  matching `END FUNC` and the spec's stated guarantee — vs. documenting the
  uppercasing as an intentional LINK-only exception. The first is a behavior change
  and belongs in the formatter's own bug if chosen. *Recommended: document the
  exception here, file the behavior question separately.* (§E7)
- **D2 — `03_console-io.md`.** Replace its Linux offset table with a link to `app/02`
  (*recommended*, per the single-source-of-truth convention) vs. correcting the three
  numbers in place. (§D2)
- **H2 — `verify_semantics` citation.** Repoint to `src/ir/mod.rs:verify_semantics`
  (the re-export the prose actually names) vs. `src/ir/verify/mod.rs:check` (the
  definition). *Recommended: the re-export*, since that is the name the sentence uses.
  (§H2)

## Dropped leads

Recorded so they are not re-litigated. Each was investigated and did **not** hold as
stated.

- **"Three audit detection algorithms are narrower in spec than in code."** Only
  **one** holds (E5, the fallible-call table). The **resource-producer** table
  (`04_audit-format.md:266-276`) matches `src/audit/collect/source.rs:688-702` row for
  row — all seven producers present, `native = false`/`closeMayFail = true` correct.
  The **capability-inference** table (`:236-247`) is a faithful summary of
  `builtin_capability` (`source.rs:539-588`), including the `os`/`math`/`datetime`
  "per builtin" split. Two of three dropped.
- **"`verify_semantics` — NO SUCH SYMBOL."** Overstated. The name exists as a
  re-export at `src/ir/mod.rs:175`; it is absent only from the *cited file*.
  Reclassified in H2 as a wrong-file citation, not a nonexistent symbol.
- **"`term::on` sets raw mode" (one of the six Linux behavioral drifts).**
  `app/03_console-io.md:25,131,145-147` describes `io::readChar`/`io::readByte`
  setting raw mode via `emit_set_raw_input_mode`, which matches
  `src/target/linux_gtk/app_io.rs:637-648`. No drift found; dropped.
- **"Redraw" (one of the six).** Could not isolate a spec claim contradicting the
  code; the redraw path is described consistently at `02_linux-runtime.md:246-248`.
  Dropped. Five of the six behavioral drifts survive as D7–D10 plus D3.
- **"~10 stale file paths."** Undercount — the actual figure is **11** stale paths
  plus **2** bare symbols (13 total), all enumerated in C9.
- **C1 nuance (not dropped, but narrowed).** The `ET_EXEC` @ `0x400000` claim remains
  **true for the static, import-less path** (`src/os/linux/link/mod.rs:16`). The
  defect is asserting it universally and denying the dynamic PIE path — the fix must
  document both, not simply invert the text.

## Summary

The engineering risk is **not** in the 51 edits — each is a localized doc change with
both sides already cited and re-checkable in seconds. It is in Phase 1 and Phase 3.

Phase 1 is where the value is: without a `#[cfg(test)]` guard, this document
regenerates itself, because `.ai/specifications.md` currently delegates the check to
human diligence on every spec commit and the 13 broken citations show what that
yields. The guard is small (two tests, no new dependencies), starts red on the exact
13 citations, and mechanically finds the path/link subset of the fixes.

Phase 3 is where the judgement is: `architecture/01_commands.md` must be **cut down**,
not refreshed. A second full copy of the CLI surface is what the spec conventions
forbid, and it is precisely why that topic never learned about `mfb test`,
`--version`, `-q`/`-v`, or four top-level commands.

Left untouched: all compiled-program behavior, the `.mfp` format, the man corpus, the
error-code registry, and the 1745 citations and 765 cross-links that resolve today.
