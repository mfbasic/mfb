# plan-126-B: The `mfb_wire` crate — byte primitives and MFP framing

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-A (letter order only; no code dependency)

Creates the third workspace crate and moves into it the byte-level decoding
primitives and `.mfp` container framing that the compiler and the registry each
implement today. After this sub-plan there is exactly one implementation of "read a
length-prefixed string from a byte cursor" and one definition of `MFP_MAGIC` in the
tree, and the registry can decode container framing without restating the
compiler's constants.

Behavioral outcome: **nothing observable changes.** Every `.mfp` this compiler
writes is byte-identical to before, every error string is unchanged, and both MFP
header decoders keep their own guard sets. This is pure code motion, and that is
what makes it safe to land alone.

References:

- `src/binary_repr/mod.rs:25-31` — the bug-340 B8 note explaining why the two MFP
  decoders are deliberately **not** merged. This sub-plan honors it; read it first.
- `repository/src/abi.rs:1-8` — the registry's own statement of the dependency
  problem this crate solves.
- `repository/Dockerfile:22-35` — the deploy image's workspace handling.
- `.ai/build-tooling.md` — rustfmt policy (and one stale line this sub-plan fixes).

## Prerequisites

See plan-126-A § Prerequisites — they govern the whole plan-126 feature.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- A new workspace member `mfb_wire` (directory `wire/`) with **no dependency on
  either `mfb` or `mfb_repository`**, and no I/O, HTTP, database or async
  dependencies — `sha2` only.
- `src/binary_repr/util.rs` lives in `mfb_wire`; the compiler's `binary_repr`
  re-exports it so its 200+ internal call sites are unchanged.
- `MFP_MAGIC`, the `.mfp` fixed-prefix field readers, and the signature-header rule
  have one home in `mfb_wire`; both `src/manifest/package.rs:read_mfp_header` and
  `repository/src/package.rs:parse_mfp_package` are built on it while keeping their
  own, different, guard sets.
- `repository/Dockerfile` still builds `mfb-repo` without compiling the compiler.

### Non-goals (explicit constraints)

- **The two MFP header decoders are NOT merged.** `src/binary_repr/mod.rs:25-31`
  records that decision (bug-340 B8): the manifest reader additionally enforces
  per-field byte limits, UTF-8, required-non-empty and `validate_package_name`, and
  returns fields the identity/payload decoder omits. Folding them would drop
  trust-boundary guards. Share the primitives; keep two named policies.
- **No behavior change of any kind.** Same error strings, same accept/reject
  decisions on both sides, byte-identical `.mfp` output.
- **`crypto`, `MfpPackage`, the `server` wire DTOs, `client` and `local` do not
  move.** They stay in `mfb_repository`; `mfb` keeps depending on it for them. This
  scope was decided explicitly — the goal is one home for what is *duplicated*, not
  a full re-layering.
- No change to the `.mfp` or MFPC wire format, section ids, or field ordering.

## 2. Current State

**Dependency direction.** Root `Cargo.toml` declares
`mfb_repository = { path = "repository" }` and `[workspace] members = [".",
"repository"]` with the same `default-members`. So `mfb_repository` cannot reference
`mfb`, which `repository/src/abi.rs:1-8` and `src/terminal_safe.rs:1-11` both state
in prose as the reason they restate or re-export things.

**The deploy image depends on the compiler *not* being built.**
`repository/Dockerfile:28` writes a stub:

```dockerfile
RUN mkdir -p src && printf 'fn main() {}\n' > src/main.rs
```

with the comment "`mfb` is a workspace member, so cargo must be able to resolve its
manifest — but it is never built here." A third member must be `COPY`d for real, not
stubbed, since `mfb_repository` will genuinely depend on it.

**`util.rs` is the compiler's byte-primitive layer**, 304 lines / 28 functions
(`grep -n "^pub(super) fn" src/binary_repr/util.rs` → 28). It has no compiler-specific
dependencies: `use super::*` pulls in `ABI_HASH_LEN` (`src/binary_repr/mod.rs:100`),
`Section` (`src/binary_repr/mod.rs:1133-1136`) and `Sha256`. Nothing from `crate::ir`,
`crate::ast`, `crate::types` or `crate::manifest`.

**Every `binary_repr` submodule imports it by glob.** `reader.rs:1`, `writer.rs:1`,
`sections.rs:1` and `builder.rs:1` all begin `use super::*;`, and `mod.rs:21` has
`use util::*;`. Re-pointing that one glob is what makes this move cheap.

**The `.mfp` fixed prefix is parsed twice**, field-for-field:

| | compiler | registry |
|---|---|---|
| entry point | `src/manifest/package.rs:71 read_mfp_header` | `repository/src/package.rs:99 parse_mfp_package` |
| magic | `crate::binary_repr::MFP_MAGIC` | literal array, `repository/src/package.rs:3` |
| `read_mfp_string` | `:169` | `:324` |
| `read_mfp_bytes` | `:184` | `:339` |
| `read_u16` / `read_u32` | `:211` / `:218` | `:421` / `:428` |
| signature-header rule | `validate_mfp_signature_header` (`src/binary_repr/mod.rs:32`) | `validate_signature_header` (`repository/src/package.rs:315`) |
| `ident` field | read `required=false` | read `required=true` |
| name charset guard | calls `validate_package_name` | none |

The last two rows are the *policy* difference bug-340 B8 protects. They stay.

### Measured populations

| What | Count | Command |
|---|---|---|
| `util.rs` lines / functions | 304 / 28 | `wc -l src/binary_repr/util.rs`; `grep -c "^pub(super) fn" src/binary_repr/util.rs` |
| `cursor_u32` call sites in `binary_repr` | 128 | `grep -rho cursor_u32 src/binary_repr --include='*.rs' \| wc -l` |
| `bounded_capacity` | 29 | same form |
| `cursor_u16` | 22 | same form |
| `cursor_string` | 17 | same form |
| `cursor_optional_str` / `cursor_prose_list` / `cursor_pair_list` | 7 / 6 / 5 | same form |
| `util_tests.rs` tests / lines | 17 / 274 | `grep -c '#\[test\]' src/binary_repr/tests/util_tests.rs`; `wc -l` |
| `MFP_MAGIC` references — compiler / registry | 15 / 3 | `grep -rho MFP_MAGIC src --include='*.rs' \| wc -l`; same over `repository/src` |
| `read_mfp_header` references in `src/` | 42 | `grep -rho read_mfp_header src --include='*.rs' \| wc -l` |
| `binary_repr` total lines | 5,949 | `wc -l src/binary_repr/*.rs` |
| Committed `.mfp` fixtures under `tests/` | 159 | `git ls-files '*.mfp' \| wc -l` |

### Verified properties

- **`util.rs` is compiler-independent.** Read all 28 functions: the only non-`std`
  reference is `Sha256::digest` in `hash_bytes` (`src/binary_repr/util.rs:85-91`)
  and the `ABI_HASH_LEN`/`Section` types, both plain data. `repository/Cargo.toml`
  already declares `sha2 = "0.10"`, so no new dependency enters the deploy image.
- **The glob-import structure holds.** Verified by reading line 1 of `reader.rs`,
  `writer.rs`, `sections.rs`, `builder.rs` — all `use super::*;`. So a
  `pub use mfb_wire::bytes::*;` in `src/binary_repr/mod.rs` reaches all four
  submodules without touching a single call site.
- **`packages/*.mfp` are build artifacts, not committed** (`git ls-files
  'packages/*.mfp'` → no output), but **159 `.mfp` fixtures under `tests/` are
  committed** and are consumed by `rt-behavior` tests. Those are the real
  regression detector for a framing change: a decoder that shifts by one byte fails
  them.
- **`.ai/build-tooling.md:21` is stale.** It states "There is no `[workspace]`
  table in the root `Cargo.toml`", but the table exists (added by bug-347) and
  `cargo metadata --no-deps` reports 2 workspace members. This sub-plan adds a third
  member, so the line must be corrected rather than left to mislead.

## 3. Design Overview

```
        mfb_wire  (new: byte primitives + MFP/MFPC framing; sha2 only)
           ↑                              ↑
           |                              |
    mfb_repository  ←──────────────────  mfb
    (server + client)                 (compiler)
```

`mfb` keeps depending on `mfb_repository` for `client`/`local`/`crypto`/`package` —
that arrow is correct, since the compiler genuinely *is* a registry client. The new
crate only removes the arrow that could not exist.

Module layout inside `mfb_wire`:

- `wire/src/bytes.rs` — the former `src/binary_repr/util.rs`, `pub`.
- `wire/src/mfp.rs` — `MFP_MAGIC`, `FIXED_PREFIX_LEN`, `read_mfp_string`,
  `read_mfp_bytes`, `read_u16`/`read_u32`/`read_u64`, and the signature-header rule.
- `wire/src/lib.rs` — module declarations and the crate doc explaining the
  dependency rule that justifies its existence.

**Where correctness risk concentrates:** the MFP framing move (Phase 3). Both
decoders read the same fixed prefix by advancing a shared offset; an off-by-one in a
shared field reader breaks package identity verification on the registry *and*
`mfb pkg` on the client. That is why it lands last, behind the 159 committed `.mfp`
fixtures.

**Where design uncertainty concentrates:** whether the Docker build still works with
a third member. Cheap to falsify — build the image — so it is Phase 1's acceptance,
before any code moves.

**Byte-identity IS this sub-plan's gate.** This is provably-neutral code motion whose
whole point is to preserve output exactly, which is the one class where byte-identity
is the right, strongest check. `scripts/artifact-gate.sh target/release/mfb all` must
report `diffs=0`, and no `.ncodesum` golden may be regenerated. A diff means a bug was
introduced in the move — objdump one fixture, localize it, fix it. **It never means
the design is dead.**

**Rejected alternative — move only the seven doc-table cursor helpers.** Measured,
`cursor_u32` alone has 128 call sites inside `binary_repr` and the helpers call each
other (`cursor_prose_list` → `cursor_u32` + `bounded_capacity` + `cursor_string`,
`src/binary_repr/util.rs:30-44`). Splitting the file leaves two homes for one layer —
the exact problem being fixed.

**Rejected alternative — invert the dependency so `mfb_repository` depends on `mfb`.**
The compiler uses five repository modules across 10 files (client 52, package 35,
crypto 25, server 18, local 11 references), all of which would have to move into the
compiler; and `mfb-repo`'s deploy image would then compile all of codegen, `image`
and `icns`, defeating `repository/Dockerfile:28`.

## Compatibility / Format Impact

Nothing externally observable changes. The `.mfp` and MFPC wire formats, section
ids, field order, error strings and public APIs of both existing crates are
unchanged. The only new public surface is the `mfb_wire` crate itself, which no
external consumer sees.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Empty crate, wired into the workspace and the deploy image

Proves the riskiest unknown (the Docker build) before any code moves.

- [ ] Create `wire/Cargo.toml` (`name = "mfb_wire"`, edition 2021, `sha2 = "0.10"`,
      and the same `[lints.clippy] items_after_test_module = "deny"` both existing
      crates carry) and `wire/src/lib.rs` with a crate doc stating the dependency
      rule: `mfb_wire` depends on neither sibling; both siblings depend on it.
- [ ] Add `"wire"` to `[workspace] members` **and** `default-members` in the root
      `Cargo.toml`, so a bare `cargo test` still means the whole workspace
      (the reason bug-347 named both).
- [ ] Add `mfb_wire = { path = "../wire" }` to `repository/Cargo.toml` and
      `mfb_wire = { path = "wire" }` to the root `Cargo.toml`.
- [ ] Add `COPY wire/Cargo.toml ./wire/` beside the existing `COPY
      repository/Cargo.toml` and `COPY wire/src ./wire/src` beside `COPY
      repository/src` in `repository/Dockerfile` (lines 23-29). Leave the `mfb` stub
      at `:28` in place and extend its comment to say the third member is built for
      real.
- [ ] Correct `.ai/build-tooling.md:21`: the root `[workspace]` table exists
      (bug-347); state which crates `cargo fmt --all` now reaches and that
      `repository/` keeps its own pass per AGENTS.md.

Acceptance: `docker build -f repository/Dockerfile .` from the repository root
succeeds and the resulting image contains `mfb-repo`; `rustup run 1.96.0 cargo
metadata --no-deps --format-version 1` reports 3 workspace members.
Commit: —

### Phase 2 — Move the byte primitives

- [ ] Move `src/binary_repr/util.rs` to `wire/src/bytes.rs` verbatim, changing only
      `pub(super)` → `pub` and replacing `use super::*;` with explicit imports.
- [ ] Move `ABI_HASH_LEN` (`src/binary_repr/mod.rs:100`) and `Section`
      (`src/binary_repr/mod.rs:1133-1136`) into `wire/src/bytes.rs`, since
      `hash_bytes`, `cursor_hash`, `hex_hash` and `encode_sections` need them.
- [ ] In `src/binary_repr/mod.rs`, delete `mod util;` / `use util::*;` and add
      `pub(crate) use mfb_wire::bytes::*;`. Verify the four submodules
      (`reader.rs:1`, `writer.rs:1`, `sections.rs:1`, `builder.rs:1`) still resolve
      via their existing `use super::*;` — **no call site should need editing.**
- [ ] Move the 17 tests from `src/binary_repr/tests/util_tests.rs` (274 lines) into
      `wire/src/bytes.rs`'s own test module, so the primitives are tested where they
      live. Delete the now-empty file and its `mod util_tests;` line in
      `src/binary_repr/tests/mod.rs`.
- [ ] Check the deletion did not orphan a doc comment onto a neighbouring item in
      `src/binary_repr/mod.rs` and `tests/mod.rs`.

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes with the 17 tests
now reported under `mfb_wire`; `git diff --stat` shows **zero** changes under
`src/binary_repr/{reader,writer,sections,builder}.rs` — if any of those four files
needed an edit, the glob re-export is wrong and should be fixed rather than
worked around.
Commit: —

### Phase 3 — Move the MFP framing (largest blast radius)

- [ ] Create `wire/src/mfp.rs` holding `MFP_MAGIC`, `FIXED_PREFIX_LEN`, and the
      fixed-prefix field readers, taken from `src/manifest/package.rs:169-225` (the
      copy that already enforces per-field byte limits and UTF-8, since it is the
      stricter of the two and its extra guards are parameters, not policy).
- [ ] Move the signature-header rule there: `validate_mfp_signature_header`
      (`src/binary_repr/mod.rs:32`, defined in `reader.rs`) and
      `repository/src/package.rs:315 validate_signature_header` must become one
      function. Read both first and confirm they agree; if they do not, that
      divergence is a finding — record it in Corrections and keep the stricter.
- [ ] Re-point `src/manifest/package.rs:read_mfp_header` at the shared readers,
      **keeping** its `ident` `required=false` argument and its
      `validate_package_name(&name)` call.
- [ ] Re-point `repository/src/package.rs:parse_mfp_package` at the shared readers,
      **keeping** its `ident` `required=true` argument and its absence of a name
      charset guard. Delete the literal `MFP_MAGIC` at `repository/src/package.rs:3`
      and its private `read_mfp_string`/`read_mfp_bytes`/`read_u16`/`read_u32`
      (`:324`, `:339`, `:421`, `:428`).
- [ ] Update the bug-340 B8 note at `src/binary_repr/mod.rs:25-31` in place: it must
      now say the two decoders share their *primitives* and remain separate
      *policies*, and name where the shared layer lives. Do not delete it — the
      decision it records is still in force.
- [ ] Tests: a test in `wire/src/mfp.rs` for each field reader's truncation and
      UTF-8 rejection; and a cross-crate test asserting the two decoders still
      disagree as designed — the same bytes with an empty `ident` accepted by
      `read_mfp_header` and rejected by `parse_mfp_package`.

Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` passes, **including** every
`rt-behavior` test that consumes one of the 159 committed `.mfp` fixtures; and
`scripts/artifact-gate.sh target/release/mfb all` reports `diffs=0` with no golden
regenerated. The cross-crate divergence test is what proves bug-340 B8 was honored
rather than silently undone.
Commit: —

## Validation Plan

- **Tests:** `wire/src/bytes.rs` (17 moved tests), `wire/src/mfp.rs` (new field-reader
  and signature-header tests, including truncation and invalid-UTF-8 negatives), and
  the cross-crate divergence test in Phase 3.
- **Coverage check:** `mfb_wire` must be in `default-members` or its tests are
  outside a bare `cargo test` — exactly the hole bug-347 fixed for `repository/`.
  Verify with `cargo test --no-fail-fast 2>&1 | grep 'Running.*mfb_wire'` before
  trusting a green run.
- **Runtime proof:** `mfb pkg info packages/jwt/jwt.mfp` and `mfb pkg doc
  packages/jwt/jwt.mfp` both succeed against a package built by the *pre-change*
  compiler, proving the moved framing still decodes existing bytes.
- **Byte-identity:** `scripts/artifact-gate.sh target/release/mfb all` → `diffs=0`.
  Per `.ai/testing-gates.md`, run it against the **release** binary. Acquire
  `scripts/gate-lock.sh` first — `artifact-gate.sh` and `test-accept.sh` are
  mutually exclusive in one worktree and refuse with exit 98.
- **`.mfp` byte-identity:** rebuild `packages/jwt/jwt.mfp` and confirm it is
  byte-identical to the pre-change build (`cmp`). The `mfp` dump kind is
  deliberately *not* in the artifact gate (`.ai/testing-gates.md`), so this check
  must be run by hand.
- **Doc sync:** `.ai/build-tooling.md:21` (stale workspace claim, Phase 1);
  `src/binary_repr/mod.rs:25-31` (bug-340 B8 note, Phase 3). No `mfb man` or
  `src/docs/spec/**` change — this is internal crate layout.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`; `docker build -f
  repository/Dockerfile .`.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run
  1.96.0 cargo fmt)` — then confirm the new crate was reached, since AGENTS.md's
  two-pass command predates the workspace table.

## Open Decisions

- **Crate name and directory.** Recommended `mfb_wire` in `wire/`, matching the
  `repository/` → `mfb_repository` convention. Alternative `mfb_format`; the
  contents are wire formats specifically, so `wire` is the more honest name. (§3)
- **Does `Section` belong in `bytes.rs` or its own `mfpc.rs`?** Recommended
  `bytes.rs` for now, since only `encode_sections` uses it; plan-126-C adds the
  section *table reader* and may be the better home. Revisit there rather than
  guessing now. (§Phase 2)

## Corrections

<!-- Fill in during execution. In particular: if the two signature-header rules turn
     out to disagree (Phase 3, task 2), record the difference and which one won. -->

## Summary

The engineering risk is concentrated in Phase 3, where two independently-written
decoders start sharing an offset-advancing reader; the 159 committed `.mfp` fixtures
and the byte-identity gate are what make that safe. Phases 1 and 2 are cheap and
mechanical — the glob-import structure means 200+ call sites move for free. What is
deliberately left untouched: `crypto`, `MfpPackage`, the `server` DTOs, `client` and
`local`, and both decoders' guard policies.
