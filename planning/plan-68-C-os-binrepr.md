# plan-68-C: OS backends + binary_repr object writers

Last updated: 2026-07-27
Overall Effort (AI): large (3h–1d)   (whole plan-68 feature)
Effort (Human): large (3h–5h)
Effort (AI): medium (1h–2h)
Depends on: plan-68-A
Produces: nothing; the eleven files below reach ≥95% line coverage and drop off
the gate's GATE-FAILURE list. No `scripts/coverage-exceptions.txt` change — see
"Classification" below: every one of these files is unit-coverable backfill, not
an integration-only exception.

Part **C** of plan-68. Shared goal, prerequisites, dependency graph, measured
populations, the two work-kinds (except vs. backfill), and the standing
requirements ("tests live in `#[cfg(test)] mod tests`", "run the full
`cargo test`", "a found bug is fixed not worked around", git-per-file discipline)
live in the overview: [plan-68-coverage-gate.md](plan-68-coverage-gate.md).
Prerequisites are stated there and gate this sub-plan too — re-run them before
starting. This sub-plan consumes A's outputs: the fresh
`target/coverage/coverage.json` (read it for each file's exact uncovered line
list — the region names below were established by reading the source, the line
numbers come from A's report) and A's worklist (which confirms all eleven files
below stay `backfill:C`).

## Classification (why nothing here is excepted)

The plan's except-rule is "unreachable from a unit test — subprocess
(linker/codesign), live network I/O, TTY/socket syscall, or GUI loop." Every file
in C's set was read and none crosses that boundary:

- `src/os/link_encode.rs`, `src/os/windows/link/mod.rs`,
  `src/os/linux/appimage/squashfs/mod.rs`, `src/os/macos/icon.rs`,
  `src/binary_repr/{writer,sections,builder,mod}.rs` — **pure in-memory
  encoders / byte writers / tree builders.** `write_executable` returns a
  `Vec<u8>`; it never spawns a linker (that orchestration is `src/cli/build/**`
  → sub-plan B, and `src/target.rs` → already excepted). `build_icns` returns
  `Vec<u8>` from the `icns`/`image` crates, no syscall.
- `src/os/linux/flavor.rs`, `src/os/linux/mod.rs`,
  `src/os/linux/appimage/mod.rs` — thin flavor accessors + wrappers that write
  into a caller-supplied directory via `std::fs`; every existing test drives them
  against a `tempfile::tempdir()`, so their remaining branches are reachable with
  crafted on-disk fixtures. No subprocess, no network.

So C adds `#[cfg(test)]` tests only; it touches no source logic (unless a test
uncovers a real bug — then AGENTS.md "never leave a bug you found": RED-first
test + own commit).

Precedent to mirror (read the nearest existing module before writing each phase):
`src/os/` carries 16 `#[cfg(test)]` modules (`grep -rl "cfg(test)" src/os`),
including sibling in-file `mod tests` in `windows/link/mod.rs:457`,
`linux/appimage/mod.rs:283`, `linux/mod.rs:93`, `macos/icon.rs:101`, and
`linux/appimage/squashfs/mod.rs:674`. `src/binary_repr/` keeps its tests in a
directory module, `src/binary_repr/tests/` (18 files; `mod.rs:15` declares it),
with shared fixtures in `src/binary_repr/tests/fixtures.rs` and per-target files
`writer_tests.rs`, `sections_tests.rs`, `builder_tests.rs`, `mod_tests.rs` /
`mod_error_path_tests.rs`. New binary_repr tests go into those existing files
(or a new sibling in that dir), reusing `fixtures.rs`.

## Phases

Each phase's Acceptance requires a fresh `sh scripts/coverage.sh` first (it
leaves the profdata the checker reads); the filter arg is a path substring.

### Phase C1 — `src/os/linux/flavor.rs` (5/10)

Small enum with **no `#[cfg(test)]` module yet** (`src/os/linux/flavor.rs:1-27`).
Covered today only through `LinuxFlavor::Glibc` used by `linux/mod.rs` tests, so
the `Musl` match arms and the whole `libc()` fn are uncovered.

- [ ] Add a `#[cfg(test)] mod tests` to `flavor.rs`. Assert:
      `LinuxFlavor::Glibc.libc() == crate::manifest::libraries::Libc::Glibc` and
      `Musl.libc() == Libc::Musl` (covers both `libc()` arms, `flavor.rs:14-17`);
      `Glibc.suffix() == "glibc"`, `Musl.suffix() == "musl"` (both `suffix()`
      arms, `flavor.rs:21-24`); iterate `LinuxFlavor::ALL` and assert it is
      `[Glibc, Musl]` (touches the `ALL` const).

Acceptance: `sh scripts/coverage-check.sh os/linux/flavor.rs` shows ≥95%.
Commit: —

### Phase C2 — `src/os/link_encode.rs` (154/172, 18 uncov)

Pure AArch64 encoders + relocation dispatch (`src/os/link_encode.rs`), **no
in-file test module**. Covered only indirectly by the macOS/ELF linker tests, so
the reach-check error arms and the external/rodata dispatch arms are uncovered.
Uncovered regions (confirm exact lines against A's report):

- [ ] `branch_imm26` (`:20`) — the misaligned/out-of-reach `Err` (`:22-26`):
      call with a `delta` that is not a multiple of 4 and one exceeding ±128 MiB;
      assert `Err` mentions "exceeds the ±128 MiB reach". Cover the `Ok` with a
      small in-reach delta.
- [ ] `adrp_page21` (`:34`) — the over-range `Err` (`:36-40`): pass `pc`/`target`
      more than ±4 GiB apart; assert the "exceeds the ±4 GiB reach" message.
- [ ] `read_u32` / `write_u32` (`:46`, `:57`) — the out-of-bounds `Err` arms
      (`:47-52`, `:59-61`): call with `offset` past a short buffer; assert the
      "exceeds text length" message. Cover the success path too.
- [ ] `symbol_vmaddr` (`:87`) — the not-found `Err` (`:98`); the three `Data`
      cases (`:102-106`): with `rodata = Some((vmaddr,size))` and `offset < size`
      (rodata window), `offset >= size` (writable-data past prefix), and
      `rodata = None` (ELF one-region); and the `Text` case. Build a small
      `EncodedImage` (see `linux/mod.rs:158-173` for the struct literal) with
      Text and Data symbols.
- [ ] `patch_aarch64_reloc` (`:132`) — the six handled arms
      (internal/data/external × branch26/page21/pageoff12) and the two external
      unbound `Err` arms (`:189-196`, `:205-212`/`:223-230`), plus the `_ =>
      Ok(false)` fallthrough (`:241`) for an unhandled kind. Drive with crafted
      `EncodedRelocation`s and populated/empty `stubs`/`got_entries` maps;
      assert the patched `text` word and the error messages.

Acceptance: `sh scripts/coverage-check.sh os/link_encode.rs` shows ≥95%.
Commit: —

### Phase C3 — `src/os/linux/mod.rs` (168/184, 16 uncov)

Public wrappers with an existing `mod tests` (`src/os/linux/mod.rs:93-256`) that
already covers `write_native_object_plan`, `validate_native_object_plan`,
`write_linked_executable` (Glibc), and `write_linked_appdir`. Uncovered: the
`seal_appimage` (`:74`) and `remove_appdir` (`:85`) delegating wrappers, and the
`Musl` flavor path through `write_linked_executable`/`write_linked_appdir`
(the suffix in the output filename).

- [ ] Extend the existing `mod tests`: build an AppDir with `write_linked_appdir`
      into a `tempdir`, then call `seal_appimage(dir, name, LinuxFlavor::Glibc,
      "aarch64")` and assert the returned `<name>-glibc.AppImage` exists and
      begins with `\x7fELF` (the runtime prefix). Then call `remove_appdir` and
      assert the `.AppDir` is gone. (These wrappers only forward to `appimage::`,
      already unit-tested at `appimage/mod.rs:432-482`; this covers the `mod.rs`
      forwarding lines.)
- [ ] Add a `LinuxFlavor::Musl` variant of `writes_linked_executable_static_elf`
      (or parametrize it): assert the path ends `-musl.out`. Covers the Musl
      suffix threading through the wrapper.

Acceptance: `sh scripts/coverage-check.sh os/linux/mod.rs` shows ≥95%.
Commit: —

### Phase C4 — appimage + squashfs cluster: `src/os/linux/appimage/mod.rs` (262/296, 34) and `src/os/linux/appimage/squashfs/mod.rs` (327/351, 24)

Both have rich existing `mod tests` (`appimage/mod.rs:283-`,
`squashfs/mod.rs:674-`) plus a shared `blobs()`/fixture helper. The residual is
error-path branches reachable with crafted on-disk / in-tree fixtures. Group them
— they share the AppDir→tree→squashfs pipeline and fixture style.

`appimage/mod.rs` uncovered (confirm against A's report):

- [ ] `read_dir_node` (`:235`) rare-node arms: the non-UTF-8 name `Err` (`:245`),
      the non-UTF-8 symlink-target `Err` (`:254`), and the "neither file, dir,
      nor symlink" arm (`:266-271`) — build a fixture AppDir containing a FIFO
      (`nix`/`libc::mkfifo`, or `std::process`-free `mkfifo` via `libc`) so the
      device/FIFO branch fires; assert the "is neither a file" message. If a
      non-UTF-8 path cannot be created portably on the test host, note it per-line
      against A's report rather than excepting the whole file.
- [ ] `seal` (`:155`) the `end != runtime.len()` mismatch `Err` (`:177-184`) — is
      defensive against a future blob swap and only fires if `elf_image_end`
      disagrees with the length; the existing `every_runtime_ends_exactly...`
      test proves it does not for shipped blobs. Reach it by unit-testing the
      guard's logic path if extractable; otherwise this is the one genuinely
      supply-chain-defensive arm — record it per-line in A's report. Cover the
      `appdir` missing `Err` (`:166-171`, likely already via
      `seal_reports_a_missing_appdir`).

`squashfs/mod.rs` uncovered:

- [ ] `plan_node` (`:275`) validation `Err`s: empty entry name (`:299`), name
      over `NAME_MAX` (`:301-306`), file past 4 GiB (`:321-325`), and empty
      symlink target (`:344-345`). Build small `SquashTree`s (use the
      `dir(mode)` test helper at `:141`) with each malformed node; assert each
      message. The 4-GiB arm: assert on `size >= u32::MAX` via a stubbed size if a
      real 4-GiB buffer is infeasible — construct the `SquashNode::File` check
      path directly, do not allocate 4 GiB.
- [ ] Any uncovered arm in `write` (`:364`) / `write_inodes` (`:486`) /
      `write_directory_listing` (`:592`) surfaced by A's report — cover with a
      multi-entry directory tree (dir + file + symlink) so all three
      `entry_type_of` (`:654`) arms and the inode-header writer run.

Acceptance: `sh scripts/coverage-check.sh os/linux/appimage` shows both files
(mod.rs and squashfs/mod.rs) ≥95%.
Commit: —

### Phase C5 — `src/os/windows/link/mod.rs` (437/513, 76 uncov)

Pure in-memory PE32+ linker with an existing `mod tests`
(`src/os/windows/link/mod.rs:457-`) covering a text-only image and an
`ExitProcess` import (idata + thunk). Uncovered: the data-bearing section paths,
the external-data relocation arms, the error arms, and the GUI subsystem flag —
all reachable by constructing `EncodedImage` literals (fixture helper at `:462`).

- [ ] `write_executable` (`:278`) data-section layout: build an image with
      `rodata_size > 0` (`.rdata`, `:295`/`:382-391`) and with
      `data.len() > rodata_size` (`.data`, `:296`/`:392-401`); assert the PE
      carries `.rdata`/`.data` sections at the expected RVAs (reuse the
      `read_at_rva`/`le_u32` helpers at `:551`/`:541`).
- [ ] `patch_relocations` (`:220`) arms: `("data","data_pc32")` (`:234`),
      `("external","data_pc32")`/`("external","got_pc32")` (`:248-260`) with a
      populated IAT slot; and the error arms — unsupported `(binding,kind)`
      (`:261-265`), missing external call binding (`:239-245`), missing external
      data binding (`:252-258`). Assert the patched `rel32` and each message.
- [ ] `append_thunks` (`:163`) missing-IAT-slot `Err` (`:175-177`) and the
      displacement-overflow `Err` (`:184-188`); `symbol_rva` (`:200`) the
      undefined-symbol `Err` (`:210`) and the `Data` arm (`:213`); `write_rel32`
      (`:439`) the out-of-range `Err` (`:445-449`) and ±2 GiB overflow (`:452`);
      `write_executable` entry-not-in-text `Err` (`:291`).
- [ ] `gui = true` path (`:280`/`:425`): assert the emitted PE optional-header
      subsystem byte is `WINDOWS_GUI` (2) vs `WINDOWS_CUI` (3) for `gui=false`
      (decode the subsystem field via the existing byte-reading helpers).

Acceptance: `sh scripts/coverage-check.sh os/windows/link/mod.rs` shows ≥95%.
Commit: —

### Phase C6 — `src/os/macos/icon.rs` (74/78, 4 uncov)

Nearly covered (existing `mod tests` at `src/os/macos/icon.rs:101-171` exercises
the default `.icns`, both source-validation rejections, and the squircle mask).
Only 4 lines remain; the file passes at **75/78 = 96.15%**, so a single covered
line suffices.

- [ ] Read A's report for the exact 4 uncovered lines. The likely residual is the
      three `.map_err(...)` closures in `build_icns` (`:60-63`, `:68-69`) that
      format an `icns`-encoder failure — defensive arms that do not fire for a
      valid 1024 canvas. Attempt to reach one via a coverable branch first (e.g.
      any not-yet-run success-path line the report names). If the only residual
      is the encoder-failure formatting, which a unit test cannot trigger with
      valid input, record those specific lines per-line against A's report with
      the boundary "internal `icns`/`image` encoder failure, unreachable with a
      validated 1024×1024 RGBA canvas" — this is a per-line note, not a whole-file
      exception, and only if ≥95% is otherwise unreachable (it needs just 1 line).

Acceptance: `sh scripts/coverage-check.sh os/macos/icon.rs` shows ≥95%.
Commit: —

### Phase C7 — `src/binary_repr/mod.rs` (124/205, 81 uncov)

The public read/build entry points (`src/binary_repr/mod.rs:463-709`); tests live
in `src/binary_repr/tests/` (add to `mod_tests.rs` / `mod_error_path_tests.rs` or
a new sibling, reusing `fixtures.rs`). Most uncovered functions read a `.mfp`;
build one in a `tempdir` from a fixture `IrProject` via
`build_package_binary_repr_bytes` / `build_binary_repr_bytes` and read it back.
Uncovered functions (confirm against A's report):

- [ ] `read_package_native_libraries` (`:481`) — build a binding package `.mfp`
      with a `NativeLibraryTable` (native-library fixtures already exist in
      `tests/native_library_table_tests.rs`); assert the returned name + table.
- [ ] `read_package_foreign_type_refs` (`:535`) and
      `read_package_type_export_hashes` (`:563`) — build a package that
      re-exports a dependency type (`FOREIGN_TYPE_KIND`); assert the ref's
      name/owner/`abi_hash` and the name→hash map. Reuse the cross-package fixture
      in `tests/cross_package_tests.rs`.
- [ ] `read_package_type_exports_resolved` (`:589`) — the foreign-resolution
      branches: owner `.mfp` present (fields filled in), owner absent (`:615-619`,
      name resolves but fields empty), and the depth-cap `Err` (`:600-605`) via a
      re-export cycle fixture; plus the fast `return` when no export is foreign
      (`:597-599`).
- [ ] `package_info_from_mfp` (`:513`), `read_package_identity_id` (`:650`),
      `read_package_ir_with_identity` (`:666`), `write_binary_repr_hex` (`:683`),
      `build_package_binary_repr_bytes` (`:703`) — each with a round-trip fixture;
      assert the decoded info / identity id / IR name / `.hex` file contents.
      Include an `Err` case for a truncated/garbage byte slice where the function
      returns `Result` (e.g. `package_info_from_mfp` on non-`.mfp` bytes).

Acceptance: `sh scripts/coverage-check.sh binary_repr/mod.rs` shows ≥95%.
Commit: —

### Phase C8 — `src/binary_repr/writer.rs` (949/1017, 68 uncov)

Pure lowering/encoding helpers; add to `src/binary_repr/tests/writer_tests.rs`
(and `writer_walker_tests.rs` for the resource-walk fns). Uncovered (confirm
against A's report):

- [ ] `external_type_metadata` (`:45`) and `external_function_metadata` (`:105`)
      and `lower_project_with_external_functions` (`:165`) — the write-path
      foreign-type / external-function branches (bug-390). Drive with a project
      that imports a dependency's type/function; assert the emitted metadata.
- [ ] `parse_function_type` (`:831`) — `ISOLATED FUNC(` prefix, plain `FUNC(`,
      and the `None` (neither prefix) case; `split_function_type_rest` (`:845`) —
      nested-paren depth tracking, the `") AS "` split, and the unmatched `None`.
- [ ] `fixed_raw_from_decimal` (`:936`) — the error arms: empty
      whole+fractional (`:944`), non-numeric whole (`:951-952`), non-digit in the
      first-28 fractional (`:964-965`) and past-28 fractional (`:970-973`),
      overflow (`:988`), and `i64::try_from` overflow (`:990`); plus the
      round-half-up carry (`:977-983`) and the `fractional_value == SCALE` carry
      (`:980-983`). Table-drive with literal→raw pairs and expected `Err`s.
- [ ] `encode_doc_table` (`:1185`) / `docs_from_ir` (`:1222`) — build a
      `PackageDocs` with a package entry and each `DeclDocEntry` kind; round-trip
      via the doc-table decoder (see `tests/doc_table_tests.rs`).
- [ ] Any uncovered arm in `source_type_payload` (`:861`) /
      `concrete_union_variants` (`:899`, the unknown-include `Err` at `:905-910`)
      / `put_field_payload` (`:917`, each visibility arm) / `collect_imported_
      calls_op`/`_value` (`:660`/`:750`) that A's report names.

Acceptance: `sh scripts/coverage-check.sh binary_repr/writer.rs` shows ≥95%.
Commit: —

### Phase C9 — `src/binary_repr/sections.rs` (956/1008, 52 uncov)

Pure table encoders + the type-graph serializer; add to
`src/binary_repr/tests/sections_tests.rs`. Uncovered (confirm against A's report):

- [ ] `TypeTable::type_id` (`:78`) composite arms not yet exercised —
      `result_type` (`:210`), `state_type` (`:224`), `list_type` (`:241`),
      `set_type` (`:255`), `map_type`/`map_entry_type` (`:266`/`:283`),
      `function_type` (`:300`), `thread_type`/`thread_worker_type`
      (`:318`/`:343`), `foreign_type` (`:370`). Intern each type name and assert a
      stable id + that a second call returns the same id.
- [ ] `mark_reexported_foreign_types` (`:435`) and `collect_reachable` (`:470`)
      including its `Err` path — build a type table with a re-exported foreign
      type reachable from an export; assert the reachable set.
- [ ] `read_native_library_table` (`:916`) and `read_native_library_locator`
      (`:944`) error/decode arms, and `table_string` (`:1014`) out-of-range `Err`
      — feed crafted/truncated section bytes; assert each message. Pair with
      `encode_native_library_table` (`:871`) round-trips (some already in
      `tests/native_library_table_tests.rs`).
- [ ] `AbiSerializer::serialize_type_inner` (`:1117`) per-kind arms
      (`serialize_record_type` `:1218`, `serialize_union_type` `:1234`,
      `serialize_enum_type` `:1254`, `serialize_function_type` `:1268`) and
      `serialize_const` (`:1283`) for any const kind A's report shows uncovered.
- [ ] `ResourceTable::add_native` (`:665`) and `add_standard_*` (`:630`/`:639`/
      `:648`) arms not covered by `tests/resource_table_tests.rs`.

Acceptance: `sh scripts/coverage-check.sh binary_repr/sections.rs` shows ≥95%.
Commit: —

### Phase C10 — `src/binary_repr/builder.rs` (231/247, 16 uncov)

The decoded-package accessors (`src/binary_repr/builder.rs`); add to
`src/binary_repr/tests/builder_tests.rs`. Uncovered (confirm against A's report):

- [ ] `resolve_resource_close_name` (`:34`) — all three arms: the
      `BUILTIN_FS_CLOSE_FUNCTION_ID` sentinel (`:39-41`), the
      `BUILTIN_NET_CLOSE_FUNCTION_ID` sentinel (`:42-44`), the function-id index
      hit (`:45-48`), and the `None` (out-of-range id) miss (`:49`). Build a
      package with a resource table exercising each; assert the resolved name.
- [ ] `package_exports` (`:54`) — the missing-function `Err` (`:66-68`) via an
      export whose `function_id` is out of range; the `has_default` param flag
      (`:81`); and the isolated-function flag (`:72`).
- [ ] `package_type_exports` (`:233`) — the missing-from-type-table `Err`
      (`:259-263`) and the `FOREIGN_TYPE_KIND` marker branch (`:264-278`). Reuse
      the cross-package/foreign-type fixtures from `tests/cross_package_tests.rs`.
- [ ] `package_info` (`:92`) global-visibility arms (`:114-118`) and the
      import↔abi-edge join with and without a matching edge (`:152-166`).

Acceptance: `sh scripts/coverage-check.sh binary_repr/builder.rs` shows ≥95%.
Commit: —

### Phase C11 — `src/os/windows/mod.rs` (0/38) — added by plan-68-A

A1 triaged this **backfill:C**, not an exception (overview Corrections). The file
is three thin wrappers over the already-tested `object`/`link` submodules and has
**no `#[cfg(test)]` module** of its own (the whole module carries `#![allow(dead_
code)]` because the Windows target is staged-landed). All three are unit-coverable
on the macOS host — they never spawn a linker; they lower/link in-memory then
`fs::write` into a caller-supplied dir. Add a `#[cfg(test)] mod tests` (mirror the
fixture style in `src/os/windows/link/mod.rs:457` / `object.rs`):

- [ ] `validate_native_object_plan` (`mod.rs:40`) — pure: build a `NativePlan`
      (reuse the object-plan fixture the sibling `object.rs` tests construct) and
      assert `Ok(())`; feed a plan that fails `object::lower_plan(...).validate()`
      and assert `Err`.
- [ ] `write_native_object_plan` (`mod.rs:25`) — into a `tempfile::tempdir`,
      assert the returned `<name>.nobj` path exists and its bytes are the
      `object_plan.to_json()`.
- [ ] `write_linked_executable` (`mod.rs:48`) — build an `EncodedImage` via the
      `link/mod.rs` fixture helper, call with `gui=false` (and once `gui=true`)
      into a tempdir, assert the returned `build/<name>.exe` exists and begins with
      the PE `MZ` magic.

Acceptance: `sh scripts/coverage-check.sh os/windows/mod.rs` shows ≥95% (fresh
`sh scripts/coverage.sh` first).
Commit: —

## Validation Plan

- **Per file:** after a fresh `sh scripts/coverage.sh`, each phase's
  `sh scripts/coverage-check.sh <substring>` shows the file ≥95% (or, for any
  genuinely unreachable defensive line in C4/C6, a per-line note recorded against
  A's report — never a whole-file exception, since every file here has a coverable
  body).
- **Whole letter:** `sh scripts/coverage-check.sh os/ binary_repr/` lists none of
  C's eleven files as a GATE FAILURE.
- **Suite:** `cargo test` → `0 failed` (the full suite, never a single module —
  new tests must not regress it).
- **No source drift:** `git diff --stat` shows changes confined to the eleven
  target files' `#[cfg(test)]` blocks + `src/binary_repr/tests/*` (plus any
  RED-first bug-fix commit if a coverage test uncovers a real defect).
- **No exception-list change:** `git diff scripts/coverage-exceptions.txt` is
  empty — C excepts nothing (per Classification above).

## Corrections

<Filled during execution — record any file that A's fresh report re-scoped, any
uncovered line that turned out genuinely unreachable (with its per-line boundary),
and any real bug a coverage test surfaced (with its fix commit).>
