# plan-132: flatten the helper-built pointer-`String` records

Status: Not started
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

## Scope — the native emitters that still assume the pointer layout

**Builders**
- `os/socket/shared.rs`: `emit_address_host_and_record`, shared by
  `emit_address_from_sockaddr` and `emit_address_from_host_and_port`. It currently allocates
  a fixed 16-byte `[host ptr][port]` record. Its callers are `net::lookup`
  (`net/gen_io.rs`), both `net::ping` backends (`net/gen_ping.rs`), `udp::receive`
  (`udp/gen_io.rs`), the macOS tls address helpers (`tls/gen_macos/address.rs`) and the
  shared socket address path.
- `net/gen_io.rs` `net::lookup`: copies a fixed 16 bytes per element into the list, with a
  fixed `VALUE_LENGTH`.
- `udp/gen_io.rs`: `Datagram` is a fixed 16-byte `[from ptr][bytes ptr]`.
- `net/gen_ping.rs`: `PingResult` stores the `Address` pointer at `RESULT_OFFSET_ADDRESS`
  (fixed 40 bytes). `PingResult` is an ordinary record, but its `address` field becomes
  inlined once `Address` is flat, so its layout moves with this plan.
- `audio/gen_{alsa,macos,windows}_devices.rs`: `AudioDevice` is six fixed 8-byte slots
  (`DEVICE_FIELD_*` in `audio/gen_shared.rs`) with `id`/`name` as pointers.

**Readers** — every native helper that loads `.host`, `.from` or `.id`/`.name` from one of
these records as a pointer: `tcp`/`tls` connect-by-address, `udp::send` to an address,
ping-by-address, audio open-by-device (`gen_{alsa,macos}_shared.rs` read `DEVICE_FIELD_*`).
Find them all with a census before editing; do not work from this list alone.

**Tests that pin the old layout** (update, do not delete the intent):
`udp::tests::datagram_field_order_is_from_then_bytes` and
`net::gen_ping::tests::ping_result_offsets_match_the_declared_field_order`.

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
