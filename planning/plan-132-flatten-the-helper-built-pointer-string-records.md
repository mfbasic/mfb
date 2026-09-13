# plan-132: flatten the helper-built pointer-`String` records

Status: In progress (worktree `worktree-P-132`, forked from `main` at `3985fa594`)
Decided: 2026-09-12 (owner) — see `planning/bug-backlog.md` Open decisions, bug-599, bug-601
Closes: bug-599 (the remaining leak) and bug-601 (for these records)

## Goal

`net::Address`, `udp::Datagram` and `audio::AudioDevice` use the ordinary record layout, the
one every other record with a `String` or nested-record field already uses. After that,
nothing in the compiler treats them specially.

## Why this is rewiring, not new design

The layout machinery already exists and is used by every other record:
- the spec-canonical record image (`spec/memory/03_heap-values.md` §Record): an 8-byte slot
  per field, with each `String` and flat composite inlined into a trailing data region and
  its slot holding a block-relative offset;
- the helper-tier builder `codegen::memory::marshal::record::emit_build_inlined_record`;
- field reads, copy, drop, collection embedding and thread transfer.

These three records are off that path only because their **hand-written native emitters**
still build and read a fixed-size block with `String`s as absolute pointers.
`is_pointer_string_record` exists solely to keep the rest of the compiler consistent with
those emitters. That exception is why the records are not `memcpy`-copyable, which in turn
causes bug-601 (`MUT` copies alias) and bug-599 (no drop is possible).

## Prerequisites

Measured 2026-09-12 when execution started. The probe script is `/tmp/p132/probe.sh`
(scratch, not a pin — the committed regression tests are Phase 1's tasks).

| Must be true | Command | Status |
|---|---|---|
| The base compiler reproduces bug-601 (so a flip is evidence) | `bash /tmp/p132/probe.sh ~/Development/mfb/target/release/mfb /tmp/p132/base` (binary built 22:49; the only later commit `3985fa594` touches `planning/todo.md` alone) | MET — `p601_remove` prints `ys=0 xs=0`; `p601_append` and `p601_append_get` print `xs=96` then exit 139; contrast `List OF String` prints `ys=0 xs=1` |
| The worktree builds a release compiler | `cargo build --release` in the worktree | MET — exit 0 |
| Linux box 2223 answers, with cargo | `ssh -p 2223 test@127.0.0.1 'uname -m; command -v cargo'` | MET — `aarch64`, `/usr/bin/cargo` |
| Windows box 2230 answers | `ssh -p 2230 test@127.0.0.1 ver` | MET — `Microsoft Windows [Version 10.0.26100.9445]` |

## Scope — the native emitters that still assume the pointer layout

**Builders**
- `os/socket/shared.rs`: `emit_address_host_and_record`, shared by
  `emit_address_from_sockaddr` and `emit_address_from_host_and_port`. It currently allocates
  a fixed 16-byte `[host ptr][port]` record. Its callers are `net::lookup`
  (`net/gen_io.rs`), both `net::ping` backends (`net/gen_ping.rs`), `udp::receive`
  (`udp/gen_io.rs`), the macOS tls address helpers (`tls/gen_macos/address.rs`) and the
  shared socket address path (`lower_net_address_helper`: `tcp`/`udp` `localAddress`,
  `tcp::remoteAddress`, Linux/Windows `tls` addresses).
- `net/gen_io.rs` `net::lookup`: copies a fixed 16 bytes per element into the list, with a
  fixed `VALUE_LENGTH`.
- `udp/gen_io.rs`: `Datagram` is a fixed 16-byte `[from ptr][bytes ptr]`.
- `net/gen_ping.rs`: `PingResult` stores the `Address` pointer at `RESULT_OFFSET_ADDRESS`
  (fixed 40 bytes). `PingResult` is an ordinary record, but its `address` field becomes
  inlined once `Address` is flat, so its layout moves with this plan.
- `audio/gen_{alsa,macos,windows}_devices.rs`: `AudioDevice` is six fixed 8-byte slots
  (`DEVICE_FIELD_*` in `audio/gen_shared.rs`) with `id`/`name` as pointers.

**Readers** — measured census (`grep -rn "load_u64(.*, 0)\|DEVICE_FIELD_ID"` over the
emitters, each hit read):
- `os/socket/shared.rs` `lower_net_endpoint_helper` `address` arm (`tcp::connect(Address)`);
- `udp/gen_io.rs` `lower_net_send_to_helper` (`udp::send(sock, Address, …)`);
- `tls/gen_shared.rs` `connect_arg_prologue` `address` arm (all three TLS backends);
- `net/gen_ping.rs` `address_form` arm in `lower_ping_posix` and `lower_ping_windows`;
- `audio/gen_macos_shared.rs` `emit_select_device`, `audio/gen_alsa_shared.rs`
  `emit_device_cstring`, `audio/gen_windows_open.rs` `emit_widen_device_id`.

**Tests that pin the old layout** (update, do not delete the intent):
`udp::tests::datagram_field_order_is_from_then_bytes`,
`net::gen_ping::tests::ping_result_offsets_match_the_declared_field_order`, and the
`net`/`tcp`/`udp`/`tls` rows of `tests/codegen/codegen_helper_scratch_release.rs`.

## Phases

1. **`net::Address`, with `Datagram` and `PingResult`.** Build `Address` through the
   canonical record builder, and rebase every reader from pointer to offset. Embed
   variable-length elements in `net::lookup`'s list. Build `Datagram` and `PingResult` the
   same way. Remove `net::Address` and `udp::Datagram` from `is_pointer_string_record`.
2. **`audio::AudioDevice`.** Same change for the three device-list builders and their
   readers. Remove it from the predicate.
3. **Delete the leftovers.** With the predicate empty, remove `is_pointer_string_record`,
   its `CodeBuilder` wrapper, its three call sites in
   `collection/layout/builder_collection_layout.rs` and `pointer_string_record_tests`.
   Update the spec's §Record "excluded" note. Correct the stale doc comment that lists
   `Error`/`ErrorLoc` (the spec says they are already flat; bug-602 tests it). Update
   `.ai/collections.md` and `.ai/codegen-invariants.md`, which describe the class.

## Tasks

### Phase 0 — prerequisites the plan did not list (see Corrections C2, C3)

- [x] The codegen `TypeModel` knows every builtin registry record a program can hold,
  whether or not it imports the declaring package (C3). Acceptance: a `tcp`-only program
  that binds, copies and reassigns a `tcp::localAddress` result resolves `net.Address` in
  `record_fields`; the artifact gate over the existing fixtures is unchanged by this task
  alone — corrected by C5 to "changes only what the tag-order fix explains". Evidence:
  `registry::builtin_record_layouts` + `TypeModel::finish` (`e04ecae8f`);
  `a_builtin_record_has_its_layout_without_an_import` (a model with no module types
  resolves `net.Address`, `net.PingResult` with `net.PingStatus`/`net.Address` fields,
  `udp.Datagram` with `net.Address`, and adds no bare `Address`) and
  `a_qualified_union_is_tagged_once_through_the_package_constructor`:
  `cargo test --release --bin mfb union_tag_tests` → 6 passed, EXIT=0. Gate on
  `e04ecae8f`: 5 diffs, all `json_codegen_cover_rt`, localized to `MATCH` tag constants
  (C5); `bash scripts/regen-ncodesum.sh /tmp/p132/mfb-d1` → `144 golden(s) refreshed`,
  and `git status --short tests/` shows exactly those five `.ncodesum` files changed;
  test-accept over every `json` fixture → 15 passed.
- [x] The helper-tier marshaller can inline a nested record field whose byte size the caller
  already holds (C2). Acceptance: a unit test builds a record with a known-size nested
  record field and the emitted size/offset stores match `emit_record_block_size_to_slot`'s
  rule. Evidence: `memory::marshal::record::emit_build_inlined_record_sized`;
  `a_nested_record_field_builds_from_its_known_size_in_both_passes` (the size slot is read
  by both the sizing and the copy pass), `a_nested_record_field_without_its_size_is_refused`,
  `a_known_size_on_a_slot_field_is_refused`, plus C6's
  `the_fixed_regs_are_the_legacy_names` and `fresh_regs_never_write_a_vreg_the_caller_already_holds`:
  `cargo test --release --bin mfb memory::marshal` → 9 passed, EXIT=0. The runtime half of
  the size rule is the `udp::Datagram`/`net::PingResult` pins in Phase 1.
- [x] A helper-tier `List OF <flat record>` builder: from `count` built element blocks and
  their sizes, it allocates the list, pads each element start to 8
  (`list_element_padding_alignment`), writes the entries, copies the blocks and frees the
  per-element scratch. Acceptance: unit test on the emitted entry/data arithmetic, plus the
  `net::lookup` runtime pins below. Unit half: `memory::marshal::record_list` (one
  allocation and two free sites, `LIST`/`OBJECT` header, both passes pad to 8, fresh
  regs) in the 9 passed above. Runtime half: `net::lookup` now builds through it, and
  `a_looped_net_lookup_runs_at_constant_rss`,
  `every_address_reads_back_after_its_builder_scratch_is_freed` and
  `rt_net_address_record_layout` pass (Phase 1 evidence below).
- [x] C8 (found while predicting counts): the receive buffer of `udp::receive` and the
  packet/receive buffers of both `net::ping` backends are released at `done` as
  `HelperScratch`. Evidence: `codegen_helper_scratch_release` rows `udp::receive
  (6, 5, 1)`, `net::ping`/`pingAddr (7, 6, 3)`, predicted by hand before measuring and
  matched (9 passed); `a_looped_udp_receive_runs_at_constant_rss` passes.
- [x] D1 (found while probing C3): a project `TYPE Address` or `TYPE AudioDevice` cannot be
  constructed — `LET a = Address["x"]` fails `2-203-0043 TYPE_UNKNOWN_VALUE` with no import
  at all, while the same program with `Url`, `Datagram` or `KeyPair` (also builtin record
  leaves) builds (`/tmp/p132/leaf_*`, base compiler). Root cause: both copies of the
  compiler-owned-record rule (`ir::shape::read_only_record`,
  `ir::verify::read_only_record_type`, and `term::is_read_only_record`) matched through
  `is_builtin_named`, which accepts the bare leaf; `TermSize` is refused the same way
  (`/tmp/p132/d1_termsize`). Source cannot name an imported builtin record bare (`AS
  Address` under `IMPORT net` → `SYMBOL_UNKNOWN_TYPE`, `/tmp/p132/d1_bare_imported`), so a
  bare leaf there is always a project type. Fixed with `ParameterType::is_builtin_qualified`;
  the bug-483 pin's disproved bare half corrected. Verified: `cargo test --release --bin mfb
  --test rt_shadowing_type_name_diagnostics -- ir::verify ir::shape shadowing …` → 503 + 8
  passed, EXIT=0 (two new cases: project `Address`/`AudioDevice`/`TermSize` construct and
  `WITH`-update with and without the import; the qualified forms stay refused);
  `bash scripts/test-accept.sh target/release/mfb /tmp/p132/accept_d1_json
  net_address_read_only_invalid device_literal_invalid func_term_terminalSize_invalid …` →
  15 passed, EXIT=0 (the three read-only fixtures' goldens unchanged).
Commit:

### Phase 1 — `net::Address`, `udp::Datagram`, `net::PingResult`

- [x] `emit_address_host_and_record` builds the canonical `Address` image through the
  marshaller and frees its host-`String` scratch; `emit_address_from_sockaddr` /
  `emit_address_from_host_and_port` leave the record pointer and its byte size
  (`AddressSlots`, `MarshalRegs::fresh`).
- [x] `net::lookup` builds its list with the record-list builder.
- [x] Both `net::ping` backends build `PingResult` with `address` inlined
  (`gen_ping::emit_ping_result`; `PING_RESULT_TYPE_ID` ungated for it).
- [x] `udp::receive` builds `Datagram` with `from` and `bytes` inlined (dead `text` arm
  deleted, C7).
- [x] `lower_net_address_helper` and both macOS TLS address helpers return the canonical
  record.
- [x] Readers rebased (`recordBase + offset`): `lower_net_endpoint_helper` address arm,
  `lower_net_send_to_helper`, `connect_arg_prologue` address arm, both ping `address_form`
  arms. Census: every registry member taking `net::Address`
  (`grep -rn "ADDRESS_TYPE_ID\|super::address()" src/codegen/builtins`) is one of these.
- [x] `net::Address` and `udp::Datagram` removed from `is_pointer_string_record` (and its
  tests' table).
- [x] Layout-pinning tests updated: `ping_result_offsets_match_the_declared_field_order` →
  `ping_result_fields_match_what_the_backends_hand_the_marshaller`,
  `datagram_field_order_is_from_then_bytes` (now also asserts both fields inlined), the
  `net`/`tcp`/`udp`/`tls` rows of `codegen_helper_scratch_release` (every new triple
  predicted by hand before measuring; `cargo test --release --test
  codegen_helper_scratch_release` → 9 passed). Unit: `cargo test --release --bin mfb --
  memory::marshal net:: udp:: pointer_string_record_tests union_tag_tests tls::` → 61
  passed, EXIT=0.
- [x] bug-601 regression tests committed and green: the three repros flip (`ys=0 xs=1`,
  no crash, correct hosts), with the `List OF String` contrast.
  `a_mut_copy_of_an_address_list_is_independent_of_its_source` in
  `rt_net_address_record_layout` (2 passed). Probes on the Phase 1 binary: macOS
  `/tmp/p132/p1.log`, Linux box 2223 and Windows box 2230 (cross-built, all exit 0,
  `ys=0 xs=1` / `ys=2 xs=1` / hosts `127.0.0.1:80`).
- [x] bug-599 RSS pin committed: `net::lookup`, `tcp::localAddress` and a user-built
  `List OF net::Address` flat at >=200k iterations — plus `udp::receive`.
  `cargo test --release --test rt_scope_drop_leaks -- --test-threads=1 a_looped_ …` → 5
  passed. Peak RSS 200k → 400k on macOS, before → after: lookup 99.8→198.3 MB became
  1.2→1.4 MB; localAddress 38.0→74.9 became 1.1→1.1; user-built list 196.4→391.8 became
  1.1→1.1; udp receive 157.5→313.6 became 1.49→1.49.
- [x] Positive pins green: `every_address_reads_back_after_its_builder_scratch_is_freed`,
  `tests/net/rt_net_address_*`, `rt_tls_listener_local_address`,
  `rt_tls_listener_thread_transfer` — 1 + 2 + 1 + 2 + 1 passed. A `tcp`-only and a
  `udp`-only program (no `IMPORT net`) copy and pass the flat records correctly on all
  three platforms (C3's prerequisite, measured).
Commit:

### Phase 2 — `audio::AudioDevice`

- [ ] macOS, ALSA and Windows `lower_devices` build each `AudioDevice` through the
  marshaller and the list through the record-list builder.
- [ ] Readers rebased: `emit_select_device`, `emit_device_cstring`, `emit_widen_device_id`
  (C4: the Windows reader also stops reading the record pointer as the id `String`).
- [ ] `audio::AudioDevice` removed from `is_pointer_string_record`.
- [ ] Emitted-code pins for the builders and readers (no audio device exists on any host, so
  there is no runtime proof; recorded in the landing record).
Commit:

### Phase 3 — delete the leftovers

- [ ] `is_pointer_string_record`, its `CodeBuilder` wrapper, its three call sites and
  `pointer_string_record_tests` deleted; any code path reachable only through the class
  found by census and deleted.
- [ ] Spec §Record "excluded" note removed; `mfb spec` renders it.
- [ ] Stale comments corrected: the `Error`/`ErrorLoc` doc comment, `record.rs` module doc,
  `tests/net/rt_net_address_record_layout.rs` header, `src/target/shared/validate/mod.rs`.
- [ ] `.ai/collections.md` and `.ai/codegen-invariants.md` updated.
- [ ] bug-599 and bug-601 docs and the backlog updated.
Commit:

### Verification (plan end)

- [~] Artifact gate: goldens regenerated; every package that moves is explained. Phase 1
  done: `bash scripts/artifact-gate.sh /tmp/p132/mfb-p1 all` → `2013 golden(s) checked,
  30 diff(s)` = `http`, `net`, `resource-xfer-slots`, `tcp`, `tls`, `udp` × 5 targets.
  Each localized by building its host `-ncode` with the Phase-0 and Phase-1 compilers
  (`/tmp/p132/fdiff.sh`): changed functions are exactly the rewritten builders/readers
  plus `main` (`net`: lookup, ping, pingAddr; `tcp`: connectAddr, localAddress,
  remoteAddress; `udp`: localAddress, receive, send, sendText; `tls`: net.lookup,
  tls.connectAddr, localAddress, localAddressListener; `http`: tcp.connectAddr,
  tcp.localAddress, tls.connectAddr; `resource-xfer-slots`: tls.connectAddr only — it
  imports `tls`); none added or removed. `net`'s `main` gains ownership code only
  (`_mfb_arena_free` 9 → 123 behind owned-value guards, owned-collection drops 4 → 6, one
  flat-copy allocation — the bug-601 copy). Crypto did not move (fixed marshaller regs).
  `bash scripts/regen-ncodesum.sh /tmp/p132/mfb-p1` → `144 refreshed`, and exactly those
  30 sums changed. Remaining: Phase 2's `audio` movement.
- [~] Acceptance over the `net`/`tcp`/`udp`/`tls`/`http`/`audio` fixtures. Phase 1:
  `bash scripts/test-accept.sh /tmp/p132/mfb-p1 /tmp/p132/accept_p1 <the 122 fixtures under
  those package directories>` → `acceptance tests passed (123 test(s) ran)`, EXIT=0.
  Remaining: rerun after Phase 2.
- [x] Runtime on Linux (box 2223) and Windows (box 2230) for the socket packages.
  Cross-built the probe set with the Phase 1 compiler (`/tmp/p132/xbuild.sh`, targets
  `linux-aarch64` and `windows-x86_64`) and ran it on each box: every program exits 0;
  the bug-601 repros print `ys=0 xs=1` / `ys=2 xs=1` with `127.0.0.1:80` hosts; the
  `tcp`-only and `udp`-only programs work; the lookup/localAddress/user-list/udp loops
  complete. Box 2223 has no GNU `time`, so RSS is measured on macOS only (the pins).
- [ ] Full suite + test-accept once, merged with main.

## Verification

- **bug-601:** its three repros must flip. `MUT ys = xs; ys = removeAt(ys, 0)` must print
  `ys=… xs=<original>`, and the in-place `append` must no longer crash.
- **bug-599:** the list, record and host `String` must be freed. Add an RSS pin at >=200k
  iterations that is flat (the fixer's probes are in bug-599's doc).
- **Positive pins:** `.host`, `.port`, `.from`, `.bytes` and device fields read back
  correctly, and copied-out elements survive their list. Keep bug-599's existing pin
  `every_address_reads_back_after_its_builder_scratch_is_freed` green.
- **Gates:** unit suite, artifact gate (goldens in net/tcp/udp/tls/http/audio will move;
  explain each package that moves), and acceptance over those packages' fixtures.
- **Runtime per backend:** the socket packages on Linux (box 2223) and Windows (box 2230)
  as well as macOS. `audio` has no device on any host, so it rides emitted-code pins; say so
  in the landing record.

## Corrections

- **C1 — the plan had no Prerequisites section and no task ledger.** Both added at the start
  of execution, with the gate measured (table above).
- **C2 — the helper-tier marshaller cannot build two of the planned records.**
  `memory/marshal/record.rs` `emit_inlined_block_size` returns
  `"helper-tier record marshaller cannot inline a nested record field"` for any inlined
  record field. Once `Address` is flat, `record_field_is_inlined` inlines it into both
  `udp::Datagram.from` and `net::PingResult.address`, so "build `Datagram` and `PingResult`
  the same way" is not possible as written. The builder of the nested `Address` already
  knows its size, so Phase 0 extends the marshaller to take a caller-held size.
- **C3 — a program that does not import `net` has no `net.Address` in its type model.**
  Measured: a `tcp`-only program binding `tcp::localAddress(server)`
  (`mfb build --nir /tmp/p132/nircheck`) has binds typed `net.Address` and a NIR `types`
  table with no `net.Address` entry; the same query with `IMPORT net/udp/audio`
  (`/tmp/p132/nircheck2`) lists `net.Address`, `udp.Datagram`, `net.PingResult` and
  `audio.AudioDevice` with qualified field types. `TypeModel::from_module` reads only
  `module.types`, so in the first program `record_field_is_pointer` falls to
  `named_field_is_pointer`, which answers "not a pointer" for an unknown nominal. On the
  pointer layout that default is harmless (an 8-byte pointer copy, no drop). On the flat
  layout it would classify the record as a scalar and copy or drop it wrongly. A missing
  prerequisite, added as Phase 0's first task.
- **C4 — Windows `emit_widen_device_id` reads the record pointer as the device id.**
  `gen_windows_open.rs` is `include!`d into `gen_windows.rs`, where `DEVID_OFF` (136) is
  the slot the open stores its first argument in (`store_u64(return_register(), …,
  DEVID_OFF)`); that argument is the `AudioDevice` record. `emit_widen_device_id` loads
  `DEVID_OFF` and reads its length at `+0` and its bytes from `+8`, which are the `id`
  pointer and the `name` pointer's bytes. A pre-existing defect in a reader this plan
  rewrites; Phase 2 fixes it.
- **C5 — Phase 0's first task was predicted gate-neutral; it moved 5 goldens, and the
  cause is a pre-existing tag double count, not the new layouts.**
  `bash scripts/artifact-gate.sh /tmp/p132/mfb-0a all` (binary of `e04ecae8f`) →
  `1437 tests, 1603 build(s), 2013 golden(s) checked, 5 diff(s)`, all
  `byte-identity/json/json_codegen_cover_rt` (every target). Localized by building that
  fixture's `-ncode` with the base and Phase-0 compilers: 15 of 160 functions differ
  (`main`, the `#json_*` bodies, three `thread_copy` functions), and in `#json_get` the
  only change is `MATCH` tag constants (`cmp 10` → `cmp 4`, `cmp 6` → `cmp 0`). The old
  values are the tags of `json.JsonObj`/`json.JsonArr` in a sorted set that ALSO holds
  their bare aliases (`JsonArr`…`JsonStr` = 0–5, `json.JsonArr`…`json.JsonStr` = 6–11):
  `from_module_and_packages` called `from_module`, which had already aliased, and then
  recomputed tags — the order the code's own comment forbids. `TypeModel::finish` runs
  the passes once, in order, so the tags are dense. Observability: every tag consumer
  (`builder_value_semantics.rs` ×2, `builder_arena_transfer.rs`,
  `builder_resource_cleanup.rs`, `builder_values.rs`) reads `union_variant_tags`, and no
  `json` emitter hardcodes a tag (`grep` over `src/codegen/builtins/json`); user unions
  have no dot, get no alias and did not move (0 other diffs). Kept, pinned by
  `a_qualified_union_is_tagged_once_through_the_package_constructor`, and proven at
  runtime by acceptance over every `json` fixture.
- **C6 — the record marshaller wrote FIXED vreg names, which the plan's new call sites
  cannot use safely.** `memory/marshal/record.rs` wrote the literals `%v9`..`%v14`,
  documented as safe because "callers spill everything live to frame slots first".
  A helper's vregs are numbered by its own allocator from `%v0`
  (`engine/util/vreg_frame.rs` `Vregs::next`), and a `HelperScratch` keeps its pointer
  in vregs until the helper's `done` (`memory/arena/native_arena.rs`
  `emit_helper_scratch_release`). The socket helpers this plan routes through the
  marshaller draw vregs all through their bodies, so whether a fixed name collided with
  a live one would depend on how many were drawn first — a silent wrong free, invisible
  to any emitted-code test. Fixed by construction: `MarshalRegs::fresh(&mut vregs)`
  for every new caller, `MarshalRegs::fixed()` (the unchanged legacy names, pinned by
  `the_fixed_regs_are_the_legacy_names`) for `crypto::generate`'s three existing calls,
  and a test per marshaller that fresh regs never write a vreg the caller holds.
- **C7 — `udp::receive`'s `text` arm is dead and is deleted, not ported.**
  `lower_net_receive_from_helper(…, text)` has one caller,
  `udp/func_receive.rs` `lower_receive`, which passes `false`
  (`grep -rn "lower_net_receive_from_helper(" src`), and the record that arm built
  (`DatagramText`) is retired — `udp::tests::no_text_receive_shape_survives` asserts
  it is not declared. Porting it to the flat layout would be building a record the
  type model cannot resolve for a path nothing reaches. `emit_string_result_build`
  stays: `tcp/gen_io.rs` and `process/func_receive.rs` still use it.
- **C8 — three helpers this plan rewrites leaked their own I/O buffers on every call.**
  Found while predicting the new `codegen_helper_scratch_release` counts by hand
  before measuring them. `grep -n "emit_alloc\|emit_arena_free\|HelperScratch"` over
  each helper:
  `udp/gen_io.rs` `lower_net_receive_from_helper` allocates its `maxBytes + 1`
  `recvfrom` buffer and has no free of it on any path; `net/gen_ping.rs`
  `lower_ping_posix` allocates the echo packet (`PKT_OFFSET`, `size + 8`) and the
  receive buffer (`BUF_OFFSET`, `RECV_CAPACITY`) and frees neither;
  `lower_ping_windows` does the same with `W_REQ` and `W_REPLY`. Every
  `udp::receive` — including every timeout, the common exit of a polling receive —
  and every `net::ping` grew the arena by those buffers, and the `udp::receive`
  RSS pin this plan adds would stay red after the flatten because of it. Fixed with
  the bug-574 pattern: each buffer is a `HelperScratch` declared before the first
  branch to `done`, named right after its allocation, and released at `done`, so
  every exit frees it and the exits before the allocation skip it on the null guard.
