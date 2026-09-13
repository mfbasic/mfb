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

- [ ] The codegen `TypeModel` knows every builtin registry record a program can hold,
  whether or not it imports the declaring package (C3). Acceptance: a `tcp`-only program
  that binds, copies and reassigns a `tcp::localAddress` result resolves `net.Address` in
  `record_fields`; the artifact gate over the existing fixtures is unchanged by this task
  alone.
- [ ] The helper-tier marshaller can inline a nested record field whose byte size the caller
  already holds (C2). Acceptance: a unit test builds a record with a known-size nested
  record field and the emitted size/offset stores match `emit_record_block_size_to_slot`'s
  rule.
- [ ] A helper-tier `List OF <flat record>` builder: from `count` built element blocks and
  their sizes, it allocates the list, pads each element start to 8
  (`list_element_padding_alignment`), writes the entries, copies the blocks and frees the
  per-element scratch. Acceptance: unit test on the emitted entry/data arithmetic, plus the
  `net::lookup` runtime pins below.
Commit:

### Phase 1 — `net::Address`, `udp::Datagram`, `net::PingResult`

- [ ] `emit_address_host_and_record` builds the canonical `Address` image through the
  marshaller and frees its host-`String` scratch; `emit_address_from_sockaddr` /
  `emit_address_from_host_and_port` leave the record pointer and its byte size.
- [ ] `net::lookup` builds its list with the record-list builder.
- [ ] Both `net::ping` backends build `PingResult` with `address` inlined.
- [ ] `udp::receive` builds `Datagram` with `from` and `bytes` inlined.
- [ ] `lower_net_address_helper` and both macOS TLS address helpers return the canonical
  record.
- [ ] Readers rebased (`recordBase + offset`): `lower_net_endpoint_helper` address arm,
  `lower_net_send_to_helper`, `connect_arg_prologue` address arm, both ping `address_form`
  arms.
- [ ] `net::Address` and `udp::Datagram` removed from `is_pointer_string_record` (and its
  tests' table).
- [ ] Layout-pinning tests updated: `ping_result_offsets_match_the_declared_field_order`,
  `datagram_field_order_is_from_then_bytes`, the `net`/`tcp`/`udp`/`tls` rows of
  `codegen_helper_scratch_release`.
- [ ] bug-601 regression tests committed and green: the three repros flip (`ys=0 xs=1`,
  no crash, correct hosts), with the `List OF String` contrast.
- [ ] bug-599 RSS pin committed: `net::lookup`, `tcp::localAddress` and a user-built
  `List OF net::Address` flat at >=200k iterations.
- [ ] Positive pins green: `every_address_reads_back_after_its_builder_scratch_is_freed`,
  `tests/net/rt_net_address_*`, `rt_tls_listener_local_address`,
  `rt_tls_listener_thread_transfer`.
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

- [ ] Artifact gate: goldens regenerated; every package that moves is explained.
- [ ] Acceptance over the `net`/`tcp`/`udp`/`tls`/`http`/`audio` fixtures.
- [ ] Runtime on Linux (box 2223) and Windows (box 2230) for the socket packages.
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
