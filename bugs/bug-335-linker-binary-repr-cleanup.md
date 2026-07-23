# bug-335: object-writer/linker duplication and `.mfp` codec structure (os/ + binary_repr cleanup cluster)

Last updated: 2026-07-18
Effort: large (3h–1d)
Severity: LOW
Class: Other (cleanup)

Status: Open
Regression Test: none new — the guard is the existing suites
(`src/os/linux/link/tests.rs`, `src/os/macos/link/tests.rs`,
`src/binary_repr/tests.rs`) plus `scripts/artifact-gate.sh` and
`scripts/test-accept.sh`, which must stay byte-identical throughout.

The two platform object writers (`src/os/macos/object.rs`, `src/os/linux/object.rs`)
are substantially the same file written twice — **821 of their 2,461 combined lines
are common**, including two blocks that are byte-for-byte identical. The same holds
for the two AArch64 linkers: `patch_relocations`' six relocation arms and the
`branch_imm26`/`adrp_page21` encoders are structurally identical with identical
instruction-encoding constants. Separately, `src/binary_repr/` mixes encoder and
decoder code within single files (`reader.rs` carries ~380 lines of *writer* code),
drives its section table from bare `u16` consts declared out of numeric order, and
strands a public type — with a stale `#[allow(dead_code)]` — in the middle of its
public-function block.

None of this changes emitted bytes. **The single correct outcome a fix produces is:
the shared object-plan and AArch64 relocation-encoding code exists once, `binary_repr`
separates its encode and decode halves under one naming convention, and the four dead
items below are gone — with every shipped binary byte-identical before and after.**

This document deliberately EXCLUDES the linker **SPEC drift** from the same review
section (findings #13–#18: ET_EXEC vs ET_DYN/PIE, `DT_FLAGS_1`, the macOS
load-command list, the `<name>.out` app-mode claim). That is documentation
correctness, some of it security-relevant, and belongs in its own document.

References:

- `src/docs/spec/linker/01_pipeline.md`, `02_object-plan.md` — the object-plan model
  both writers implement.
- `src/docs/spec/package/03_metadata-encoding.md` — anchors `reader.rs:function_sig_hash`,
  `reader.rs:type_sig_hash`, `reader.rs:serialize_type`, `reader.rs:is_exported_function`.
  **Any file split in `binary_repr/` must re-point these anchors.**
- Found during the cleanup review (Agent 16 — binary_repr + os/linker), base `25c38ba1`.
- Converges with the `align`-duplication finding in `src/target/shared/code/type_utils.rs:221`
  (Agent 04 #13).

## HAZARD — read before touching the byte helpers

**The macOS linker's `read_u32`/`write_u32` panic where the Linux ones return `Err`.**
Verified:

```rust
// src/os/macos/link/mod.rs:615-621
fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("slice length"))
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
```

```rust
// src/os/linux/link/mod.rs:608-625
fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let slice = bytes.get(offset..offset + 4).ok_or_else(|| {
        format!(
            "linux linker: relocation offset {offset} + 4 exceeds text length {}",
            bytes.len()
        )
    })?;
    Ok(u32::from_le_bytes(slice.try_into().expect("slice length")))
}
// write_u32 likewise uses .get_mut() and returns Result<(), String>
```

Both are called from `patch_relocations` with `relocation.offset` taken straight from
the same `EncodedImage` relocation stream (`src/os/macos/link/mod.rs:250`,
`src/os/linux/link/mod.rs:165`). An out-of-range relocation offset therefore **aborts
the compiler on macOS and produces a diagnostic on Linux** — for identical input.
This is visible directly in the arm-by-arm diff: every macOS call site is
`write_u32(text, relocation.offset, word);` where Linux is
`write_u32(text, relocation.offset, word)?;`.

Unifying the byte helpers on the Linux (`Result`-returning) shape closes this as a
side effect. **Do not let that happen silently.** This is a real robustness defect —
a compiler abort instead of a diagnostic — and deserves its own bug with its own
regression test (an `EncodedImage` whose relocation offset exceeds `text.len()`,
asserting `Err` on both platforms). File it before or alongside item A4; this cleanup
must not be the changelog entry for a panic fix.

> **Resolved 2026-07-22 by bug-351**
> (`bugs/completed-bugs/bug-351-macos-linker-relocation-panic.md`). The macOS
> `read_u32`/`write_u32` now return `Result` and every `patch_relocations` call
> site `?`-propagates, matching the Linux shape above. The behavior is guarded by
> `os::macos::link::tests::patch_relocations_rejects_out_of_range_offset` (bisected:
> panics pre-fix, diagnoses post-fix). So item A4 may now collapse the two helpers
> together freely — the contract is test-pinned and the panic fix is already its
> own changelog entry.

## Current State

Measured on branch `main`, base `25c38ba1`.

### The two object writers are 821 common lines

```
$ wc -l src/os/macos/object.rs src/os/linux/object.rs
    1410 src/os/macos/object.rs
    1051 src/os/linux/object.rs
    2461 total

$ diff src/os/macos/object.rs src/os/linux/object.rs | grep -c '^<'   # 589
$ diff src/os/macos/object.rs src/os/linux/object.rs | grep -c '^>'   # 230
```

589 macOS-only + 230 Linux-only = 819 differing; 1410 − 589 = 1051 − 230 = **821 lines
in common**.

Two blocks diff to nothing at all:

```
$ diff <(sed -n '31,101p' src/os/macos/object.rs) <(sed -n '29,99p' src/os/linux/object.rs)
# (no output — the ten plan structs are byte-identical)

$ diff <(sed -n '733,760p' src/os/macos/object.rs) <(sed -n '532,559p' src/os/linux/object.rs)
# (no output)
```

The second block, verbatim from `src/os/macos/object.rs:733-757` (identical at
`src/os/linux/object.rs:532-556`):

```rust
fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn reject_duplicates(label: &str, values: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(format!(
                "native object plan has duplicate {label} '{value}'"
            ));
        }
    }
    Ok(())
}

fn align(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

trait ToObjectJson {
    fn to_json(&self, indent: usize) -> String;
}
```

`join_json` differs **only in its parameter name** — `items` at
`src/os/macos/object.rs:937`, `values` at `src/os/linux/object.rs:736`; bodies are
otherwise character-identical.

### Dead code, confirmed by the compiler

```
$ cargo check --message-format=short 2>&1 | grep 'never used'
src/os/linux/appimage/mod.rs:40:7: warning: constant `RUNTIME_X86_64_LEN` is never used
src/os/linux/appimage/mod.rs:41:7: warning: constant `RUNTIME_AARCH64_LEN` is never used
src/os/linux/squashfs.rs:136:19: warning: associated function `dir` is never used
```

These three warnings are emitted on every non-test build today.

Separately, deleting `#[allow(dead_code)]` from `src/binary_repr/mod.rs:465` and
re-running `cargo check` produced **zero new warnings** — proving the attribute is
stale. (The file was restored; no edit is staged.)

## Root Cause

Three independent mechanisms, none of them a logic error:

1. **Per-platform files were seeded by copy.** `src/os/linux/object.rs` was created
   from `src/os/macos/object.rs`; the ELF writer still carries the Mach-O struct name
   `LoadCommandPlan` (`src/os/linux/object.rs:29`) and puts `"PT_LOAD"` into
   `SectionPlan.segment` (`:116`, `:125`). No shared object-plan module was ever
   extracted, so the ISA-neutral half (plan structs, JSON emission, dedup helpers) has
   no home and lives twice.

2. **`binary_repr` has no encode/decode seam.** Files are named by *role*
   (`reader.rs`, `writer.rs`, `sections.rs`) but populated by *subject*, so a function
   lands wherever its sibling data lives. `reader.rs` therefore opens with a decoder
   (`doc_kind_name`) and then an encoder (`encode_doc_table`, `:21`) and closes with a
   ~200-line `impl AbiSerializer` (`:1368`) that is a hash *writer*.

3. **`mod.rs` interleaves public functions and type definitions.** The public API is
   split across `:98-124` and `:402-506`, with 265 lines of public type definitions
   between them and internal types from `:508`. A public type dropped into the second
   pub-fn block (`BinaryReprResourceExport`, `:466`) reads as internal at a glance —
   which is exactly how its stale `allow` survived.

## Goal

- The ISA-neutral object-plan model, the AArch64 relocation encoders, and the byte
  helpers exist **once** each.
- `binary_repr` encode and decode code are separated, under one naming convention.
- The four dead items (A/B/D below) are gone; `cargo check` emits no `never used`
  warning from `src/os/`.
- **Every emitted artifact is byte-identical before and after.**

### Non-goals (must NOT change)

- **Any emitted byte.** Not the `.mfp` wire format, not ELF/Mach-O layout, not
  section ordering, not the `.nobj` JSON dumps. `scripts/artifact-gate.sh` must show
  zero diffs.
- Section id **values** (`SECTION_DOC_TABLE = 17` etc.). An enum may be introduced;
  the numbers are wire format and are frozen.
- The macOS panic → `Err` change must NOT be folded in silently. See the HAZARD
  section: it is a behavior change and needs its own bug + test.
- **Tempting wrong fix, forbidden:** do not relax or delete a link test because a
  fixture refactor made it awkward. The two link `tests.rs` suites are the only thing
  standing between this refactor and a mislinked shipped binary.
- Spec anchors in `src/docs/spec/package/03_metadata-encoding.md` point into
  `reader.rs` by symbol. Splitting `reader.rs` requires re-pointing them in the same
  commit.

## Items

### (A) `os/` object-writer and linker duplication

**A1 — the two `object.rs` files are 821 common lines.**
`src/os/macos/object.rs:1-1410` vs `src/os/linux/object.rs:1-1051`. Byte-identical:
the ten plan structs (macOS `:31-101` / Linux `:29-99`) and the
`push_unique`/`reject_duplicates`/`align`/`ToObjectJson` block (macOS `:733-757` /
Linux `:532-556`). `join_json` (macOS `:937`, Linux `:736`) differs only in a
parameter name. Fix: extract `src/os/object_plan.rs` holding the plan structs, the
`ToObjectJson` trait, `join_json`, `json_string_list`, `push_unique`,
`reject_duplicates`, and one `align`.

**A2 — `LoadCommandPlan`, a Mach-O concept, is the ELF writer's struct name.**
`src/os/linux/object.rs:29` (also `src/os/macos/object.rs:31`); `SectionPlan.segment`
holds `"PT_LOAD"` at `src/os/linux/object.rs:116,125`. Rename to something neutral
(`ContainerDirectivePlan` / `LoadEntryPlan`) as part of A1. **This changes the `.nobj`
JSON only if the field name is serialized — check `ToObjectJson for LoadCommandPlan`
(`src/os/macos/object.rs:759`) before renaming, and keep the emitted key string
unchanged.**

**A3 — `patch_relocations`' six AArch64 arms are duplicated.**
`src/os/macos/link/mod.rs:250-364` (115 lines) vs `src/os/linux/link/mod.rs:165-250`
(86 lines). *Correction to the lead:* they are **not verbatim**. The bit-extraction
and instruction-encoding constants are identical, but the copies differ in four ways:
(a) macOS threads `rodata_vmaddr`/`rodata_size` through `symbol_vmaddr`, Linux does
not; (b) every macOS `read_u32`/`write_u32` call lacks the `?` its Linux twin has (see
HAZARD); (c) error prefixes `"macOS linker"` vs `"linux-aarch64 linker"`; (d) Linux
carries one extra GOT-slot comment. Fix: share the arm bodies behind a small context
struct (platform label + optional rodata window); do **not** attempt this before the
byte helpers agree on a return type.

**A4 — `branch_imm26` and `adrp_page21` are duplicated.**
`src/os/macos/link/mod.rs:587,604` vs `src/os/linux/link/mod.rs:564,581`. Verified: the
pair differs **only** in the error-message prefix, one comment's wording
(`"matching the riscv path"` vs `"mirror \`riscv_hi_lo\` and return an error"`), and a
doc-comment tail. Identical range checks, identical bug-168 rationale, identical
masks.

> *Correction to the lead:* these exist **twice, not three times**.
> `src/arch/aarch64/encode/sizing.rs:159` does define a `branch_imm26`, but it is a
> **different implementation** — a two-line delegation to a generic
> `branch_imm(source, target, bits, span)` (`sizing.rs:142`) with different error text
> and a word-count range check rather than a byte-delta one. And `sizing.rs` has
> **no `adrp_page21` at all**. Fix: hoist the linker pair to one shared module; a
> later, separate change may reconcile it with `sizing.rs`'s generic — that is a
> behavior-visible error-message merge and is out of scope here.

**A5 — byte-writer helpers in three modules; `align` in four copies with two
implementations.** `put_u16`/`put_u32`/`put_u64` at `src/os/macos/link/mod.rs:623-634`,
`src/os/linux/link/mod.rs:627-638`, `src/os/linux/squashfs.rs:657-668`.

`align` — **verified, and the lead's characterization is wrong**:

| Site | Implementation |
| --- | --- |
| `src/os/macos/object.rs:751` | `value.div_ceil(alignment) * alignment` |
| `src/os/linux/object.rs:550` | `value.div_ceil(alignment) * alignment` |
| `src/os/macos/link/mod.rs:583` | `value.div_ceil(alignment) * alignment` |
| `src/os/linux/link/mod.rs:604` | `(value + alignment - 1) & !(alignment - 1)` |

It is **not** "macOS div_ceil vs Linux masking" — three of the four use `div_ceil`, and
the masking outlier is `linux/link/mod.rs:604` alone. The distinction is real and
matters: the masking form is correct **only for power-of-two `alignment`** and returns
garbage otherwise, whereas `div_ceil` is general. All current call sites pass page/word
alignments, so nothing is wrong today — but the two forms are not interchangeable, and
**the unified helper must be the `div_ceil` one.** A fifth copy at
`src/target/shared/code/type_utils.rs:221` additionally guards `alignment == 0`; that
guard is the most defensive of the five and is the right shape for the shared helper.

**A6 — `squashfs` is `pub(crate)` with exactly one consumer.**
`src/os/linux/mod.rs:11` declares `pub(crate) mod squashfs;` while its siblings
`appdir` (`:3`), `appimage` (`:6`), `link` (`:8`), and `object` (`:9`) are all private.
The sole consumer is `src/os/linux/appimage/mod.rs:17`. Fix: move under
`appimage/squashfs.rs` and drop `pub(crate)`.

**A7 — `src/os/linux/squashfs/` is the only directory in the tree with a `tests.rs`
and no `mod.rs`.** Verified: `ls src/os/linux/squashfs/` → `tests.rs` only. Every
other `src/os` module keeps tests inline. Resolved for free by A6.

### (B) `binary_repr` structure

**B1 — `reader.rs` contains encoder code.** `src/binary_repr/reader.rs` (1,569 lines):
- `encode_doc_table` (`:21`) — the file's **second** function; sole caller
  `src/binary_repr/writer.rs:978`.
- `docs_from_ir` (`:114`) — IR→`PackageDocs` lowering; sole caller
  `src/binary_repr/writer.rs:246`.
- `encode_export_kind` (`:1298`).
- `function_sig_hash` (`:1325`), `type_sig_hash` (`:1352`), and `impl AbiSerializer`
  (`:1368-1565`) — ~200 lines of hash *writer*.
- `is_exported_function` (`:1567`) — **verified to have no caller inside `reader.rs`
  at all**; its only callers are `src/binary_repr/writer.rs:1016,1048` and
  `src/binary_repr/sections.rs:636`.

**B2 — `reader.rs` has zero internal grouping.** `grep -c '^mod \|^// ===\|^// ---'` →
**0**. 1,569 lines, four interleaved concerns, no banners.

**B3 — section ids are bare `u16` consts, declared out of numeric order.**
`src/binary_repr/mod.rs:26-48`: ids run 1–8, jump to 10–11, then **17, 15, 16** in that
order (`SECTION_DOC_TABLE = 17` at `:44` precedes `SECTION_ABI_INDEX = 15` at `:45` and
`SECTION_BINARY_REPR = 16` at `:48`). They drive **13 hand-written fetch blocks** at
`src/binary_repr/reader.rs:349-429` — 9 required (`.ok_or_else(...)` with 9 distinct
`"MFPC is missing the …"` strings) and 4 optional (`match sections.get(&…)` with
`Some`/`None`). *Correction to the lead:* there are 13 blocks but **9** error strings,
not 13. Fix: a `SectionKind` enum with `require()`/`optional()` accessors collapses 81
lines to roughly 25, and puts the ids in order at their declaration.

**B4 — five encoder conventions against one decoder convention.** Encoders:
(i) `impl <Table> { fn encode(&self) }` — `src/binary_repr/sections.rs:17,359,458,532,607,682`;
(ii) free fn `encode_native_library_table` — `sections.rs:718`;
(iii) `impl … { fn encode_<thing>(&self) }` — `writer.rs:988,1012,1034,1052`;
(iv) free fn `encode_doc_table` — `reader.rs:21`;
(v) scalar mapper `encode_export_kind` — `reader.rs:1298`.
Decoders are uniformly free `read_*` fns in `reader.rs`. Consequences: name mismatch
(`TypeTable::encode` ↔ `read_type_entries`), and `read_string_pool` (`reader.rs:575`)
returns `Vec<String>` while its siblings return their table type — so the caller
re-wraps by hand at `reader.rs:349-357`. Only `NATIVE_LIBRARY_TABLE` keeps both halves
adjacent (`sections.rs:718` encode / `sections.rs:763` decode) — **that is the shape
the other twelve should follow.**

**B5 — repeated decode boilerplate despite `util.rs` cursor helpers.**
`src/binary_repr/reader.rs:579-591` hand-rolls what `cursor_string` already does, while
`read_import_table` (`:983`) in the same file is pure `cursor_*` calls.

**B6 — `mod.rs` public API split by 265 lines of type definitions.**
`src/binary_repr/mod.rs`: public fns `:98-124`, public types `:127-398`, more public fns
`:402-506` (interleaved with types), internal types `:508-699`.
`BinaryReprResourceExport` — a **public** type — is stranded at `:466`, between
`read_package_type_exports` (`:455`) and `read_package_resources` (`:480`). See D3:
this is *how* its stale `allow` stayed invisible.

### (C) Test organization

**C1 — `binary_repr/tests.rs` is 3,698 lines but already `mod`-grouped, so the split
is mechanical.** 17 modules verified at lines 4, 89, 318, 427, 630, 992, 1232, 1627,
1735, 1848, 2469, 2554, 2723, 3026, 3092, 3443, 3495. A shared `mod fixtures`
(`:89-317`) is imported by **12** of them (`grep -c 'fixtures::' → 12`). Fix: promote
each `mod` to its own file under `binary_repr/tests/`, with `fixtures.rs` shared.
Largest resulting file ≈ 850 lines.

**C2 — ~500 lines of `EncodedImage` fixture boilerplate across the two link suites.**
**45 inline literals** verified: 18 in `src/os/linux/link/tests.rs`, 27 in
`src/os/macos/link/tests.rs`, each spelling all eleven fields (see
`src/os/linux/link/tests.rs:497-525` for a representative one — most carry an identical
dead four-field tail: `entry`, `initializers: Vec::new()`, `signing_metadata: None`,
`rpaths: Vec::new()`). **`src/os/macos/link/tests.rs:876-879` already demonstrates the
fix** — it is the *only* site in either file using `..none` struct-update syntax. Fix:
one `none()` base fixture per suite, then `..none()` everywhere.
`inst` (`src/os/macos/link/tests.rs:1168`) and `rv_inst`
(`src/os/linux/link/tests.rs:1090`) have identical bodies under different names.

> *Correction to the lead:* `func` (`src/os/linux/link/tests.rs:802`) and `code_fn`
> (`src/os/macos/link/tests.rs:1149`) are **not** byte-identical. They are structurally
> the same but differ materially: `returns` is `"Integer"` vs `"Nothing"`, and `func`
> is a *nested* fn inside a test body (indented) while `code_fn` is module-level.
> Unifying them requires a `returns` parameter, not a rename.

**C3 — both link suites are flat lists.** `src/os/linux/link/tests.rs:1570` is the
file's **only** banner; `src/os/macos/link/tests.rs` has **zero**. Contrast
`src/os/linux/squashfs/tests.rs`, which has 4. Also **9** `cfg(target_os = "macos")`
gates scattered through `src/os/macos/link/tests.rs` collapse into one grouped module.

**C4 — four helpers repeat setup a parameterized helper already covers.**
`src/os/linux/link/tests.rs:493-530,533-562,565-598,848-878` each rebuild the
image-and-assert shape that `expect_unbound` (`:730-775`) already parameterizes —
and `expect_unbound` is used by exactly one test (7 calls at `:779-789`). Also
`encode_dynamic_elf` is called 6 times, 5 with identical arguments.

**C5 — redundant `#[cfg(test)]` on fns already inside a gated module.**
`src/os/linux/link/tests.rs:1325,1384` and `src/os/macos/link/tests.rs:1334,1369`. Both
files are already reached through `#[cfg(test)] mod tests;`
(`src/os/linux/link/mod.rs:49-50`, `src/os/macos/link/mod.rs:22`), so these are no-ops;
the other ~15 helpers in the same files carry none.

**C6 — test names diverge across the two suites for identical concepts.** Cosmetic;
fold into C3.

### (D) Dead / misplaced

**D1 — `RUNTIME_X86_64_LEN` / `RUNTIME_AARCH64_LEN` are test-only but at module scope.**
`src/os/linux/appimage/mod.rs:40-41`, used only at `:263-264`, inside the
`#[cfg(test)] mod tests` that begins at `:256`. **`cargo check` warns on both today**
(output quoted in Current State). Fix: move them into the tests module beside the
assertion that consumes them.

**D2 — `SquashNode::dir` is dead outside tests.** `src/os/linux/squashfs.rs:136`; sole
caller `src/os/linux/squashfs/tests.rs:790`. Production constructs the variant directly
(`src/os/linux/appimage/mod.rs:253`: `Ok(SquashNode::Dir { entries, mode })`).
**`cargo check` warns.** Fix: delete, or move into the tests module.

**D3 — stale `#[allow(dead_code)]` on `BinaryReprResourceExport`; all five fields are
live.** `src/binary_repr/mod.rs:465-475`. Every field is read at
`src/syntaxcheck/mod.rs:1146-1173` (`type_name` :1146/:1172/:1173, `close_function`
:1149, `native` :1158, `sendable` :1165, `close_may_fail` :1166). **Proven stale:
removing the attribute and running `cargo check` produced zero new warnings.**
Two further problems in the same five lines:
- The comment at `:462-464` says `native` "**is read by** the later native-resource
  phase (`plan-link-update`)" — phrased as *pending*, but that plan is complete and
  archived, and the read exists today.
- The comment uses `//`, not `///`, so it sits **between** the doc comment (`:461`) and
  the struct — a doc-comment gap that also detaches `:461` from the item.

*Correction to the lead:* the finding cited a second read at `validation.rs:351`. The
only `validation.rs` in the tree is `src/target/shared/code/validation.rs`, which is
codegen plan validation and unrelated. `src/syntaxcheck/mod.rs` is the sole consumer.

**D4 — `build_binary_repr_bytes` is `pub` with no external caller.**
`src/binary_repr/mod.rs:111-116`. Callers: `src/binary_repr/mod.rs:104` (same module)
and `src/binary_repr/tests.rs:269,2858,2981`. Every sibling `pub fn` in the file has a
verified external caller. Fix: narrow to `pub(crate)` or `pub(super)`.

**D5 — degenerate single-arm match.** `src/binary_repr/reader.rs:1062-1064`:

```rust
let kind = match cursor_u16(bytes, &mut offset)? {
    kind => decode_callable_export_kind(kind)?,
};
```

One irrefutable binding arm. Fix: `let kind = decode_callable_export_kind(cursor_u16(bytes, &mut offset)?)?;`

## Blast Radius

- `src/os/macos/object.rs`, `src/os/linux/object.rs` — A1/A2; both rewritten against a
  new shared module.
- `src/os/macos/link/mod.rs`, `src/os/linux/link/mod.rs` — A3/A4/A5; **highest risk in
  this document**, these produce the shipped executables.
- `src/os/linux/squashfs.rs`, `src/os/linux/squashfs/tests.rs`,
  `src/os/linux/appimage/mod.rs`, `src/os/linux/mod.rs` — A6/A7/D1/D2.
- `src/binary_repr/{mod,reader,writer,sections,util}.rs` — B1–B6, D3–D5.
- `src/binary_repr/tests.rs`, `src/os/{linux,macos}/link/tests.rs` — C1–C6.
- `src/docs/spec/package/03_metadata-encoding.md` — **four symbol anchors into
  `reader.rs`** (`function_sig_hash`, `type_sig_hash`, `serialize_type`,
  `is_exported_function`). In scope: must be re-pointed by B1.
- `src/target/shared/code/type_utils.rs:221` — a fifth `align`. **Latent, same
  duplication, out of scope**: it is in the codegen layer with a different visibility
  (`pub(super)`) and a zero-alignment guard; unifying across `src/os` and
  `src/target` is a separate change.
- `src/arch/aarch64/encode/sizing.rs:142-165` — `branch_imm`/`branch_imm26`.
  **Out of scope**: a genuinely different implementation with different error text;
  merging it changes diagnostics.
- `src/os/icon`, `src/os/linux/appdir`, `src/os/mod.rs` — unaffected; no object-plan or
  relocation code.

## Fix Design

Extract-and-delete, in dependency order, with a byte-identity gate between every step.

1. **`src/os/object_plan.rs`** — the ten plan structs, `ToObjectJson`, `join_json`,
   `json_string_list`, `push_unique`, `reject_duplicates`, and one `align` (the
   `div_ceil` form with the `alignment == 0` guard from `type_utils.rs:221`). Both
   `object.rs` files import it. This is the largest line win (~400 lines) at the lowest
   risk, because `.nobj` goldens catch any drift immediately.
2. **`src/os/link_encode.rs`** — `branch_imm26`, `adrp_page21`, `put_u16/u32/u64`,
   `read_u32`, `write_u32`. **Resolve the panic-vs-`Err` question first** (see HAZARD)
   and land that as its own commit with its own test; only then unify.
3. **A3** — share `patch_relocations`' arms behind a context struct. Do this *after*
   step 2, so the arms already agree on `?`-propagation and the diff is purely the
   rodata window and the platform label.
4. **`binary_repr`** — split `reader.rs` into `decode.rs` + `encode.rs` (or move the
   encoder half into `writer.rs`/`sections.rs` beside its data, following the
   `NATIVE_LIBRARY_TABLE` shape), introduce `SectionKind`, reorder `mod.rs` into
   *public fns → public types → internal types*, and land D3–D5. Re-point the four
   spec anchors in the same commit.
5. **Tests** — C1 then C2; both are mechanical and independently landable.

**Rejected — do not re-litigate:**

- *Making the two `object.rs` files one file with `cfg` branches.* The differing 819
  lines are genuine format divergence (Mach-O load commands vs ELF program headers);
  `cfg` would interleave two formats in one body. Extract the neutral half, keep the
  format-specific halves separate.
- *Reconciling the linker `branch_imm26` with `sizing.rs`'s generic in this change.*
  It merges user-visible error strings. Separate change.
- *Renumbering section ids into order.* They are wire format. Only the *declaration*
  order in `mod.rs` moves; a `SectionKind` enum makes the ordering self-evident without
  touching a number.
- *Fixing the macOS panic quietly under A5.* Forbidden — see HAZARD.

Expected output shift: **none.** Every step must be byte-identical.

## Phases

### Phase 1 — establish the byte-identity baseline (no code change)

- [ ] Build `mfb` at base and record artifact hashes for every golden-bearing project
      (`scripts/artifact-gate.sh <exe>`); confirm zero diffs on a clean tree.
- [ ] Run `scripts/test-accept.sh` and record the pass count as the reference.
- [ ] Confirm the three `cargo check` `never used` warnings (D1/D2) reproduce, and that
      deleting `src/binary_repr/mod.rs:465`'s `allow` adds none (D3).
- [ ] File the macOS `read_u32`/`write_u32` panic as its own bug with a failing test.

Acceptance: baseline hashes recorded; the panic bug exists with a reproducing test.
Commit: —

### Phase 2 — `os/` extraction (A1–A7)

- [ ] Land the panic fix from Phase 1 (its own commit), then `src/os/object_plan.rs`
      (A1, A2, A5-`align`), then `src/os/link_encode.rs` (A4, A5-byte-helpers), then
      A3, then A6/A7, then D1/D2.
- [ ] Re-run `scripts/artifact-gate.sh` after **each** commit — not once at the end.

Acceptance: both link suites green; `artifact-gate.sh` zero diffs at every commit;
`cargo check` clean for `src/os/`.
Commit: —

### Phase 3 — `binary_repr` restructure (B1–B6, D3–D5) + tests (C1–C6)

- [ ] Split encoder/decoder; add `SectionKind`; reorder `mod.rs`; land D3–D5.
- [ ] Re-point the four `03_metadata-encoding.md` anchors; verify they resolve.
- [ ] Split `binary_repr/tests.rs` (C1); apply the `..none` fixture (C2); group and
      rename the link suites (C3–C6).
- [ ] Full `scripts/test-accept.sh`; `scripts/artifact-gate.sh` zero diffs; on Linux,
      `scripts/test-appimage.sh`; on macOS, `scripts/test-macapp.sh`.

Acceptance: full suite green at the Phase 1 reference count; `.mfp` bytes unchanged for
every golden project; every spec anchor resolves.
Commit: —

## Validation Plan

- **Regression tests:** none new for the cleanup itself — the existing
  `src/os/linux/link/tests.rs` (18 fixtures), `src/os/macos/link/tests.rs` (27), and
  `src/binary_repr/tests.rs` (17 modules) **are** the guard, and must survive the
  fixture refactor with identical assertions. One new test is required, for the panic
  hazard, filed separately.
- **Byte-identity proof:** `scripts/artifact-gate.sh <exe>` after every commit — it
  regenerates the deterministic `-ast/-ir/-br/-nir/-nplan/-nobj/-ncode` dumps and diffs
  them against committed goldens without linking or running, so it catches object-plan
  drift in minutes.
- **Runtime proof:** `scripts/test-accept.sh` end-to-end (the linkers produce the
  shipped binaries, so a passing unit suite is not sufficient); plus
  `scripts/test-appimage.sh` / `scripts/test-macapp.sh` for the app-mode paths that
  A6/D1/D2 touch.
- **Doc sync:** the four `03_metadata-encoding.md` symbol anchors (B1). The linker
  spec drift is a **separate** document — do not fix it here.
- **Full suite:** `scripts/test-accept.sh` at the Phase 1 reference pass count.

## Open Decisions

- **B1 shape** — split `reader.rs` into `decode.rs` + `encode.rs`, *or* move each
  encoder next to its data (the `NATIVE_LIBRARY_TABLE` shape at `sections.rs:718/763`).
  Recommended: the latter, since B4 already names it as the target convention and it
  disturbs fewer spec anchors. (§B1, §B4)
- **A3 scope** — share the six arms fully, or share only the four that differ solely in
  the platform label. Recommended: full share behind a context struct, once step 2 has
  made the `?`-propagation uniform. (§A3)
- **D2** — delete `SquashNode::dir` or relocate it into the tests module. Recommended:
  relocate; it is a legible constructor and the tests do use it. (§D2)

## Summary

Zero behavior change; all risk is in the mechanics. It concentrates in **A3/A5** — the
AArch64 linkers produce the shipped executables, and there is no golden for a linked
binary, only for the plan dumps that precede it, so `scripts/artifact-gate.sh` alone is
not sufficient proof and `scripts/test-accept.sh` must run end-to-end. **B1's file split
is the second risk**, because four spec anchors point into `reader.rs` by symbol and
will silently rot if not re-pointed in the same commit. The test items (C1–C6) are
mechanical and independently landable.

Untouched: every emitted byte, the `.mfp` wire format including section id **values**,
the linker spec drift (findings #13–#18, a separate document), the fifth `align` in
`src/target/shared/code/type_utils.rs:221`, and `src/arch/aarch64/encode/sizing.rs`'s
distinct `branch_imm26`.

The macOS `read_u32`/`write_u32` panic is **not** part of this cleanup. It is a real
robustness defect — the compiler aborts on macOS where it diagnoses on Linux, from the
same relocation stream — and must be filed, tested, and fixed on its own before the
byte helpers are unified, so that the fix is not buried in a refactor's diff.
