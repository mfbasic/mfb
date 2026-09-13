# bug-599: a `List OF net::Address` is never freed, in any position

Last updated: 2026-09-12
Effort: large — the list's own drop needs a design decision (see "Decision needed"); two
helper-internal leaks in the same shapes are fixed and **landed on main in `c70c6d5d8`**
Severity: MEDIUM — unbounded growth in any loop that resolves a host or asks a socket for an
address, successful or not
Class: Memory

Status: **Open — DECIDED 2026-09-12: flatten** (candidate 2 below). The owner chose to
move `net::Address`, `udp::Datagram` and `audio::AudioDevice` onto the ordinary inline-`String`
record layout and delete the pointer-`String` record exception; the migration is in progress.
Previously: reproduced, attributed, partially fixed, and blocked on that decision.
The branch lands the two leaks that were only ever missing frees (the address builder's
`inet_ntop` buffer and `net::lookup`'s temporary record): −38% growth per `net::lookup`,
−57% per `tcp::localAddress`. What remains is the list, record and host `String` themselves,
which have no owner because the whole value class has no copy-insertion; a drop for it alone
is a double free (proven below). Either fix is a design/ABI decision.
Regression Test: `tests/codegen/codegen_helper_scratch_release.rs` (the free counts of every
address-building helper) and the positive pin
`every_address_reads_back_after_its_builder_scratch_is_freed` in
`tests/runtime/rt_scope_drop_leaks.rs`. No RSS pin yet: no shape of this bug is flat after
the partial fix, and an `#[ignore]`d red pin is not a test.

## How it was found

The bug-593 agent found this while separating variables. It is not bug-593: bug-593's
fix gives an inline `TRAP`'s `Result` wrapper an owner, and this list leaks with **no
`TRAP` at all**. Its note is in `bugs/bug-593-…md` on branch
`bug-593-failing-helper-flat-block` ("`List OF net::Address` bindings — a SEPARATE
defect"). The number was verified free on main, every worktree, and
`git log --all --grep=bug-599`. Defect search: `grep -rliE "net::Address|pointer-string"
bugs/` finds only bug-483's layout doc, which says nothing about drops, and bug-55, which
is `net::lookup`'s `freeaddrinfo` on a fault path.

## The shape, as measured by that agent (macOS, release)

- A **succeeding** `net::lookup("127.0.0.1")` propagated out of a `FUNC`, with no `TRAP`,
  grows **18.0 → 34.8 MB** from 20 000 to 40 000 iterations.
- On the error path, the default empty list `$trap_valN` leaks the same way.
- The emitted code of the `List OF net::Address` probe carries no
  `_mfb_rt_drop_owned_collection` at all.

## Reproduction on this tree (base `8435c09ca`, macOS aarch64, release)

Each probe is a `WHILE i < N` loop; built with `mfb build <dir>`, peak RSS from
`/usr/bin/time -l build/<name>.out`, one run at a time. Scripts: `/tmp/b599/measure.sh`,
`/tmp/b599/run_all.sh` (probe sources in `/tmp/b599/src/`).

| probe (loop body) | base 200k | base 400k | fixed 200k | fixed 400k |
| --- | ---: | ---: | ---: | ---: |
| `LET xs = net::lookup("127.0.0.1")`, `n = n + len(xs)` — bound | 161.7 | 321.9 | 99.8 | 198.3 |
| `n = n + len(net::lookup("127.0.0.1"))` — unbound | 161.7 | 321.9 | 99.9 | 198.3 |
| `LET xs = resolve()` where `resolve` RETURNs the lookup — returned | 161.6 | 321.9 | 99.8 | 198.3 |
| `LET b = tcp::localAddress(server)`, `len(b.host)` — single `Address` | 87.2 | 173.4 | 38.0 | 74.9 |
| `MUT ys AS List OF net::Address = []` + 2× `append(ys, a)`, `a` bound once outside — user-built, no helper in the loop | 196.4 | 391.7 | 196.5 | 391.8 |
| user `FUNC … AS List OF net::Address` that `FAIL`s, bound with `TRAP … CONTINUE` | 126.0 | 251.0 | 126.0 | 251.0 |
| `FOR EACH a IN xs` over a list bound ONCE outside the loop — iterated | 1.2 | 1.2 | — | — |
| `LET a = collections::get(xs, 0)` over a list bound once outside — indexed | 1.2 | 1.2 | — | — |
| **contrast** `MUT ys AS List OF String = []` + 2× `append` | 1.0 | 1.0 | — | — |

(MB. "—" = not re-run; the change does not touch those programs' emitted `main`.)

Separating the variables:

- **Success vs failure:** both grow. The failure row uses a user `FAIL`, not a failing
  resolve, so it isolates the `$trap_valN` default list from the helper.
- **Bound vs unbound vs returned:** identical to the tenth of a MB. The binding shape is
  irrelevant — there is no drop in any of them.
- **User-built vs `net::lookup`-returned:** both grow; the user-built row has no runtime
  helper in the loop at all, so the list block itself leaks.
- **Iterated vs indexed:** reading elements of a live list is flat either way. `get` of an
  element does not allocate for this type (see "Attribution" — it aliases).
- **A copied-out element that outlives the list:** reads back correctly
  (`firstOf` in the positive pin), because nothing is ever freed.
- **A single `net::Address`**, not in a list, grows too: the defect is the value class, not
  the list.

## Attribution

Two separate causes share these shapes.

### 1. Two blocks the address builders orphaned (FIXED on the branch)

- `emit_address_from_sockaddr` (`src/codegen/os/socket/shared.rs`) allocates a 64-byte
  `ADDR_STR_CAP` buffer for `inet_ntop`, copies its bytes into the host `String`, and never
  frees it. No caller reads `dst_off` afterwards — every call site declares it a scratch
  slot and passes it nowhere else (`grep -n DST` over `net/gen_io.rs`, `net/gen_ping.rs`,
  `udp/gen_io.rs`, `tls/gen_macos/address.rs`, `socket/shared.rs`). Reached by
  `net::lookup` (per element), `tcp`/`udp`/`tls` `localAddress`/`remoteAddress`,
  `udp::receive` and both `net::ping` backends.
- `lower_net_lookup_helper` (`src/codegen/builtins/net/gen_io.rs`) copies each 16-byte
  record the builder returned into the list's data region and drops the pointer.

### 2. The value class has no owner (NOT fixed — the rest of the growth)

`net::Address`, `udp::Datagram` and `audio::AudioDevice` are the pointer-`String` records
(`is_pointer_string_record`). Such a record, and anything holding one, is not
`type_is_memcpy_copyable`, so `is_freeable_flat_value` is false and:

- the `Bind` lowering (`owns_freeable_value`, `builder_control.rs`) registers no
  `OwnedValue` cleanup, and `register_pending_temp` no statement free — no drop anywhere;
- `lower_value_owned` makes **no owning copy** of an aliasing source, and
  `materialize_owned_element` makes none for `get` (its copy is gated on
  `is_freeable_flat_value`, or on a cycle for bug-538's class).

Measured in the emitted code (`mfb build -q -ncode`, calls from `_mfb_fn_main`):

| program | `_mfb_arena_alloc` | `_mfb_rt_drop_owned_collection` |
| --- | ---: | ---: |
| `LET xs = net::lookup(…)` in a loop | 0 | 0 |
| `LET xs = net::lookup(…)`, `MUT ys = xs`, `ys = removeAt(ys, 0)` | 0 | 0 |
| same with `LET xs AS List OF String = […]` | 2 | 6 |

## Evaluating the two candidates

### Candidate 1 — a deep drop: unsound on its own (double free)

A drop that only ADDS a free requires every live block to have exactly one owner. For this
class they do not, today, and that is measured rather than inferred:

- `MUT ys = xs` then in-place `ys = removeAt(ys, 0)` prints `ys=0 xs=0` — the two bindings
  are one block;
- `MUT ys = xs` then one in-place `append` prints `xs=96` and then SIGSEGVs — the grow arm
  freed the block `xs` still names.

So a drop at `xs`'s scope and one at `ys`'s would free the same list, and a drop of a list
would free host `String`s still reachable through an element fetched with `get`, through a
list another binding shares, or through a record an `append` copied the host pointer into.
That sharing is filed separately as **bug-601** (it is a live memory-safety defect with no
drop involved). A correct deep drop therefore needs copy-insertion at every owning store
first — bind, assign, return, record/union construction, collection insert/set/literal,
closure capture — which is exactly bug-536 shape C's prerequisite, and the
**USER DECISION of 2026-09-06** moved that to a design plan. The existing per-type deep
COPY would be reusable (`copy_value_to_current_arena` →
`fix_collection_transfer_payload` → `copy_record_fields_into_existing` already deep-copies
an `Address` host for thread transfer); the drop would be its new inverse.

The `TRAP` Ok path adds one more alias (bug-593's `ErrorOnly` wrapper drop exists because
the success binding aliases the wrapper payload), so the drop would also need a single
owner decided there.

### Candidate 2 — flattening the three records: sound, but an internal ABI change

Removing the three names from `is_pointer_string_record` puts them on the ordinary
inlined-`String` layout. They become `type_is_memcpy_copyable`, and the machinery every user
record already relies on — owning copy at a bind, `get` copy, `OwnedValue` drop, flat
`List` drop — applies with no new ownership code. It would also close bug-601 for this class
(not for recursive types). It moves no lifetime rule; it moves a LAYOUT, and every hand
emitter of that layout must move with it. Census (read-only sweep of `src/`, `tests/`,
`packages/`):

- **Writers (13):** `emit_address_host_and_record`, `emit_address_from_sockaddr`,
  `emit_address_from_host_and_port`, `lower_net_address_helper` (`socket/shared.rs`);
  `lower_net_lookup_helper` (a fixed 16-byte payload today — it would become
  variable-width); `lower_ping_posix` + `lower_ping_windows` (store the `Address` pointer in
  `PingResult`); `lower_net_receive_from_helper` (`Datagram`); `lower_tls_address_macos` +
  `lower_tls_listener_address_macos`; `lower_devices` ×3 (macOS, ALSA, Windows).
- **Readers (~12 fns):** `lower_net_endpoint_helper`'s address form (`tcp::connect`),
  `lower_net_send_to_helper` (`udp::send`), `connect_arg_prologue` for three TLS backends
  (OpenSSL, Schannel, Network.framework), both ping backends' `pingAddr` form, and the
  `AudioDevice` id reads in `emit_select_device` / `emit_device_cstring` /
  `emit_widen_device_id`.
- **Knock-on layouts:** once `Address` is flat, `record_field_is_inlined` inlines it into
  `net::PingResult` and `udp::Datagram.from`, so those two records' hand-written layouts move
  too.
- **Predicates:** `is_pointer_string_record`, `record_field_is_inlined`,
  `record_has_inline_data`, `flatness_of_model_type`; the read-only guards
  (`ir::verify`/`ir::shape` `read_only_record_type`) are layout-independent and stay.
- **Tests/goldens:** `pointer_string_record_tests` (3), `tests/net/rt_net_address_*`,
  `rt_tls_listener_local_address`, `codegen_helper_scratch_release`, the ping/udp/audio
  shape tests, and the `byte-identity/{net,tcp,udp,tls,audio,http}` `.ncodesum` goldens on
  all five targets. No MFBASIC package source reads these fields.

Per-platform duplicates (ping ×2, TLS connect ×3, audio devices ×3) mean a large part of it
can only be runtime-proven on Linux and Windows boxes. `audio` has no rt-behavior fixture at
all.

## Decision needed

1. **Flatten** the three helper-built records (candidate 2): closes this bug and bug-601 for
   the class, no new ownership code, a large hand-emitter change across 5 targets.
2. **Wait for the shape-C plan** (candidate 1): fold this class into the recursive
   copy-insertion + deep-drop design, since it needs the same two halves.

Recommendation: 1 — the class is closed (three compiler-owned, read-only records with
bespoke builders), and flattening removes it instead of adding a second copy/drop inverse
pair to keep in lockstep. Not started: it is an ABI change across `net`/`tcp`/`udp`/`tls`/
`audio`, which this ticket was told to bring back rather than make.

## Memory gate — for what landed (the two helper frees)

1. **RSS:** the table above — `net::lookup` 160.2 → 98.5 MB per 200k iterations, single
   `tcp::localAddress` 86.2 → 36.9 MB; the two no-helper controls unchanged to 0.1 MB. Not
   flat, and not claimed to be: the remainder is attribution §2.
2. **Contract:** `mfb spec language memory-semantics` §14 preamble — "Each live value is
   owned by exactly one binding, container slot, temporary, …" — and §14.3.1's native heap
   contract. The `inet_ntop` buffer and the temporary record are neither a value nor
   reachable from one after the copy; they are the helper's own scratch, the bug-574 class
   (`HelperScratch`), and §14.7 does not apply to them because no scope ever sees them.
   **The change only ADDS two frees.** No existing free moves, no returned block is freed,
   and no layout changes: the host `String` and the record the value holds are untouched
   (the lookup free releases the 16-byte block whose words were already copied, not the host
   `String` those words point at).
3. **Positive pin:** `every_address_reads_back_after_its_builder_scratch_is_freed`: a copied-out
   element read after its list is gone and after 500 more lookups, per-element iteration and
   indexing inside the loop, `FOR EACH` output, `tcp::localAddress` stable over 500 calls
   (the port is written after the new free — the sharpest witness of the record-pointer
   reload), `udp::receive`'s nested `from`, and bug-593's two error shapes (trapped code; a
   propagated code with its origin line). The same program's full stdout was identical on the
   base and fixed compilers (`diff` empty, `/tmp/b599/proj/pin_positive_500_{pin,pinfix}`).
4. **Golden deltas:** `bash scripts/regen-ncodesum.sh target/release/mfb` refreshed 144
   sums, of which 25 changed: `byte-identity/{net,tcp,udp,tls,http}` × the five targets.
   Every one emits the changed builder — `http_codegen_cover_rt` calls `_inet_ntop` through
   `tcp::localAddress`; `audio` (the third pointer-string record, a different builder) did not
   move. Zero `.run`, `.ast`, `.ir` or `build.log` goldens changed. Then
   `bash scripts/artifact-gate.sh target/release/mfb all` →
   `1431 tests, 1597 build(s), 2009 golden(s) checked, 0 diff(s)`, exit 0.
   `tests/codegen/codegen_helper_scratch_release.rs` moved by exactly the predicted frees:
   `+1` unguarded free on every `emit_address_from_sockaddr` caller (`tcp`/`udp`/`tls`
   `localAddress`, `tcp::remoteAddress`, `udp::receive`, `net::ping`/`pingAddr`, the OpenSSL
   and Schannel `localAddressListener`), `+2` on `net::lookup`; the Network.framework
   `localAddressListener` (built by `emit_address_from_host_and_port`, no `inet_ntop`) stayed
   `(2, 0, 0)`. No allocation count and no guarded-release count moved. 9/9 passing.
   **What the gate cannot see:** linking and execution. Runtime is covered by the positive pin
   and the `tests/net/rt_net_address_*` / `rt_tls_listener_local_address` suites on macOS
   only; Linux and Windows execution of the new free are not run from this host (the emitted
   sequence is the same target-neutral `abi::` stream on every backend, and cross-target
   `-ncode` builds succeed for all five).

## Interaction with bug-593

bug-593 (branch `bug-593-failing-helper-flat-block`, unmerged) frees the inline-`TRAP`
`Result` wrapper for a non-flat `T`, on the failure tag only for a block payload
(`ErrorOnly`). It does not touch `$trap_valN`'s default empty list, which is this bug's
failure-path row (126.0 → 251.0 MB here, without bug-593). File overlap, measured with
`git diff --stat main...bug-593-failing-helper-flat-block`: bug-593 touches
`engine/builder/mod.rs`, `engine/control/builder_control.rs`,
`engine/value/builder_values.rs`, `cleanup/owned/builder_owned_cleanup.rs`,
`collection/buffer/collection_buffer.rs`, `collections/func_partition.rs`,
`collections/gen_flow.rs`, `.ai/codegen-invariants.md` and
`tests/runtime/rt_scope_drop_leaks.rs`. This branch shares only the last two: a different
paragraph of the `.ai` file (pointer-string records vs the TRAP row), and both APPEND to the
leak test file, so that merge is a concatenation. No source file is shared, and neither
change alters code the other emits.

## Docs vs code

`.ai/codegen-invariants.md` said "Only `Address`, `Datagram`, `DatagramText`,
`AudioDevice` keep pointer strings" and cited `src/target/shared/code/...:586`. The doc was
wrong: `is_pointer_string_record` lists three names (bug-483 removed `DatagramText`, which is
no longer declared) and lives in `src/codegen/collection/layout/`. Corrected on the branch,
with the ownership consequence recorded beside it.
