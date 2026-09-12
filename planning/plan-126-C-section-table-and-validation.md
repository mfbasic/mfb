# plan-126-C: One MFPC section table, one validation module, one sanitizer

Last updated: 2026-09-06
Effort: medium (1h–2h)
Depends on: plan-126-B

Ports `repository/src/abi.rs` onto the shared MFPC framing and moves the last three
duplicated things — the section-id/wire-enum constants, the package-name validators,
and the terminal sanitizer — into `mfb_wire`. This is the phase where the *tamper
checks on a signed payload* stop existing in two independently-written copies.

Behavioral outcome: the registry's MFPC section-table reader gains the two guards
the compiler's has and it lacks — an MFPC major-version check and a checked
`u64 → usize` conversion — and every constant the registry currently restates from
the compiler is imported instead.

References:

- `repository/src/abi.rs:1-8` — "this crate does not depend on the compiler crate",
  the comment that motivated every restatement this sub-plan removes.
- `src/binary_repr/reader.rs:383-427` — `read_binary_repr_package`, which contains
  the compiler's section-table decode inline.
- `src/terminal_safe.rs:1-11` — the bug-489 re-export shim and its own statement of
  the problem.
- AGENTS.md § "Never edit a test/golden to pass" — Phase 2 changes what the registry
  accepts; the four-question gate applies to any test that pushes back.

## Prerequisites

See plan-126-A § Prerequisites, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-126-B complete (`mfb_wire` exists with `bytes` + `mfp`) | `ls wire/src/bytes.rs wire/src/mfp.rs` → both exist | MET (measured 2026-09-12: both exist; B landed as baf71a193 + c2ef2dc23) |

If plan-126-B is not complete, this sub-plan cannot start, full stop.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop, and report
> the status of *all* prerequisites if you stop.

## 1. Goal

- One `read_section_table` in `mfb_wire`, used by both `src/binary_repr/reader.rs`
  and `repository/src/abi.rs`, enforcing the union of today's guards.
- MFPC section ids, the `libc`/`libtype` wire encodings, `NATIVE_LIBRARY_HASH_LEN`,
  `ABI_HASH_LEN`, `ABI_FORMAT_VERSION` and `MAX_DESCRIPTION_BYTES` declared once.
- One `validation` module in `mfb_wire` holding both package-name policies under
  names that say what each guards, plus `fold_owner`, `validate_owner_name`,
  `validate_version`, `validate_ident` and the three limits.
- `terminal_safe` implemented in `mfb_wire`; both crates import it and the bug-489
  shim direction is reversed.

### Non-goals (explicit constraints)

- **No change to the MFPC wire format**, section ids, or field layout.
- **The two package-name policies keep their current accept sets** unless the Open
  Decision below is resolved to unify them — and if it is, that is a deliberate,
  tested behavior change, never a side effect of the move.
- **`crypto`, `MfpPackage`, the `server` DTOs, `client` and `local` still do not
  move** (plan-126-B § Non-goals).
- The registry's best-effort posture toward optional sections is preserved: a
  package with no section 10 or 18 is still normal, not an error.

## 2. Current State

### The section table is decoded twice, with different guards

| Guard | compiler (`src/binary_repr/reader.rs:383-427`) | registry (`repository/src/abi.rs:317-343`) |
|---|---|---|
| MFPC magic | yes (`:384`) | yes (`:318`) |
| **MFPC major version == 2** | **yes** (`:389-395`, `MFPC_MAJOR_VERSION`, `src/binary_repr/mod.rs:82`) | **no** |
| section-table length overflow | yes, `checked_add`/`checked_mul` | yes, same shape |
| truncated section table | yes | yes |
| truncated section | yes | yes |
| duplicate section id (PKG-06) | yes (`:420-427`) | yes (`:339-341`) |
| **`u64 → usize` conversion** | **`checked_usize`** (`:412-413`) | **`as usize`** (`:334-335`) |

The registry's comment at `:339` — "matches the compiler reader" — is the tell: two
copies kept in sync by hand, and they have already drifted on two rows.

The compiler's decode is **not** a separate function; it is inline inside
`read_binary_repr_package` (`src/binary_repr/reader.rs:383`). Extracting it is part
of this work.

### Constants the registry restates

`repository/src/abi.rs:11-23` declares `MFPC_MAGIC`, `SECTION_MANIFEST`,
`SECTION_STRING_POOL`, `SECTION_NATIVE_LIBRARY_TABLE`, `SECTION_ABI_INDEX`,
`SECTION_PACKAGE_META`, `PACKAGE_META_FIELD_DESCRIPTION`, `MAX_DESCRIPTION_BYTES`,
`ABI_FORMAT_VERSION`, `ABI_HASH_LEN`; `:28-37` declares `WIRE_LIB_TYPE_VENDOR`,
`NATIVE_LIBRARY_HASH_LEN` and `WIRE_LIBC_UNSPECIFIED`/`GLIBC`/`MUSL`, with a comment
citing `src/binary_repr/mod.rs:353-359` as the source of truth. Compiler-side homes
are `src/binary_repr/mod.rs:39-74` (ids) and `:82`, `:100`.

### The two package-name validators are closer than they look

Read both in full:

- `src/manifest/package.rs:58-70` — first char `[A-Za-z0-9_]`, remaining
  `[A-Za-z0-9_.-]`. No length cap, no distinct empty-input message (an empty name
  fails because `chars.next()` is `None`).
- `repository/src/validation.rs:47-72` — explicit empty check, `len > 128` cap
  (`PACKAGE_LIMIT`), first char `[A-Za-z0-9_]`, all chars `[A-Za-z0-9_.-]`.

**The charsets are identical.** The only substantive difference is the registry's
128-byte cap, plus different error strings. This is a smaller divergence than the
existence of two functions implies.

### `terminal_safe` is already shared, in the wrong direction

`src/terminal_safe.rs` is a 12-line `pub(crate) use mfb_repository::terminal_safe::{is_terminal_unsafe, safe};`
whose doc comment says the implementation lives in the registry crate "because
`mfb_repository` cannot depend on `mfb`" (bug-489). The 100-line implementation is
in `repository/src/terminal_safe.rs`. A terminal sanitizer is not registry code.

### Measured populations

| What | Count | Command |
|---|---|---|
| `repository/src/abi.rs` lines / tests | 1,063 / 21 | `wc -l repository/src/abi.rs`; `grep -c '#\[test\]' repository/src/abi.rs` |
| `repository/src/validation.rs` lines / tests | 172 / 6 | same form |
| `repository/src/terminal_safe.rs` lines / tests | 100 / 4 | same form |
| `terminal_safe::{safe,is_terminal_unsafe}` call sites — compiler / registry | 37 / 2 | `grep -rho 'terminal_safe::\(safe\|is_terminal_unsafe\)' src --include='*.rs' \| wc -l`; same over `repository/src` |
| `validation::*` call sites in the registry | 12 | `grep -rho 'validation::[a-z_]*' repository/src --include='*.rs' \| wc -l` |
| `validate_package_name` references in `src/` | 16 | `grep -rho validate_package_name src --include='*.rs' \| wc -l` |
| Constants restated in `abi.rs` | 15 | `sed -n '11,37p' repository/src/abi.rs \| grep -c '^const'` |

### Verified properties

- **The registry really is missing the major-version check.**
  `grep -n "MFPC_MAJOR\|major" repository/src/abi.rs` returns one hit, at `:486`,
  inside a *test* fixture builder (`put_u16(&mut bytes, 2); // major`) — never a
  validation. The compiler's check is at `src/binary_repr/reader.rs:389-395`.
- **A stricter reader will not reject a publish; it will silently empty a field.**
  Read `abi_index_json` (`repository/src/abi.rs:306-315`): every parse error maps to
  `serde_json::json!({})`. `parse_package_description` and `parse_vendor_blobs`
  likewise return `Ok(None)`/`Ok(vec![])` for a non-MFPC payload
  (`:214-217`, `:108-118`). So a newly-strict `read_section_table` changes *recorded
  metadata*, not accept/reject — which is more insidious, and is why Phase 2 needs
  an explicit test rather than "the publish tests still pass".
- **The charsets of the two name validators are identical**, established by reading
  both functions in full (see above), not by their names or call sites.

## 3. Design Overview

Three independent moves, ordered by blast radius:

1. **`wire/src/validation.rs`** — both name policies plus the owner/version/ident
   validators and limits. Pure functions, no callers change semantics.
2. **`wire/src/terminal_safe.rs`** — the implementation moves out of the registry;
   `repository/src/terminal_safe.rs` becomes the re-export shim, and
   `src/terminal_safe.rs`'s shim re-points. 39 call sites across both crates are
   untouched because both shims keep the same paths.
3. **`wire/src/mfpc.rs`** — `MFPC_MAGIC`, `MFPC_MAJOR_VERSION`, the section ids, the
   wire enums, the limits, and `read_section_table`. Both crates re-point.

**Where correctness risk concentrates:** move 3. It changes tamper checks on bytes
covered by a package signature, and the failure mode is silent (an emptied ABI index
rather than a rejected publish). It lands last, with a test that asserts the *new*
guard actually fires.

**Where design uncertainty concentrates:** whether unifying the two name validators
is safe (Open Decision). Do not resolve it by guessing; measure whether any package
name over 128 bytes exists in the tree first — that is a one-line command and it is
Phase 1's first task.

**Byte-identity is this sub-plan's gate for moves 1 and 2** (pure code motion; no
codegen effect expected, and `scripts/artifact-gate.sh` should report `diffs=0`).
For move 3 it is the wrong gate — the registry's behavior legitimately changes — so
the check there is the new negative tests plus the repository crate's 351 lib tests.
A `diffs=0` from the artifact gate proves nothing about move 3 and must not be cited
for it.

**Rejected alternative — leave `abi.rs` alone and restate section id 17 for the doc
table.** That is the fourth restatement of the same knowledge in a file that already
carries a comment apologizing for the first three, and it leaves the drifted
tamper-check rows drifted.

**Rejected alternative — make the compiler adopt the registry's laxer reader.** The
divergences both favor the compiler (a major-version check and a checked cast are
strictly better); the union is the compiler's set.

## Compatibility / Format Impact

- **Registry behavior changes** for a malformed or wrong-version MFPC payload: what
  previously produced an empty `abiIndex`/`description` via a partially-successful
  parse now produces an empty one via an explicit early error. Externally the JSON
  shape is identical; the difference is which packages land with empty metadata.
  A payload the *compiler* would refuse to load is the only input affected.
- No change to the `.mfp`/MFPC format, section ids, HTTP routes, or DB schema.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same commit
> as the work it describes; `- [~]` plus one line for partial; `- [x] ~~text~~ —
> moot: <evidence>` for moot. Fill `Commit:` the moment a phase lands. **An unticked
> box means NOT DONE.**

### Phase 1 — Validation module

- [x] Measure first: the longest package name across all **160** committed
      `.mfp` fixtures is **34 bytes** (`native_resource_state_export_valid`), and
      **zero** exceed 128. Measured by walking each fixture's header rather than
      by `mfb pkg info` per file — the plan's loop would have been 160 process
      spawns; a 30-line Python header walk over `git ls-files '*.mfp'` reads the
      `name` field directly and reported 160/160 parsed, 0 unreadable. This
      decides the Open Decision below.
- [x] Create `wire/src/validation.rs` holding, from `repository/src/validation.rs`:
      `OWNER_LIMIT`, `PACKAGE_LIMIT`, `VERSION_LIMIT`, `fold_owner`,
      `validate_owner_name`, `validate_version`, `validate_ident`; and both name
      policies renamed to say what they guard —
      `validate_path_component_name` (guards `packages/<name>.mfp`) and
      `validate_registry_package_name` (guards the log payload and
      `/index/<ident>`). Every error string kept verbatim.
- [x] Re-point `repository/src/validation.rs` to a re-export shim, and
      `src/manifest/package.rs` likewise, so the registry and compiler call
      sites are unchanged. Verified: `git diff --stat HEAD` against
      `repository/src/{server,store,client,local}.rs` prints **nothing**.
      The registry's shim binds `validate_package_name` to the **registry**
      policy — the name is ambiguous now that two exist, so a test pins which
      one it resolves to (see below).
- [x] Move the 6 tests from `repository/src/validation.rs` into
      `wire/src/validation.rs` and add
      `the_two_package_name_policies_differ_only_by_a_length_cap`: a
      200-character all-legal-charset name passes the path-component policy and
      fails the registry's. It goes further than the plan asked and asserts the
      **charsets agree** across 7 accepted and 11 rejected inputs, so "the cap
      is the only substantive divergence" is measured rather than claimed.
- [x] Added task: `the_reexported_package_name_validator_is_the_registry_policy`
      in `repository/src/validation.rs`. With two same-charset policies behind
      one re-exported name, binding the shim to the wrong one would compile,
      pass every existing test, and silently drop the `PACKAGE_LIMIT` cap from
      the log payload. An over-cap name is the single input that tells them
      apart, so it is now asserted.
- [x] Added task: `state_is_active` (plan-126-A) stays in the registry crate and
      did not move with the validators. Its doc comment now records why, and the
      duplication it shares with the client's `state_is_floating_eligible` is
      noted in Corrections as a candidate for a later pass.

Acceptance: MET. `cargo test -p mfb_wire` → **30 passed**;
`cargo test -p mfb_repository --lib` → **376 passed; 0 failed** (381 − 6 moved
out + 1 new shim test). The divergence test is present and green, so the next
reader learns the two policies differ only by a length cap rather than having to
diff them.
Commit: 8af4a40eb

### Phase 2 — Terminal sanitizer

- [x] Move `repository/src/terminal_safe.rs`'s 100-line implementation and its 4
      tests to `wire/src/terminal_safe.rs` (via `git mv`).
- [x] Replace `repository/src/terminal_safe.rs` with a re-export shim, and update
      `src/terminal_safe.rs`'s shim to point at `mfb_wire`. Both doc comments
      rewritten: each now says the sanitizer lives in `mfb_wire` because it is
      shared, records that bug-489's rationale described a constraint that no
      longer binds, and says explicitly **not to restore it** — the old wording
      left a *terminal sanitizer* owned by the package-registry crate.
- [x] Confirm neither shim removal orphaned a doc comment onto a neighbouring
      item. Both shim files were rewritten whole, so there is no neighbouring
      item for a comment to land on.
- [x] Added task: fix the spec provenance citation this move invalidated.
      `src/docs/spec/tooling/04_audit-format.md` cited
      `[[src/terminal_safe.rs:is_terminal_unsafe]]`, which is now a shim rather
      than the implementation. Re-pointed at
      `[[wire/src/terminal_safe.rs:is_terminal_unsafe]]`. The file-level
      citation gate could not have caught this — the cited file still exists and
      even still contains the symbol's *name*, in the `pub(crate) use` line.
      Second instance of this class in plan-126; see Corrections.

Acceptance: MET. `cargo test -p mfb_wire` → **34 passed**, with all four
`terminal_safe::tests::*` reported under `mfb_wire`.
`cargo test -p mfb_repository --lib` → **372 passed; 0 failed** (376 − 4 moved).
Call sites re-counted after the move: **37 compiler + 2 registry = 39**, and
`git status` lists only the two shim files, the moved implementation, the
citation, and the plan — **no call site changed**.
`cargo test --bin mfb citations_resolve` → ok.
Commit: 3fd644f54

### Phase 3 — One MFPC section table (largest blast radius)

- [x] Create `wire/src/mfpc.rs` with `MFPC_MAGIC`, `MFPC_MAJOR_VERSION`, every
      `SECTION_*` id, `PACKAGE_META_FIELD_DESCRIPTION`, `ABI_FORMAT_VERSION`,
      `MAX_DESCRIPTION_BYTES`, `NATIVE_LIBRARY_HASH_LEN`, and the
      `WIRE_LIBC_*` / `WIRE_LIB_TYPE_*` encodings, each with its doc comment
      (the bug-277 `ABI_FORMAT_VERSION` and plan-61-D `SECTION_PACKAGE_META`
      rationales travelled intact). **Plus `MAX_MFPC_SECTIONS`**, which the plan
      did not list because the plan predated bug-578 — see Corrections.
      `ABI_HASH_LEN` stays in `bytes.rs` (plan-126-B put it there with the
      primitives typed on it). The "restated here" apologies in
      `repository/src/abi.rs` are deleted, not moved.
- [x] Extract the section-table decode from `read_binary_repr_package` into
      `mfb_wire::mfpc::read_section_table`, keeping **all** compiler guards: magic,
      `MFPC_MAJOR_VERSION`, `checked_add`/`checked_mul` on the table length,
      truncation, `checked_usize` on offset and length, and the PKG-06
      duplicate-id rejection with its explanatory comment — **and adding** the
      registry's `MAX_MFPC_SECTIONS` declared-count ceiling, which the compiler's
      copy lacked. The union is not the compiler's set (Corrections).
- [x] Re-point `read_binary_repr_package` at it, adopting `BTreeMap` as
      recommended: `SectionKind::require`/`optional` in `reader.rs` switched from
      `HashMap` to `BTreeMap`; no compiler caller used `HashMap`-specific API.
- [x] Delete the registry's `read_section_table` and re-point its callers.
      **Four, not five** — `grep -n read_section_table repository/src/abi.rs`
      showed the definition, four production callers and three test-only uses;
      the hedged "fifth" does not exist (Corrections). Its now-dead `read_u64`
      helper and that helper's test row were removed with it.
- [x] Preserve the best-effort posture at each registry call site: a
      non-container payload still yields "no such section", not an error. Pinned
      by `a_non_container_payload_still_means_no_such_section_not_an_error`, which
      feeds an empty payload, garbage, and a structurally-valid **v3** container
      to `parse_vendor_blobs`, `parse_package_description` and
      `parse_manifest_metadata` and asserts `Ok(empty)` from all three.
- [x] Tests in `wire/src/mfpc.rs`: duplicate section id rejected; truncated table
      rejected; truncated section rejected; **MFPC major version 3 (and 1)
      rejected**; declared count over `MAX_MFPC_SECTIONS` rejected, with exactly
      256 accepted. The planned "`u64` offset that does not fit `usize`" test
      **cannot be written against a 64-bit host** — replaced by
      `a_colossal_u64_offset_or_length_is_refused_not_used` plus a direct
      `checked_usize` pin gated on `target_pointer_width = "32"` (Corrections).
      Also `the_section_ids_are_frozen_wire_values` (pinned by literal) and a
      well-formed decode in id order.
- [x] Test in `repository/src/abi.rs`:
      `a_wrong_mfpc_major_version_is_now_rejected_by_this_crate_too` asserts
      `parse_abi_index` returns the explicit "unsupported MFPC major version"
      error for v1 and v3, **and** that `abi_index_json` swallows it to `{}` — so
      the silent failure mode is asserted, not inferred from a green publish.
- [x] Added task: move `Section`, `encode_sections` and `MFPC_MAJOR_VERSION` from
      `bytes.rs` into `mfpc.rs`. Creating `mfpc.rs` had left
      `MFPC_MAJOR_VERSION` defined **twice**. Resolves plan-126-B's second Open
      Decision; adds `encode_sections_round_trips_through_read_section_table`.
- [x] Added task: delete the compiler's local `WIRE_LIBC_*`, `WIRE_LIB_TYPE_*`,
      `NATIVE_LIBRARY_HASH_LEN` and `ABI_FORMAT_VERSION`. They **survived** the
      `mfpc::*` glob re-export because a local `const` silently shadows a glob
      import — compiling and passing with two copies (Corrections).

Acceptance: MET.
`cargo test -p mfb_wire` → **45 passed**.
`cargo test -p mfb_repository --lib` → **374 passed; 0 failed**;
`abi::` alone → **31 passed** (was 29; +2).
`cargo test --bin mfb binary_repr` → **171 passed; 0 failed**.
`cargo test --bin mfb citations_resolve` → ok.
**The guards are load-bearing, measured:** disabling the major-version check
(`if false && major != MFPC_MAJOR_VERSION`) turned **both**
`mfpc::a_container_declaring_major_version_three_is_rejected` and
`abi::a_wrong_mfpc_major_version_is_now_rejected_by_this_crate_too` red; restored.
`scripts/artifact-gate.sh target/release/mfb all` → 1427 tests, 1593 builds,
**2001 goldens checked, 0 diffs**, `git status tests/` clean. A rebuilt
`packages/jwt/jwt.mfp` is `cmp`-identical to the pre-plan-126-B build. Note the
compiler's decode is **not** purely relocated as the criterion claimed: it gained
the `MAX_MFPC_SECTIONS` ceiling. The 0-diff result is still the expected one —
every real package declares ≤14 sections, far under 256.
The whole-workspace `cargo test --no-fail-fast` is the plan-wide final gate in
follow-plan §5.
Commit: 427af4b4f

## Validation Plan

- **Tests:** DONE. `wire/src/validation.rs` (6 moved + 1 divergence),
  `wire/src/terminal_safe.rs` (4 moved), `wire/src/mfpc.rs` (**13** — the 5
  planned negatives became 9 guard/decode tests, plus 2 moved/new
  `encode_sections` tests and 2 constant/`checked_usize` pins),
  `repository/src/abi.rs` (**29** existing stayed green, not the planned 21; +2
  new: the major-version test and the best-effort-posture test).
- **Coverage check:** DONE. Confirmed the `abi.rs` tests actually run rather than
  assuming it: `cargo test -p mfb_repository --lib abi::` → `31 passed`, all
  under `abi::tests::`.
- **Runtime proof:** DONE 2026-09-12. Published `alice#p126pkg@0.1.0` — a package
  with one `EXPORT FUNC answer()` and a section-18 description — to a live
  `mfb-repo` on `127.0.0.1:7792` through `mfb repo publish` (log inclusion
  verified). `GET /packages/alice%23p126pkg` reported `abiIndex` =
  `['answer']` (**non-empty**) and `description` =
  `'plan-126-C runtime proof package.'` (**exact**). The stricter reader did
  not silently empty metadata for a valid package.
  Two execution notes. The package must *export* something, or an empty
  `abiIndex` is indistinguishable from the failure being tested. And the
  registry refuses to start with its database directly in `/tmp`: it requires
  the db's parent directory at mode 700 because it holds the signing key and
  session secret in plaintext, and `/tmp` is 777. That's a correct security
  guard, not a regression. Give the db its own directory.
- **Byte-identity:** DONE. `scripts/artifact-gate.sh target/release/mfb all` →
  2001 goldens, `diffs=0`, run after every compiler-side change in this
  sub-plan. Valid evidence for Phases 1–2 and for the compiler half of Phase 3;
  **not** evidence for the registry half, which the runtime proof and the
  load-bearing-guard check cover instead.
- **Doc sync:** DONE. Deleted the "restated here because this crate does not
  depend on the compiler crate" comments in `repository/src/abi.rs` and the
  bug-489 rationale in both `terminal_safe` shims. Also re-pointed one spec
  provenance citation the plan did not anticipate
  (`04_audit-format.md`, Phase 2). `.ai/` checked with
  `grep -rn "abi\.rs\|terminal_safe\|read_section_table\|restated here\|section table" .ai/`:
  three hits, **all unrelated**. Two refer to `target/shared/abi.rs` (codegen
  ABI) and one to the Windows PE section table. None describes the registry
  `abi.rs`, where `terminal_safe` lives, or the MFPC reader, so no `.ai/` doc
  change is needed.
- **Acceptance:** `rustup run 1.96.0 cargo test --no-fail-fast`; `tests/cli/cli_repo_publish.rs`;
  `docker build -f repository/Dockerfile .`.
  Docker half DONE 2026-09-12 after Phase 3:
  `docker build --load -f repository/Dockerfile -t mfb-repo-p126:c .` → exit 0,
  compiling `mfb_wire v0.1.0 (/build/wire)` then `mfb_repository`, with the
  registry's `abi.rs` now reading its section table through
  `mfb_wire::mfpc::read_section_table`. The whole-workspace `cargo test` is the
  plan-wide final gate in follow-plan §5.
- **Format:** `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Unify the two package-name validators into one?** Their charsets are identical;
  the registry's adds a 128-byte cap. Recommended: **keep both, renamed** (as in
  Phase 1) until the Phase 1 measurement shows no name in the tree exceeds 128
  bytes — then unifying becomes a safe, separately-tested follow-up rather than a
  side effect of a code move. Alternative: unify now and accept that
  `read_mfp_header` starts rejecting names over 128 bytes. (§Phase 1)
- **`HashMap` or `BTreeMap` for the shared section table?** Recommended `BTreeMap`
  (the registry's choice) so iteration order is deterministic — `HashMap` deciding
  an order has bitten this tree before. Confirm no compiler caller depends on
  `HashMap`-specific API. (§Phase 3)

## Corrections

- **The union of guards is NOT the compiler's set — both copies were missing
  something.** § Design Overview's rejected alternative asserted "the
  divergences both favor the compiler … the union is the compiler's set", and
  the guard table in § Current State listed only two rows where the registry
  lagged. Reading the registry's `read_section_table` in full (it had grown with
  bug-578 after this plan was written) found a third row running the *other*
  way: a declared-section-count ceiling, `MAX_MFPC_SECTIONS = 256`, checked
  before the table walk — which the compiler's inline decode did **not** have
  (`grep -rn MAX_MFPC_SECTIONS src/` → no hits before this phase). So the shared
  reader enforces magic + major version + count ceiling + overflow + truncation
  + duplicate id + `checked_usize`, and **the compiler gained a guard too**, not
  only the registry. The module doc's guard table in `wire/src/mfpc.rs` records
  all three drifted rows.

- **There is no fifth `read_section_table` caller.** Phase 3 named four and
  hedged "the fifth found by grep". Measured before the move:
  `grep -n read_section_table repository/src/abi.rs` → the definition, **four**
  production callers (`parse_vendor_blobs`, `parse_manifest_metadata`,
  `parse_package_description`, `parse_abi_index`), and three test-only uses.
  All four were re-pointed; each keeps its best-effort posture, now pinned by
  `a_non_container_payload_still_means_no_such_section_not_an_error`.

- **`checked_usize`'s rejection is unreachable on a 64-bit host, and the first
  test written for it was wrong.** The plan's "a `u64` offset that does not fit
  `usize` rejected" test cannot be written against this host:
  `usize::try_from(u64::MAX)` *succeeds* where `usize` is 64 bits, so the value
  passes `checked_usize` and is refused one step later by
  `offset.checked_add(length)` ("invalid MFPC section length"). The first draft
  asserted the address-space message and went red for exactly that reason.
  Corrected to (a) assert the combined guard chain refuses a colossal value
  whichever step catches it, and (b) pin `checked_usize` directly, with its
  failure direction under `#[cfg(target_pointer_width = "32")]`. The
  `checked_usize`-vs-`as usize` divergence between the old copies is therefore a
  **32-bit-target** fix; on the macOS/Linux 64-bit hosts CI runs, the two were
  observably equivalent. No 32-bit target build exercises it.

- **A local `const` silently shadows a glob import, so re-exporting the section
  ids did not remove the compiler's duplicates.** After adding
  `pub(crate) use mfb_wire::mfpc::*;` to `src/binary_repr/mod.rs`, the compiler's
  own `WIRE_LIBC_*`, `WIRE_LIB_TYPE_*`, `NATIVE_LIBRARY_HASH_LEN` and
  `ABI_FORMAT_VERSION` definitions **still compiled** — a local item beats a glob
  rather than colliding with it — with identical values, so every test passed
  and two copies of the wire vocabulary remained behind a comment claiming one.
  Found by grepping for the definitions, not by the compiler. Removed; the
  deletion site carries a note so the trap is visible to the next move.

- **plan-126-B's second Open Decision resolved here: `Section`,
  `encode_sections` and `MFPC_MAJOR_VERSION` moved into `mfpc.rs`.** Creating
  `mfpc.rs` with its own `MFPC_MAJOR_VERSION` left the constant defined **twice**
  (`wire/src/bytes.rs` from B, `wire/src/mfpc.rs` from C) — no error, because the
  only ambiguous *use* in the compiler was the inline decode this phase deleted.
  Moved the writer beside the reader so the one field has one writer, one reader
  and one definition (`grep -rn "const MFPC_MAJOR_VERSION"` → exactly
  `wire/src/mfpc.rs`), and added
  `encode_sections_round_trips_through_read_section_table` — before this the
  writer and its reader lived in different files (the reader in two copies) and
  nothing tested the pair.

- **`read_u64` in `abi.rs` became dead code and was removed.** Its only
  production caller was the registry's `read_section_table`. Keeping it would
  have left a helper used by nothing but a test row that tested the helper; the
  shared equivalent `bytes::checked_u64_at` has its own tests.

- **`abi.rs` had grown well past the plan's measurement.** § Measured
  populations said 1,063 lines / 21 tests; measured 2026-09-12 before this phase:
  **1,488 lines / 29 tests** (bug-578's format ceilings landed after the plan was
  written). After this phase: **31** tests (+2 guard/posture tests).

- **Phase 1 measurement recorded (the Open Decision's input):** longest header
  name across all 160 committed `.mfp` fixtures is **34 bytes**; zero exceed
  `PACKAGE_LIMIT` (128). Unifying the two name policies is de-risked but remains
  a behavior change, left to a separately-tested follow-up.

- **Both moves in this sub-plan invalidated a spec provenance citation, and the
  citation gate caught neither.** Phase 2's `terminal_safe` move left
  `04_audit-format.md` citing a shim. `spec_citations_resolve` is file-level only
  and the shim even still contains the symbol's name. Same class as
  plan-126-B's `validate_mfp_signature_header` citation. The durable lesson:
  any symbol move must grep `src/docs/spec/` for `[[…:Symbol]]`.

- **Follow-up candidate, deliberately not acted on:** `state_is_active`
  (registry, plan-126-A) and `state_is_floating_eligible` (client) are the same
  allowlist in two crates — exactly the duplication `mfb_wire` exists to remove.
  Unifying them means editing `src/cli/pkg.rs`, which plan-126-A's Non-goals
  forbid, so it stays two predicates held together by a five-state agreement
  test.

## Summary

The engineering content is Phase 3: two hand-synchronized copies of a tamper check
on signed bytes become one, and in the process the registry gains an MFPC
major-version check and a checked width conversion it has been missing. The
insidious part is that the failure mode is silent — `abi_index_json` maps every
error to `{}` — so the phase's acceptance deliberately asserts the guard fires
rather than trusting a green publish test. Phases 1 and 2 are mechanical; the
package-name unification is explicitly deferred behind a measurement rather than
folded into a code move.
