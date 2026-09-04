# Collections codegen (List/Map/Set) invariants

Invariants and hard-won lessons for the MFB compiler's native collection codegen — memory management, in-place mutation, presizing, optional-param lowering, HOF rewrite economics, and read-only borrows.

## List memory management: headroom + amortized-O(1) append

Collection mutation codegen is rewritten for amortized-O(1) append.

- **In-place MUT append**: `try_inplace_append_assign` (`collection/assign/builder_inplace_assign.rs`) detects `name = collections::append(name, item)` for a single element on a non-`by_ref` owned MUT list local and routes to `lower_list_append_in_place` (`collection/list/list_mutate.rs`): write into the spare slot + bump count/dataLength when there's room, else realloc with geometric headroom. Soundness rests on value semantics + copy-insertion (no live alias) and `FOR EACH` snapshotting count at loop entry (in-place writes only past that count). transform/filter use the same helper on their private accumulator.
- **Headroom**: `emit_write_collection_header_full` sets capacity/dataCapacity > count/dataLength. Growth shape (`emit_geometric_step`): lookup 4→1024 then ×1.5; data 32→64KiB then ×1.5. Literals/splices stay tight.
- **GOTCHA — data base uses capacity, never count**: with headroom the data region is at `header + capacity*ENTRY`. Always use `emit_collection_data_pointer`. Two hand-written runtime helpers (`_mfb_rt_fs_path_join`, `_mfb_rt_sort_string_list` in mod.rs) had count-based bases → read garbage from a grown list; fixed to load COLLECTION_OFFSET_CAPACITY. Any NEW hand-rolled collection reader must do the same. (Note: `_mfb_*` helper calls clobber all caller-saved registers x0-x17 — spill live scratch such as x14/x15 to stack slots.)
- **Shrink-to-fit copies**: `copy_collection_tight` re-tightens every collection value copy (copy_flat_block routes collections to it) so headroom never leaks into a snapshot or across a thread boundary.
- Removal stays eager-repack (no lazy holes) — meets the contract without liveBytes tracking.
- Result: benchmark/append 44ms→5.3ms (4× faster than CPython, ~C -O2). Runtime proof: tests/collection-memory-grow-rt.


## In-place mutation: one seam, one gate inventory (plan-121-A)

`x = OP(x, …)` on a uniquely-owned collection is lowered as a mutation of the
live buffer whenever nothing else can observe that buffer. The recognisers are
the `try_inplace_*` family, dispatched at
`src/codegen/engine/control/builder_control.rs:879-909` (plain local + record
field) and `:1050`/`:1056` (`RES … STATE`), each falling through to the general
copying reassignment when it declines. **Declining is always correct** — in-place
is an implementation strategy the program cannot observe, so an arm that is not
sure must fall through.

The part every arm repeats — resolve the destination slot, prove unique
ownership, run the aliasing gates — lives in
`src/codegen/collection/assign/inplace_dest.rs`:

* `InPlaceDest` — `Direct { slot }` for a plain local (the slot holds the
  collection block pointer; a realloc repoints it) vs
  `Inlined { block_slot, field_index, write_back }` for a record or `STATE`
  field (the collection lives *inside* the owning record's block, so a realloc
  grows the **record** block). `write_back` is `Some` only for `STATE`, whose
  block pointer is shared with the resource record and must be republished
  through `RESOURCE_OFFSET_STATE` after the mutation (§15).
* `InPlaceGate` — the proof obligations, with `admits_with` a pure predicate over
  a borrowed `LiveIterables` view so the decline conditions are unit-testable
  without building a whole `CodeBuilder`.

**Read `planning/plan-121-gate-inventory.md` before adding an arm.** It lists all
23 decline conditions (`G1`–`G23`), the 2 post-lowering assertions that are hard
`Err`s rather than declines, the 4 emission obligations, and — the part that
bites — a footnote justifying every asymmetry between arms, including which
guards are load-bearing only as a *side effect* of the element-type check and so
re-open a hole if that check is widened.

Two rules from it that are easy to get wrong:

* **`O-order-1`: every gate runs before the first `lower_value`.** No arm may
  lower a value and then decline — that emits dead code and leaks a stack slot,
  and because vreg/stack-slot allocation order is observable in the emitted
  bytes, it also breaks byte-identity for every unrelated fixture in the
  function.
* **`FOR EACH` permits an append but not a shift.** A loop snapshots the buffer
  pointer and count at entry. An `append` writes only *beyond* that snapshot, so
  it may proceed (until it reallocs — hence the guard). An `insert`, `removeAt`,
  `prepend` or entry-compacting delete rewrites entries *below* the snapshot,
  which a live iterator can observe, so those must decline whenever any
  `FOR EACH` walks the collection. `append`'s permissive reasoning does not
  transfer.
* **There is a SECOND aliasing surface, and only payload-relocating ops hit it.**
  The `FOR EACH` rule above is about what an *iterator* sees; it is not the whole
  question. Ask also: **what else holds a reference into the bytes this operation
  moves?**

  | op | what it moves |
  |---|---|
  | `append`, bulk `append` | writes only *past* the live data |
  | `insert`, `prepend` | shift the 40-byte *lookup entries*; new payload goes at the data *tail* |
  | `set` (same-size) | overwrites one payload in place |
  | **`removeAt`** | **compacts the data region — relocates surviving payloads** |

  Relocating a payload is safe only while nothing refers into it, and for a
  **recursive** element type something does. `type_participates_in_cycle`
  (`collection/layout/builder_collection_layout.rs`) marks exactly that class:
  such a value is a *pointer-linked graph* that inline copy codegen cannot
  reproduce, so it needs a per-type runtime copy function — and an ordinary
  `collections::get` of one is therefore **not** the independent deep copy a
  `String`, record or nested-list element gets. Read an element, remove one in
  place, and the value you read follows moved bytes.

  `try_inplace_remove_at_assign` declines on that predicate (gate `G24`).
  **Any future arm that relocates existing payloads inherits it** — including a
  length-changing `set`, which shifts the data tail by design. An arm that shifts
  only *entries* does not: `insert` on the same recursive-union shape was measured
  byte-identical to the pre-change compiler. The failure signature is worth
  recognising: every element wrong except the last, because at `count == 1` the
  shift length is zero.
* **A fixed-width list is entry-FREE, and the two are the same predicate.**
  `list_entry_stride` returns 0 for exactly `list_element_is_fixed_width`
  (`collection/layout/builder_collection_layout.rs`), so inside any
  `if let Some(payload) = list_element_is_fixed_width(..)` branch, **every
  `if entry_stride != 0` is dead**. Element `i` is found at `i * payload` by
  arithmetic; there is no 40-byte entry record to maintain, shift or rebuild.

  Two loops "writing the identity mapping over entries `0..count`" lived inside
  such a branch — in `lower_list_splice_in_place` (`prepend`/`insert`) and in the
  out-of-place `lower_list_insert_collection`, which `collections::set` also
  reaches. Each ran `count` iterations per call and wrote **nothing**: 20
  instructions per element whose only effects were a spill slot overwritten three
  times and never read, and an `add_imm x8, x8, 0`. Removing them made
  `list (Fixed) insert` 2.3× faster (0.754 → 0.329 ms) and left `removeAt`, which
  never had one, unchanged.

  **The general lesson: dead work is invisible to every behavioural test, by
  construction.** No fixture, golden or spike could go red, because the loop
  changed no observable value — a spike comparing two programs cannot see waste
  present in both. It is findable only by reading the emitted instruction stream,
  and it stays gone only if a codegen-inspection test asserts the label's
  *absence* (`a_fixed_width_splice_emits_no_identity_entry_loop`), paired with one
  asserting the variable-width entry shift survives so the deletion cannot later
  widen into a miscompile.
* **A collection inlined in a record: the container decides who holds a pointer,
  and that is the whole question (plan-121-C).** A record/`STATE` field's
  collection is *bytes inside the owner's block*, not its own allocation, so the
  seven mutating operations split by whether they can **reallocate** — a line that
  cuts across every other way of grouping them:

  | | operations | how the mutation reaches the field |
  |---|---|---|
  | cannot grow | `removeKey`, `removeAt`, Set `remove`, `set` of a **fixed-width** list element | the inlined **sub-block address** — the plain-local lowering, unchanged |
  | can grow | `add`, `set` on a `Map`, `insert`, `prepend` | grow the **record** block and repoint it (`InlineGrow`) |

  Read the lowering, do not assume: `lower_map_remove_key_in_place` touches its
  slot four times and **every one is a load**, so it can be handed a sub-block
  address and never learns it is not looking at a plain local.
  `lower_map_set_in_place` **stores a fresh block pointer back into that slot**
  and calls `emit_free_pre_grow_buffer` on the old one — given a sub-block address
  that is a `free()` of a pointer **into the middle of a live allocation**. Not a
  slow path: heap corruption.

  For the growing half, `Option<InlineGrow>` redirects the lowering's own realloc
  sites to (1) request `fieldOffset + collectionSize`, (2) treat the allocation as
  the new **record** and copy the prefix `[0, fieldOffset)` verbatim, and (3) free
  the whole old record. The field's offset is read **once, before** the grow — the
  prefix is copied verbatim so the offset survives, while the sub-block *address*
  does not.

  Two further rules that are easy to get wrong:
  - **The whole-record rebuild is elided by the arm returning `true`**, and
    `updates.len() == 1` (`G14`) is what makes that sound. Match a two-field
    `WITH` and the sibling field's new value is silently dropped — a wrong answer,
    not a slow one, so it needs a test asserting the *decline*.
  - **`set` splits by element width, not collection kind.** A fixed-width list
    element is always replaced by one of exactly its own size, so
    `lower_list_set_in_place`'s rebuild branch is *unreachable* (its own comment
    says so) and the sub-block route is sound. A variable-width element makes that
    branch reachable, so the arm must decline.
* **The third container is `RES … STATE`, and it differs from a record field by
  exactly one obligation (plan-121-D).** The reallocation split above transfers
  unchanged — it is a property of the operation, not of who owns the block — so
  the same seven arms use the same two routes. What a STATE block adds is that it
  has a **second holder**: the resource record's `RESOURCE_OFFSET_STATE` slot,
  which every alias of the handle reads through, so a reallocated block must be
  republished (`close_inplace_dest`, obligation `O4`). A record local has no such
  holder, which is why its arms end without one.

  No cross-thread decline is needed, and that is measured: `thread::transfer`
  copies the resource into the receiving arena and closes the transferring
  binding, so two threads never hold live handles to one STATE at once.

  **Do not read a green artifact gate as coverage for this container** — no
  `.ncodesum` fixture contains a STATE collection update at all, so it reports 0
  diffs either way. See `.ai/resources-packages.md` for the full rule and the
  instruments that can see it.
* **A length-changing `set` on a variable-width element shifts inside the block;
  it does NOT rebuild (plan-121-F).** A same-length replacement was always O(1)
  (offsets unchanged, nothing to move). **Any** length change — longer *or*
  shorter — used to take the `removeAt` + `insert` rebuild: three allocations and
  two full copies per call. Measured, that was **O(N^1.6)**, not the O(N) a data
  shift costs; the excess is the arena free-list degradation `benchmark/README.md`
  documents under mixed-size transient churn.

  The path is now: widen or narrow the span where it lies, then fix up every
  entry whose payload sat after it. Three things about it are easy to get wrong:

  - **The two directions are different code.** Widening moves the tail **up into
    itself** and needs a **backward** copy; narrowing moves it down and needs a
    forward one. A forward copy used for widening smears the first tail bytes
    over the region whenever the shift distance is less than the tail length — and
    still looks correct on a 1–2 element list, which is what a small test uses.
  - **The offset fixup has two directions too, and they are not one operation
    with a negated argument.** `emit_offset_compaction_fixup` subtracts;
    `emit_offset_expansion_fixup` adds. Offsets are read back **unsigned**, so
    passing a negative `hole_len` to the subtracting one wraps. Both use `>` not
    `>=`, which is what leaves the written element's own entry alone.
  - **The overflow path must grow GEOMETRICALLY, or the shift never runs.** This
    is the one that hid: with an in-block shift added but the overflow still
    falling back to the rebuild — which produces a **tight** buffer — every
    widening overflowed on its first call, rebuilt tight, and overflowed again.
    The widening cost was **unchanged** (72 → 828 → 11619 → 122465 ns/set over
    N = 50…3200) while narrowing, which cannot overflow, improved ~7×. A test
    exercising only the narrowing case would have shown a real win and hidden
    that half the feature was dead code.

  `emit_grow_list_data_capacity` is deliberately simpler than `append`'s grow:
  because `capacity` is unchanged, the header, the entry table and the live data
  are one **contiguous** prefix, so it is a single verbatim block copy — and the
  data region keeps the same block-relative base ("data base uses capacity, never
  count"), so no entry offset moves.

  With both halves: **37× and 41× faster at N = 3200**, with the same-length path
  flat at ~10 ns throughout as the control.

  **Testing rule this path imposes:** the failure mode of a partial offset fixup
  is a list that reads correctly up to the written index and returns **garbage
  after**. A folded checksum can miss that. `p121f-string-set-readback-rt` reads
  back **every** element and reports the **first** mismatching index. And because
  the old path was *correct* and merely slow, an unchanged runtime result proves
  nothing about which path ran — `tests/codegen_string_set_shift.rs` is what
  distinguishes them.
* **A String-accumulating `reduce` is rewritten into the loop it is sugar for
  (plan-121-G).** `collections::reduce` over a `List OF String` with a
  concatenating reducer was **O(N²)** — the reducer is called N times and each
  call returns a fresh tight `len(acc) + len(x)` string. The identical fold as a
  hand loop is O(N), because `a = a & x` on a `MUT String` local is matched by
  `try_inplace_concat_assign`. At N = 8000 the two spellings were **790× apart
  for the same answer**.

  **The fix could not live in the fold's lowering**, and that is the reusable
  lesson: at `lower_collection_reduce_impl` the reducer is a function **pointer**
  called indirectly, so its body cannot be inspected — and the cost is *inside*
  the reducer, so no way of threading the accumulator removes it. A callee-side
  "append into `acc` in place" is unsound too: a caller does not give up ownership
  of a `String` argument. The only sound move is to stop calling the reducer and
  emit the loop, where the existing concat arm applies unchanged.

  It is a post-pass over the ops `lower_statement` produces (`src/ir/lower.rs`),
  mirroring `hoist_trap_calls` — the expression lowerer returns an `IrValue` and
  has no statement sink.

  **The condition is deliberately narrow, and one part of it is not about the
  reducer at all:** because the fold is hoisted to the front of its statement, it
  is rewritten only when it is the **first effectful node** there. Anything
  evaluated before it must be effect-free or the hoist reorders observable work.
  Note a call's *arguments* evaluate before the call, so a fold inside
  `len(reduce(...))` — the benchmark's own spelling — still qualifies.

  **This is the third time in plan-121 that a correct-looking optimization did
  nothing and only measurement noticed.** The first version of this pass compiled,
  kept all fifteen semantic fixtures green, and never fired: it declined
  `len(reduce(...))` by treating the enclosing `len` as the first effectful node.
  Because the old path was *correct* and merely slow, an unchanged runtime result
  proves nothing — `tests/codegen_reduce_concat_fold.rs` uses the absence of
  `reduce_call_loop` to tell "fired" from "never ran".

  Result: `reduce` tracks the hand loop within 3% (219 µs vs 212 µs at N = 8000),
  both linear — 145×, and 79× for `reduceRight`.
* **When a shift-based op looks slow, compare it against its sibling before
  theorising.** `insert` sat at 10× worse per byte moved than `removeAt`; that gap
  is what exposed the dead loop. Afterwards, two plausible explanations for the
  remainder were both **measured and refuted**: a descending word copy is *not*
  slower than an ascending one on Apple silicon (46.2 vs 45.7 GB/s, and chunking
  it is 2.12× *worse*), and the `sub`-before-`ldr` ordering in
  `emit_block_copy_backward` costs nothing (1.00×). `insert` runs its shift at
  7.0 GB/s against a `-O0` C word loop's 7.36 — it is at the rate of the loop it
  emits, and what is left is that it shifts *while growing* into fresh buffers
  where `removeAt` shifts inside one that only gets hotter.

## In-place map mutation: branch arg order, dead slack, BUCKETS_READY

Facts for native map-mutation lowerings (`src/target/shared/code/map_mutate.rs`), from landing in-place `removeKey` (mapchurn churn 161→22 ms, ~7.3×):

- **`emit_collection_payload_matches_value_branch(key_type, "", coll, key_off, key_len, query_key, ARG7, ARG8)`: ARG7 is the on-MATCH label, ARG8 is on-NO-MATCH.** Getting this backwards silently removes/keeps the WRONG entry (a swap made removeKey delete entry 0 regardless of the key — output looked plausible but was wrong). The authority is `lower_map_remove_key`'s scan: it passes `(…, scan_next, scan_keep)` where scan_next (skip/advance) is the removed-key MATCH path and scan_keep (retain) is NO-MATCH. Byte-identical parity vs the `.mfb` out-of-place path is the only real check.
- **In-place map mutation leaves DEAD DATA SLACK, and that's the accepted pattern — NOT a leak.** `lower_map_set_in_place` already leaves "old value becomes dead slack, tightened on copy" when overwriting a value; an in-place removeKey that compacts only the ENTRY TABLE (shift `[i+1..count)` down one 40-byte slot, COUNT-=1) and leaves the removed key/value bytes as slack is consistent with it. The slack is reachable and reclaimed on the next TIGHT copy (`copy_collection_tight` on bind/return/param) — grow-realloc copies dataLength VERBATIM (keeps slack), tight copy drops it. So no data-tail shift / offset fixup is needed for correctness (though it would reclaim slack eagerly).
- **The map hash index (open addressing, absolute entry indices, `mod.rs:2407-2703`) cannot be repaired incrementally after a delete** — no DELETED sentinel, probe halts on empty. Any in-place delete must set `BUCKETS_READY=0` (header +4) so the next probe rebuilds. A true O(1) tombstone delete would be a structural project (sentinel + probe + bucket_put + live-count + USED-skip in every `0..count` consumer), out of scope.
- Header offsets: BUCKETS_READY +4 (u8), COUNT +8, DATA_LENGTH +24; entry stride 40 (`error_constants.rs:984-1019`). In-place assign hooks live in `builder_inplace_assign.rs`, wired into the `&&`-chain at `builder_control.rs` ~592; keep the `by_ref` + live-FOR-EACH guards (a compaction shift IS observable to a live iterator).
- Entry-table-only compaction (vs a full data-tail rebuild) is what gave removeKey ~7.3× (churn 161→22ms).

## Map copy-then-bulk-insert pays grows: presize instead

`copy_collection_tight` (`builder_collection_layout.rs`) always sizes the copy **tight**: `capacity == count`, `dataCapacity == dataLength`, buckets sized for `count`. So a builtin that copies a map and then bulk-inserts NEW keys (the `merge` shape: `MUT result = a; FOR EACH e IN b: result = set(result, e.key, e.value)`) forces a geometric grow (entry+data realloc + bucket re-reserve) on the first inserted new key — even though the total size is known up front.

Measured cost (`mapchurn iterate`, 1000-entry map + 10 new keys): the tight copy is ~2µs but the 10 inserts add ~5µs (~500ns each) — dominated by the grow/rehash, NOT the inserts themselves. `x = set(x, k, v)` on a from-scratch map (built by repeated `set`) IS cheap/in-place; the cost is specific to inserting into a **copy** of a populated map.

A plain owning copy (`MUT r = a`) MUST stay tight (value semantics — the caller may never insert), so this can't be fixed in `copy_collection_tight`. The fix is builtin-specific: **presize**. Write a `copy_collection_with_capacity(a, extraEntries, extraData)` variant — alloc `HEADER + (count+extraEntries)*ENTRY + (dataLength+extraData) + buckets(count+extraEntries)`, copy a's entries+data verbatim, but store `COLLECTION_OFFSET_CAPACITY := count+extraEntries` and `COLLECTION_OFFSET_DATA_CAPACITY := dataLength+extraData` EXPLICITLY (the tight header write sets `capacity==count`; `emit_write_collection_header` is compile-time-count only). Then the bulk inserts hit no grow. Same principle as the list `sortBy` reserve (`reserve_integer_index_list`) and A1's index buffers.

## Native collection lowering: optional params arrive pre-padded

When adding a native codegen fast-path for a source-generic collection builtin (the `#collections_<name>$<types>` prefix gates in `builder_values.rs`), the NIR `Call.args` already has the **optional/default parameters filled in**, so gate on the FULL arity, not the source-visible one.

Example: `collections::findLastIndex(xs, pred)` (2 source args) lowers to target `#collections_findLastIndex$String` with **3** args — `[xs, pred, endIndex]`, where the padded default `endIndex = -1` appears as `NirValue::Unary { op: "-", operand: Const Integer "1" }` (a unary negation of `1`, NOT a plain `Const "-1"`). Verify with `mfb build <proj> -ir` and grep the emitted `#collections_*` target.

So a native gate written as `args.len() == 2` never fires — use `args.len() == 3` and have the lowering handle the default value generally (evaluate the padded arg, normalize negatives, bounds-check) rather than special-casing the default shape. This mirrors the Fill/`default_argument_padding` side. Also: in a predicate-scan lowering, test the callback's boolean (RESULT_VALUE_REGISTER) BEFORE calling `free_collection_loop_item` — its `bl _mfb_arena_free` clobbers that caller-saved register.

## Native String-HOF rewrites are marginal/capped (except groupBy)

The interpreted `.mfb` collection HOF bodies (`__collections_chunks`/`window`/`zip`/`groupBy` in `src/builtins/collections_package.mfb`) already build their results out of **native** primitives — `__collections_slice` (itself native, `lower_list_slice_range`), `collections::append` (native in-place), `collections::get`. So a native-codegen rewrite of one of these only removes the interpreted per-iteration dispatch overhead; the dominant cost — copying variable-length String bytes — is unchanged.

Measured (aarch64, `--run 10`): native String `chunks` 13.4 → **12.98 ms**, native String `window` 88.66 → **84.53 ms** — correct, non-regressing, but **marginal (~3–5%) and NOT a full row clear** (Python's C-backed slicing shares string refs; mfb value-copies every byte, so `chunks` py=1.69 / `window` py=8.68 are unreachable). So these rows are **structurally capped**, like the transcendental band.

Two hard lessons:
1. **A per-element materialize+append+free rewrite REGRESSES** vs the `.mfb`'s per-chunk native `slice` — the first native `chunks` attempt was 2.3× SLOWER (30.5 ms) despite byte-identical output. Build tight inners with one `slice`-alloc per sub-list (`emit_string_list_slice_block`), not growable reserve+append per element.
2. **Byte-identical output does NOT prove a native path is worthwhile — only the benchmark does.** Always re-measure with the same `--run 10` before claiming a win (cf. the sortBy gotcha where the `.mfb` fallback silently ran with correct output).

**Big exception — groupBy.** The rule above holds only when the `.mfb` body is built on *efficient* native primitives. `__collections_groupBy` is NOT: it inserts into a `Map OF K TO List OF V` with `bucket = get(result,k); append; result = set(result,k,bucket)` per element — each get/set copies the WHOLE bucket (O(bucket)). Native groupBy (Integer key, String value) reusing the fixed-width inline hash table — buckets as top-level lists appended in place, no per-element map copy — took **groupby 162 → 0.366 ms (~445×), COMPLETE**. So before dismissing a HOF as "marginal", check whether its `.mfb` does a per-element whole-container copy (map set/get, whole-record STATE rebuild) — those are the big wins.

Practical upshot: the constant-factor-only rewrites (chunks/window/zip) don't clear their rows; the band-clearers are groupBy-class O(bucket)→O(N) fixes and the E/F/G/H sub-plans (E borrow-element, F memchr, G bounds-check-elim, H vector-inline).

## get read-only borrow: pending-temp trap + MATCH-desugar chain

Making `collections::get` return an aliasing borrow (no copy) for a read-only MATCH scrutinee (dispatch union 160→43 ms, ~3.7×) surfaced two non-obvious traps:

1. **`lower_value` registers EVERY freeable-flat call result as a pending temp** (`builder_values.rs:17` → `register_pending_temp:30`) — a statement-scope `arena_free`. `lower_value_owned` claims it (`claim_pending_temp:70`) to transfer ownership to the binding. If you make a `get` return an ALIAS (skip `materialize_owned_element`'s `copy_flat_block`) and bind it with plain `lower_value` (no claim), the alias stays registered → the statement-scope free `arena_free`s a pointer INTO the container's data region → garbage reads + free-list corruption that surfaces as a LATER "Allocation failed". **Fix: gate `register_pending_temp` to early-return while the borrow flag is set — the alias is not a fresh block.** (This cost the most debugging; the symptom looked like the alias pointer was wrong, but the alias was fine — it was being freed.)
2. **`MATCH e` desugars to `$matchN = e; MATCH $matchN`, and the MATCH reads the scrutinee once as its `value` PLUS once per case via `UnionExtract`** (the variant bindings `n = UnionExtract($matchN)`). So a "used only as a MATCH scrutinee" classifier must (a) follow the copy chain `e → $matchN`, and (b) count `UnionExtract` reads as MATCH-internal, not as escapes. A naive `reads == scrutinee_reads` check sees `$matchN` read 4× (1 scrutinee + 3 UnionExtracts) and rejects it. BOTH `e` and `$matchN` must borrow, or the copy just moves from the `get` to the `$matchN` bind (net zero win).

Soundness gates (a freed/dangling borrow is a UAF into the container): the container `L` must be immutable (bound ≤1, never `Assign`ed, not address-taken — a reassign frees `L`'s block while the borrow points in), and the element must be freeable-flat-**non-String** (a String `get` returns an OWNED fresh block — skipping its cleanup leaks; skipping its copy dangles). Gate the copy-skip AND the cleanup-skip (`owns_freeable_value` exclusion) on the SAME set. Verify with negatives (e RETURNED / container reassigned must fall back to copy) + a churn stress with interleaved allocations — matching output does NOT prove the borrow fired (the copy path is also correct); only the benchmark drop does.

## Collection-binding validity = THREE orthogonal front-end gates (bug-434)

Whether `MUT xs AS <collection>` (no initializer) compiles is decided by three INDEPENDENT checks, each with its own rule code — do not conflate them, and fixing one does not unlock the others:

1. **Defaultability** (`src/ir/verify/resources.rs:is_defaultable`) → `2-203-0060 TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE`. Since bug-434 every `List`/`Set`/`Map` is defaultable **unconditionally** — the default is the empty collection, which materializes no element, so `T`/`K`/`V` defaultability is irrelevant. The three collection arms `return true` ahead of the FUNC/RES/STATE and union/enum arms. Only records recurse (field-by-field, cycle-guarded); a record reachable only through a collection field is defaultable because the collection arm terminates without recursing into the element. Codegen (`lower_default_value` → `lower_empty_collection`) already materialized the empty form this way, so bug-434 was a pure front-end predicate lift.
2. **Resource ownership** → `2-203-0056 TYPE_COLLECTION_OWNERSHIP_VIOLATION`: an *ordinary* collection cannot store resource/thread ownership at all. So `Set OF File` / `List OF File` are rejected here regardless of defaultability — and even `MUT s AS Set OF File = []` fails on this axis, so any claim that "`= []` is already legal" for a resource-element collection is FALSE.
3. **Element comparability** (`src/ir/verify/values.rs:is_comparable_seen`) → `2-203-0061 TYPE_REQUIRES_COMPARABLE`: a `Set` element and a `Map` KEY must be comparable (a union/FUNC/resource is not); a `Map` VALUE has no such constraint.

Upshot for future work: a plan/bug claiming "collection defaultability now makes `Set OF File` / `List OF RES X` valid" is wrong — those stay rejected on the ownership (and, for Set/key, comparability) axes. When a test asserted the *old* defaultability rejection of such a type, after a defaultability change repoint it to the axis that STILL rejects (ownership), don't assert acceptance. The `RES … STATE <collection>` and `FUNC … return STATE <collection>` checks (`ops.rs`, `calls.rs`) share the same `is_defaultable`, so STATE-of-a-collection fell out as valid for free.


## Layout/model tables are keyed by `ParameterType`, not by a spelling (plan-111-C)

`TypeModel`'s nine tables (`records`, `record_fields`, `union_names`,
`union_variants`, `union_variant_tags`, `enums`, `resource_names`,
`resource_closers`' key, `native_resources`) are keyed by `ParameterType`. So are
`ir::shape`'s `types`/`resource_types` and `ir::verify`'s `TypeEnv`. What this
changes for collection-layout work:

- **Build every key with `ParameterType::declared` (= `parse`), never
  `ParameterType::named`.** A declared type may shadow a builtin spelling
  (`TYPE Integer` compiles). `named` makes the record `Integer` and the scalar
  `Integer` two different keys; the string tables merged them, so `named` is a
  silent behavior change, not a refactor. `union_is_data` and
  `inline_collection_payload_size` in `builder_collection_layout.rs` are the
  collection-side sites.
- **The STATE clause is still peeled before a union lookup.** A transferred
  stateful union arrives as `Stateful { base: Stream, state: Cursor }` and the
  union set is keyed on the bare `Stream` (plan-75 gap 3): `without_state()`
  first, then look up.

  **But peel for the RIGHT table, and only that one.** Peeling before a *record*
  lookup is a rule change, not a tidy-up, and plan-111-G6 shipped it as a bug:
  `File STATE Cursor` is absent from the record table, so the member access was
  left unchecked; the bare `File` base IS present — a resource declares inline
  fields — so `.state` got rejected on every stateful resource. When a
  conversion changes a lookup KEY, ask what the OLD key resolved to, not which
  key reads more nicely.
- **Do not re-wrap a type through its own name to "normalize" it.** An
  `other => named(&other.name())` catch-all in a structural match flattens
  `Stateful { base, state }` back into an opaque nominal and destroys the
  structure the arms above it depend on. Both `ir/shape.rs`'s
  `validate_package_type` and its sibling `is_comparable_seen` had this shape;
  the second one silently reported a stateful RESOURCE as **comparable** (so a
  `Set OF <stateful resource>` would have passed the comparability gate above)
  and no test caught it. Add the explicit variant arm ahead of the catch-all.
- **`refined_list_literal_type` builds `ListOf(Box::new(element))`
  structurally** — it no longer `format!("List OF {element}")`s and the caller
  no longer parses the spelling back.

## `global = <op>(global, f(...))`: operand 0 is snapshotted before a later operand's call can reassign it (bug-496)

```
__CANVAS_GLYPH_PINS = collections::append(__CANVAS_GLYPH_PINS, __canvas_glyphEntry(...))
```

resolves the `append`'s list operand against the global *as it was before the call*, and
`__canvas_glyphEntry` reassigns that global (its eviction pass replaces the list). That is
the **defined** semantics — operand 0 is the pre-call value, so the nested write is lost
and the outer store wins — but until bug-496 the implementation read operand 0 through a
pointer into the global's block, which the nested reassignment's `StoreGlobal` had already
freed: the process **stopped**, with the faulting thread simply gone (on the canvas
graphics thread a hang at 0% CPU with the worker parked in `_pthread_cond_wait`, three
threads in `sample <pid>` where there should be four), or `&` silently dropped operand 0's
bytes (`arena_free` keeps the quick-bin link at offset 0, where `byteLength` lives).

`src/codegen/engine/value/operand_snapshot.rs` closes it at the one seam every operand
goes through, `lower_value`: on entry to a multi-operand node (`Call`, `Binary` incl. the
fused `&` chain, literals, `Constructor`, `WithUpdate`) it records each operand that lowers
to a pointer into storage a callee can reassign — a `Global`, a by-ref or address-taken
local, a resource's STATE, or a member/extract of one — when a LATER operand contains a
call that can run user code (a module function, an indirect FUNC-value call, or any call
handed a FUNC value). When that operand is lowered it is `copy_flat_block`ed into a
statement-scope pending temp and the copy is what the op consumes. Matching is by node
address, so a cloned/rewritten operand fails safe to the old behavior, never to a wrong
copy. Two things keep it narrow, and `tests/rt_operand_snapshot.rs` pins both: a plain
local is unreachable (value semantics), so `x = append(x, f())` / `s & f()` on locals
emit no snapshot and the in-place `x = append(x, <pure>)` fast path never sees a `Call`
node at all; and a global followed only by pure native builtins (`GS & toString(n)`)
emits none either. Count `stackSlots` of type `operand_snapshot` in the `.ncode` to tell
"fired" from "never ran". Fixture: `tests/rt-behavior/collections/bug496_operand_snapshot_rt`.

The binding-first spelling still reads more clearly, and is what the canvas code does:

```
LET entry AS Integer = __canvas_glyphEntry(...)
__CANVAS_GLYPH_PINS = collections::append(__CANVAS_GLYPH_PINS, entry)
```

but it is no longer load-bearing. What remains open is bug-487's other half: the
**in-place** `RES … STATE` arms (`try_inplace_state_*`) capture the STATE pointer before
lowering the operand and never pass through a `Call` node's `lower_value`, so
`f.state.xs = append(f.state.xs, sideEffect(f))` with a STATE-growing `sideEffect` is
untouched by this seam.
