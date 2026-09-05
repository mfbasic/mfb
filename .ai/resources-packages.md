# Resources, packages & builtin authoring

Consolidated reference for the MFB compiler's resource (RES) system, the package/import subsystem, and builtin-package authoring (registration seams, optional params, companion growth, `.mfb` source gotchas, and the data-objects pass).

## The resource (RES) system end to end

Everything about `RES` resources: the ownership model, the record/STATE layout, resource unions, cleanup/free wiring, and thread transfer.

### The RES model: a pointer, not a "borrow"

There is a **SINGLE** resource; a `RES` is a **pointer** to it, owned by the **highest/outermost scope** to touch it — ownership floats UP, never down. A `RES` parameter is the same pointer seen from a deeper scope, not a weaker handle. Never say "borrow" (imports Rust vocabulary that mis-describes this and yields wrong conclusions, e.g. that closing through a `RES` param invalidates something). **close ≠ drop**: `fs::close` releases the OS handle, drop frees the memory, so an early close from a deeper scope conflicts with nothing. When something is rejected, name the real constraint (§15.1: no interprocedural inference → invalidation must be visible at the call site), not "it's a borrow". The spec self-contradicts: §15.6 (`15_resource-management.md:129,135`) is correct scope-ownership; §15.1 (`:13,:20`) + rule `TYPE_RESOURCE_BORROW_INVALIDATE` (2-203-0086, `rules/table.rs:962`) layer borrow vocabulary on top — reading it back uncritically is how the drift recurs.

### Canonical resource-record header

EVERY built-in and package resource has ONE canonical record header (in `src/target/shared/code/error_constants.rs`), so STATE has a free slot in every layout:
- `RESOURCE_OFFSET_TAG = 0` (self-describing `RESOURCE_TAG_*`; `0` = invalid)
- `RESOURCE_OFFSET_HANDLE = 8` (fd / macOS-NW conn ptr / audio `H_KIND` / native `CPtr`)
- `RESOURCE_OFFSET_CLOSED = 16` (moved from 8)
- `RESOURCE_OFFSET_STATE = 24` (generic STATE ptr — free in every backend)
- type-specific tail at 32+; envelope `RESOURCE_RECORD_SIZE = 96` (was 80)

Per-header + per-backend compile-time asserts (`FILE_OFFSET_STATE == RESOURCE_OFFSET_STATE`; `TLS_OFFSET_STATE` in `tls/mod.rs`, `REC_STATE` in `tls/macos/mod.rs`, audio in `audio/mod.rs`) fail the build on drift. This fixed the union-STATE defect (before the canonical header, union STATE at offset 16 clobbered `SSL*` / NW `REC_QUEUE` / schannel block in a `TlsSocket` → SIGSEGV); union STATE now works over ANY variant incl. `TlsSocket` (`tests/rt_macos_d4_union_state_tls.rs`).

**Adding a NEW native backend:** reserve tag@0/handle@8/closed@16/STATE@24, fields at 32+, write tag + zero STATE@24 at construction, add the `== RESOURCE_OFFSET_*` asserts. Do NOT store a per-record CLOSE fn ptr at offset 32 — collides with `FILE_OFFSET_BUF_PTR@32` which drop-reclaim `free()`s. Close dispatch stays compile-time-resolved by the static type; `tag@0` is descriptive, not read at runtime. Declare union variants with BARE ids (no `pkg::Type` normalization).

### Resource-union value layout + STATE wiring

A resource-union VALUE is a `{ tag@0, variant-record-ptr@8 }` block — NOT the record itself. A concrete resource value already IS its record (STATE at `+24`). So every union STATE path first loads the variant record ptr from `+8`, then reads/writes `record+24`. Helper `emit_resource_record_ptr(value_ptr, type_)` (`builder_value_semantics.rs`): union → load `+8`, concrete → identity; used by `.state` read (`lower_field_access`), `emit_resource_state_init`, `StateAssign` (`builder_control.rs`). A `MATCH` extract yields the concrete record (loads `+8`), so the extracted variant's `.state` uses the plain concrete path — but the case binding's type string must carry the STATE suffix (`File STATE Cursor`), set in lowering (`ir/lower.rs::match_case_binding` appends `STATE <T>`).

A stateful union is spelled `Stream STATE Cursor`; its variant close-op wiring lives in THREE bind-type-keyed places that ALL key on the bare union name and must `base_resource_name()`-strip the STATE suffix first (miss one → break):
- `plan/symbols.rs::collect_bind_type_names` DEFINES the close symbols (miss → link error `_mfb_rt_fs_fs_close is not defined`; the doubled `fs_fs` is the existing convention — don't "fix" it).
- `validate/capabilities.rs::collect_bind_types` marks them USED (miss → `NIR declares unused runtime helper`).
- `runtime/usage.rs::push_op_helpers` (via `resource_union_closes`) DECLARES them (miss → `NIR runtime call requires undeclared helper`).

The **plain** (non-union) resource bind has the same three places, and the third of them was missing until bug-535. `plan/symbols.rs::collect_runtime_symbols_from_ops` DEFINES the close symbol, `runtime/usage.rs::push_op_helpers` DECLARES the helper, and `validate/capabilities.rs::collect_owning_resource_bind_types` marks it USED. That last one did not exist, so `RES s AS tcp::Socket = thread::accept(t, 1000)` in a worker with no other `tcp::` call was rejected with `NIR declares unused runtime helper 'tcp'` — the plain arm's close is codegen-emitted at scope exit, exactly like the union arm's, so an NIR-call scan never sees it. It only surfaced once `thread::accept` could hand a resource to a module that calls nothing else in that package; ANY other call into the package hides it. Two things keep the new arm honest: `resource_close_function` peels `STATE` itself (no textual strip needed, unlike the union path, whose lookup is keyed on the union NAME), and the arm carries the DECLARER's aliasing gate (`value_aliases_live_resource` — a bare local or `collections::get`/`getOr`, bug-375). **Used and declared are compared against each other, so the used-side arm must recognize no more shapes than the declarer** — recognizing more turns the fix into its mirror image, `NIR runtime call requires undeclared helper`, on programs that build today. This is deliberately NOT `CodeBuilder::value_aliases_live_resource`, which knows three further shapes.

Also `resource_union_cleanup` (`builder_resource_cleanup.rs`) must strip the base or a stateful union registers NO drop cleanup and leaks. **Resource-union `thread::transfer`/`accept` is UNIMPLEMENTED** (stateless too): transfer-copy dispatch, variant-record deep-copy (the `+8` ptr aliases the sender arena → UAF), `Result`-payload classification (`result_payload_is_block` treats a resource union as a data block; only a *data* union is), and the accept side all have gaps.

### Resource-union param widening ALREADY works for user funcs

Passing a variant into a `RES` param naming its union (`File` → `RES s AS Stream`) already works for user functions — no checker change needed. Both `compatible()`s strip `RES` and subsume variant→union at the param position: `ir/shape.rs` `compatible` (union arm), reached at `check_call_shape`; ir/verify `compat.rs` (union arm ~`:355`, RES stripped ~`:328`), reached at `ir/verify/calls.rs`. Directionality holds automatically: a union actual into a concrete `RES File` is rejected (`compatible("File","Stream")` → false), which keeps every close op / transfer / accept safe WITHOUT a blocklist (symmetric widening would hand a union to a concrete close op — UAF class). What still needs work is the **builtin** path (app::-specific): `ir/shape.rs check_builtin_call`'s term arm (flat `param_types`) + a package `resolve_call` doing context-free `exact()` matching with no type registry — they can't see `app.Button` is a variant of `app.Widget` (the per-overload table + package variant predicate). The §15 spec sentence was more restrictive than the code; amended to permit the directional variant→union widening in non-owning param position.

### STATE mutation is a whole-record rebuild

`s.state.field = v` (and `s.state = v`) desugar (`ast/stmt.rs`) to `Statement::StateAssign` = a whole-state `WITH` over `resource.state`. At codegen (`NirOp::StateAssign`, `builder_control.rs`) that rebuilds the ENTIRE STATE record (`emit_build_inlined_record`) incl. inlined `List`/`String` payloads → any STATE mutation is O(payload), accumulating into a STATE buffer is O(n²). Two halves:
- **Scalar field** in-place store is a pure codegen pattern-match (`try_inplace_state_scalar_assign`): a `NirValue::WithUpdate` whose target is exactly `MemberAccess{Local(resource),"state"}` and every updated field is a plain inline scalar → store new values at `8*field_index` in the existing STATE block. No IR/op change, no golden churn.
- **Collection field** in-place grow IS a pattern-match after all — `try_inplace_state_collection_append` (bug-430). This paragraph used to say it was not, on the reasoning that inlined and growable are mutually exclusive and the field would have to become an out-of-line pointer. The way out was to grow the **whole STATE block** instead of the field: require the collection to be the record's *last inlined* field (`record_collection_last_inlined`), so extending its trailing sub-block extends the block's tail without shifting a sibling or the offsets stored into it. On a realloc the new STATE pointer is republished through `RESOURCE_OFFSET_STATE` so the owner and every alias observe it (§15). No layout change was needed. Since plan-121-A the destination resolution is shared with the plain-local and record-field arms — see `src/codegen/collection/assign/inplace_dest.rs` and `planning/plan-121-gate-inventory.md`.

Regression signal (build-only `--ncode`, `linux-x86_64`, `tests/rt_res_state_inplace_mutation.rs`): count `stackSlots` of `type == "state_assign_value"` (one per whole-record replace, zero in-place) and `append_inplace_realloc` labels (present only when a list grows in place).

### Collection element STATE carry (`List OF RES Stream STATE S`)

Adding a `STATE T` clause to a `RES` collection element needs FAR less work than a `strip_prefix("RES ")` census suggests: every consumer that strips only `RES ` and keeps the remainder already CARRIES the STATE (the remainder handling splits ` STATE T` itself): `resolve_type_name`, `list_element`/`map_parts`/`collection_iteration_type` return `Stream STATE S` unchanged; `base_resource_name`/`state_type_name` are composite-safe (refuse to split a ` STATE ` nested inside `List`/`Map`/`Thread`) — mirroring `parse_resource_plane_type`. The ONE real break: the collection item-compare in builtin resolvers (`resolve_append`/`prepend`/`insert`/`set`/`getOr` in `builtins/collections.rs`) — `ir::verify check_builtin_call_args` strips ` STATE T` off resource *arguments* (`resource_base_type`) but the *element* keeps it → mismatch. Fix = compare both sides by `base_resource_name` (`general::element_accepts_item`). BLIND SPOT: this surfaces ONLY as codegen-time `TYPE_CALL_ARGUMENT_MISMATCH` (plain `error:`, no context) during a FULL `mfb build` — `mfb build -ast -ir` exits 0 without it, so a `tests/syntax/*` fixture (which runs only `-ast -ir`) can't protect a collection-builtin type error; use a `tests/rt-behavior/*` fixture.

### Owned-list union drain (a FLOATED `List OF RES <Union> STATE`)

The owned-list drain only fires when the `RES` elements FLOAT — produced in an INNER scope (loop body) and appended to an outer collection (§15.6). A same-scope union list never floats, so never emits the drain (why an earlier bug passed while the browser's dynamically-built `List OF RES http::Stream STATE PendingState` did not) — reproduce by forcing the float with a loop-scoped binding. Four independent codegen layers (2-4 latent for ANY stateful resource union, surfacing once layer 1 lets the program reach later stages):
1. **Owned-list drain** (`builder_owned_cleanup.rs`/`builder_resource_cleanup.rs`): no single close op — drain each node by tag-dispatch on the active variant, then free the uniform STATE. Share `emit_union_tag_dispatch_drop`; node field-0 is the `{tag@0,record-ptr@8}` block. Keep the concrete path byte-identical.
2. **Verifier** (`ir/verify/values.rs:check_result_value_type`): strip `Result OF ` BEFORE base-normalizing, else the annotation reduces to `Stream` but the element keeps `Stream STATE PendingState` and a correct `ResultValue` is rejected asymmetrically (bites only across a package boundary).
3. **Builtin call-site resolver** (`builtins/http.rs` ready/pump/done/finish): match the BASE name — a call-site stream is spelled `Stream STATE PendingState`, so an exact match resolves to `Unknown` (FATAL when the TRAP desugar binds an intermediate `$trap_val` of that type: "no native storage class for 'Unknown'").
4. **Default-value materializer** (`builder_value_semantics.rs:lower_default_value`): a diverging-handler union `RES x = <fallible> TRAP` had no closed default — materialize a closed `{tag@0,record-ptr@8}` (any variant tag; closed → drop is a no-op), STATE init via `emit_resource_record_ptr`.

### Scope-drop frees owned flat values

Two pieces landed together: **copy-insertion** `lower_value_owned` (`builder_values.rs`) deep-copies (`copy_flat_block`) an aliasing source at `LET`/`MUT`/`StoreGlobal`/`Assign`/closure-capture/`RETURN` (aliasing sources = `Local`/`Global`/`Capture`/`MemberAccess`/`UnionExtract`/`Result{Value,Error}` AND static `String` constants); **frees** `ActiveCleanup::OwnedValue` (`emit_owned_value_drop`) at every scope exit for `is_freeable_flat_value` locals (returned named local = move, no copy). Gotchas (each a loud crash, never silent):
1. `LET s = "lit"` / default empty String point into **rodata, not arena** — freeing faults; `value_needs_owning_copy` copies static strings into the arena.
2. Some builtins **returned a borrow into an arg**: `collections::get`/`getOr` (now `materialize_owned_element`), `strings::replace` no-op (now copies).
3. **Thread-boundary results** (`thread::receive`/`waitFor`) are runtime-managed → `value_is_runtime_managed` excludes them (else non-deterministic shutdown SIGSEGV from freeing a received message corrupting the free-list).
4. A binding whose initializer **traps before storing** would free an uninit slot → freeable slots are zero-initialized first + `emit_owned_value_drop` is null-guarded.

Validation: only thread tests crash if wrong; lldb masks the race — use a 20× run loop + macOS crash report (`~/Library/Logs/DiagnosticReports`) + `otool -tvV` at the imageOffset. Only 3 `.ncode` goldens churn; identical runtime output.

### Escape analysis must ride the NIR visitor

A conservative "does this local escape / is it used as a value" analysis over NIR (deciding a binding is dead at scope end and freeable) MUST build on `nir::visit::{NirVisitor, walk_value, walk_op}` (`function_lowering.rs` `collect_address_taken_locals`/`collect_value_used_locals`), NOT a hand match over `NirValue`/`NirOp` — the `walk_*` recursion is exhaustive, so a new NIR variant is a compile error in one place, not a silently-missed escape = use-after-free. A closure/function INVOCATION lowers to `Call { target: name(String), args }` — `target` is NOT a `NirValue`, so an invoke-only binding never appears as a `Local`/`LocalRef`; any real escape (return/store/pass-arg/alias `LET g=f`/capture/address-take) DOES → "name never in `collect_value_used_locals`" == provably non-escaping. A closure value is `FUNC(...)`-typed, N+2 arena blocks (16B {code@0,env@8} + env `captures.len()*8` + one deep-copied block per freeable-flat `Local` capture); free at scope-drop by extending `OwnedValueCleanup` with an optional capture list (reuses the OwnedValue path). Free ONLY `Local` captures — a by-ref (`LocalRef`/`by_ref Capture`) or inline scalar/float is a wild free (leave them; bounded arena-reclaimed leak). Reload block ptrs from spill slots between `arena_free` calls (they clobber caller-saved).

### TRAP desugar hides producers as locals

`RES f AS File = fs::openFile(…) TRAP … END TRAP` does NOT reach codegen as a call-initialized bind; `ir::lower` desugars it to `bind $trap_res0 : Result OF File = callResult …`, `bind $trap_val1 : File = null`, `bind f : File = local $trap_val1` (the PRODUCER, shaped like an alias). So at NIR/IR a producing bind and an aliasing bind can have the identical `Local` initializer — an ownership decision from initializer shape alone is wrong in one direction: treat every `Local` as owning → `RES g = f` alias closes the caller's live handle at callee exit (`7-703-0004`, exit 255); treat every `Local` as aliasing → a leak (invisible to exit codes/stdout). The alias is safe because `$trap_valN` is itself a resource bind in the **same scope** carrying the close obligation — verify by counting close sites (`"op": "bl","target":"_mfb_linker_<lib>_<close>"`) in `--ncode` (`tests/native_resource_scope_drop.rs`).

**That "same scope" safety BREAKS when the RES binding FLOATS** (inline TRAP producer in a loop appended to an outer `List OF RES …`): escape analysis floats `f` but not `$trap_valN` (not a `RES` name), so the temp keeps a `ResOwner::Local` close at the INNER loop scope and closes the shared record every iteration → plain resource `7-703-0004` "already closed"; resource UNION → SIGSEGV reading STATE through the null record-ptr (`ldr x9,[x8,#0x18]`, x8=0 — the browser CSS-download crash). Fix in codegen: on the float bind (`ResOwner::Float`) if the initializer is `local $src`, call `deactivate_resource_cleanup($src)` (ownership moved to the collection, mirrors the RETURN-escape path in `builder_exits.rs`). Escape analysis (`src/escape.rs`) runs on the AST where these temps don't yet exist — the honest home if a shape test ever proves insufficient. Note `collections.get`/`getOr` are calls that yield a *pointer* per §15.6 (`is_resource_element_pointer` in `ir/verify`).

### LINK close thunks never reclaim the record

A `LINK` close thunk (`_mfb_linker_<alias>_<func>`) sets `RESOURCE_CLOSED_BIT` and NOTHING else — unlike the built-in `fs.close` runtime helper it never `arena_free`s. So the scope-exit `ActiveCleanup::Resource` (its `emit_resource_block_reclaim` half) is the ONLY thing that frees the record, even when the program already closed explicitly. This makes a native resource emit one MORE close site than the `File` equivalent — looks redundant, is NOT. Three sites gate on the builtin-only `resource_close_function`: `resource_cleanup_symbol` (`builder_codegen_primitives.rs`, extended for user resources — the fix), `deactivate_moved_resource_arguments` (~`:1512`), the `RETURN` transfer (~`:2364`). Do NOT extend the lookup to the latter two to "close exactly once" — retiring the cleanup drops the reclaim and leaks a record per explicitly-closed resource. Both are already correct: the 2nd close is a defined `ERR_RESOURCE_CLOSED` no-op (closed flag), and `RETURN` is covered by `escaping_value_slot` identity skip. Verified flat over 40k iterations.

### Borrowed-return + return-type-overloaded builtin wiring

A builtin overload that returns a **borrowed** resource ptr aliasing a `List OF RES X` element and/or is **return-type-overloaded** (worked example `tcp::poll(List OF RES tcp::Socket) AS tcp::Socket`) spans:

- **All seven mutating operations now reach a STATE-held collection in place
  (plan-121-D)**, not just `append`. The dispatch is
  `try_inplace_state_collection_assign` (`builder_control.rs`), one arm per
  builtin, and the arms live beside their record-field twins in
  `collection/assign/builder_inplace_assign.rs` because they share the lowerings
  verbatim. Two rules decide everything:

  **1. The route is chosen by whether the operation can REALLOCATE**, not by
  collection kind and not by container (this is plan-121-C's Correction C2,
  which transfers to STATE unchanged):

  | | operations | how it reaches the field |
  |---|---|---|
  | cannot grow | `removeKey`, `removeAt`, Set `remove`, `set` of a **fixed-width** list element | the inlined **sub-block address** (`open_inplace_inlined_subblock`) |
  | can grow | `append`, `add`, Map `set`, `insert`, `prepend` | grow the **STATE block** and repoint it (`InlineGrow`) |

  Handing a growing lowering a sub-block address is not a slow path: it
  `free()`s a pointer into the middle of the live STATE block.

  **2. `O4` is the one thing the record container does not need.** Every STATE
  arm ends in `close_inplace_dest`, which republishes the (possibly moved) block
  through the resource record's `RESOURCE_OFFSET_STATE` slot, because a STATE
  block has a **second holder** — every alias of the handle reads through that
  slot (§15) — where a record local has none. It is a no-op for the other two
  containers by construction, so the arms do not have to know which they are in.

  **Decline conditions.** There are no *cross-thread* ones, and that is measured
  rather than assumed: `thread::transfer` **copies** the resource into the
  receiving arena and **closes the transferring binding**, so at most one thread
  holds a live handle at any instant and no thread can observe another's
  half-grown block. `.ai/canvas-threading.md`'s "arena state is per-thread" is
  *why* the hazard cannot arise, not evidence that it can. The real declines are
  the ordinary gate set: `G14` (a second updated field in the `WITH` — eliding
  the rebuild would silently drop its new value), `G17` (not the last inlined
  field), `G18`/`G12` (not a self-update, or a self-alias), `G24` (`removeAt` of
  a **recursive** element type — it relocates payloads and `get` on a
  pointer-linked element is not an independent copy), and the variable-width
  `list set` row, which belongs to plan-121-F.

  **The trap when changing any of this:** *no `.ncodesum` golden in the tree
  contains a STATE collection update*, so `artifact-gate.sh` reports 0 diffs
  whether these arms fire or not. A green gate is a drift sentinel here, never
  coverage. The instruments that can actually see the path are
  `tests/rt_res_state_inplace_mutation.rs` (codegen inspection, a positive AND a
  decline per operation) and the `p121d-state-*-rt` fixtures.
- **Borrowed-return classification** lives ONLY at the code-lowering layer: `value_aliases_live_resource` (`builder_values.rs`), hardcoded to `collections::get`/`getOr` — generalize with a per-package predicate (`net::returns_borrowed_resource`). A borrowed return registers NO close obligation; `builder_control.rs` gates this behind a resource-typed bind (`resource_cleanup_symbol(type).is_some()`), so classifying on the call name is safe even when a sibling overload returns a non-resource.
- A borrowed return resolves to the BARE type (RES stripped): `general::list_element("List OF RES Socket") == "Socket"`.
- **Return-type-overloaded**: `DefaultResolver::return_type_name`/`call_return_type_name` returns `None`, so package `resolve_call` must supply it — AND `builder_values::lower_runtime_helper_call` computes `result_type` via `call_return_type_name` too, so add an arg-shape branch there or it fails "native runtime call 'X' has no return type". `flags.return_type_overloaded` is INERT (asserted false in `descriptor.rs`); don't set it true.
- **Multi-overload param names**: move the call from `call_param_names` to `call_param_name_overloads` (mirrors `connectTcp`).
- **Overload → distinct lowering symbol**: remap the NIR target in `runtime_target` by arg shape (`net.poll`→`net.pollList`); add a `RuntimeHelperSpec` + register in `catalog.rs` SPECS AND `CODE_LAYER_ONLY_CALLS`; add the `mod.rs` dispatch arm; and **force-emit** the helper body whenever the base call is present (the NIR only names the base call).
- **`List OF RES X` layout** (X a resource → kind-0 entry table): element `i`'s record ptr = `load((coll+HEADER+capacity*ENTRY_SIZE) + entry[i].value_offset)`, `entry[i]=coll+HEADER+i*ENTRY_SIZE`, `value_offset` at `+COLLECTION_ENTRY_OFFSET_VALUE_OFFSET`.
- Transient dynamic buffer in a standalone helper: `emit_alloc` + `branch_link(ARENA_FREE_SYMBOL)`+`internal_branch(symbol, ARENA_FREE_SYMBOL)`; hold cross-call-live scalars in sp-relative locals via `finalize_vreg_body_with_locals(.., FRAME_SIZE)` (arena/libc calls clobber all caller-saved).

### Thread resource plane split

`thread::transfer`/`accept` uses two direction-isolated queues (mirrors data send/receive): `THREAD_OFFSET_RESOURCE_INBOUND_QUEUE` (104, parent→worker), `THREAD_OFFSET_RESOURCE_OUTBOUND_QUEUE` (112, worker→parent); `THREAD_BLOCK_SIZE` 112→120. Before, a shared queue @104 let a worker's own `transfer` be re-read by its own `accept` (cross-talk). Mechanics (`src/target/shared/code/`): `runtime_target` splits `transferResource`→`emitResource` (worker) / `acceptResource`→`readResource` (parent), keyed on `types::is_worker_thread_handle` (the `ThreadHandle { worker }` flag; the name-prefix `is_worker_thread_type` it replaced is deleted); mod.rs companion-symbol expansion (~1282) MUST push `emitResource`/`readResource` when `transferResource`/`acceptResource` is present (like `receive`⟹`read` — miss = "internal relocation target not defined"); `thread_queue_read_helper` uses `ThreadReadMode` (WorkerSelf/ParentBounded/ParentWaitable) — the `x20` current-thread move is valid ONLY for a worker caller, a parent `accept` reads the worker's CB so must NOT clobber x20 (why ParentWaitable exists). Both queues close on worker exit so an indefinite parent `accept(-1)` terminates. LATENT BUG (out of scope): default-`timeoutMs` `transfer`/`accept` reach codegen WITHOUT the trailing timeout arg (no padding in `lower_runtime_helper_call`, unlike `send`/`receive`); tests pass only because the uninit register is reliably ≥0. Padding `=0` breaks existing tests (workers rely on the garbage being negative to block) — real fix pads AND makes worker tests wait explicitly.

### Thread transfer move-flag is success-gated

`copy_resource_to_current_arena` (`builder_arena_transfer.rs`) flags the SOURCE record `moved|closed` right after copying into the dest arena — but on the SEND path that runs BEFORE the enqueue outcome is known. A failed/timed-out `transfer`/`emit` (`ErrTimeout`/`ErrInterrupted`/`ErrResourceClosed`) keeps ownership with the sender, so flagging at copy time tombstones a handle the sender still owns → its `TRAP`/scope cleanup aborts with `ErrResourceClosed`/`ErrResourceMoved` (HARD error) or leaks the fd. **Invariant:** the source flag store is deferred via the transient `suppress_resource_source_flag` flag and re-emitted only on the enqueue `Ok` branch in `emit_thread_send_runtime_helper_call` (`builder_thread_cleanup.rs`) — mirrors the success-gated `deactivate_moved_resource_arguments`. The ACCEPT side and nested union/collection copies keep flagging inline (their source is a transient queue record). Don't "fix" a transfer-failure leak by swallowing the close no-op or force-closing in the send helper (double-closes on the raw/`TRAP` path). A failed bare-resource transfer's orphaned dest copy (one `RESOURCE_RECORD_SIZE` block) is handed to arg-3 so the queue pending-free list reclaims it; a stateful resource's separate STATE block leaks bounded-until-teardown.

## The package / import subsystem

Everything about `IMPORT`ed packages: `.mfp` type/resource serialization, the embedded-builtin `.mfb` source path, and how each feeds validation vs codegen.

### Type-export closure feeds BOTH validation and codegen

`binary_repr::read_package_type_exports` (→ `read_package_type_exports_resolved`) is the single source that populates:
- the front end's type tables — `ir::lower::TypeIndex` / `ir::ImportedTypeDef` and the shape pass's `TypeShape` (its `PACKAGE_INVALID` metadata walk, `validate_package_type`) — and
- native codegen `type_model` (`target/shared/code/validation.rs:444` `from_module_and_packages` → `add_package_type_export`, filling `record_fields` / `union_variant_fields`).

So a reader-side transitive-closure fix reaches both — but they need DIFFERENT things. Validation (`validate_package_metadata_type`) walks a union's variants' FIELD types; it only needs field-referenced user types (missing → `PACKAGE_INVALID: references unknown type`). Native codegen inlines each **data-union variant's record** into the union block, so it also needs the **variant record itself** (`Box`/`ElementNode`) in `record_fields` (missing → codegen `native inlined field size not available for type '<Variant>'`, which validation never catches). So when closing a re-exported type's transitive set, enqueue BOTH: (1) every identifier in each field's rendered `type_` string (filter by owner-pool membership → discards builtins `List`/`String`), AND (2) each union variant NAME. Resolve from the owner's already-resolved export list (itself closed → a deeper re-export is present too); cycle-guard with a `seen` set (`List OF Node` self-ref terminates). Reader-only: no `.mfp` bytes / `.ncode` goldens shift. See `enqueue_referenced_types` in `src/binary_repr/mod.rs`.

Invariant — `TypeTable.foreign_types` is keyed by BARE exported name: the writer's `foreign_types` map (`external_type_metadata`) keys each imported type by its bare name (`Node`), so `TypeTable::type_id` (`binary_repr/sections.rs`) must resolve a **dotted** `dep.Type` name via a bare-name fallback (`rsplit_once('.')` → look up the last segment) and encode it via `foreign_type` interned under the bare name — identical to the unqualified spelling. Without it, a package-qualified imported type in an exported signature (`FUNC f(n AS dep::Type)`, which lowers to the dotted IR name `dep.Type`) misses the map and falls through to the empty kind-1 RECORD placeholder that fails its own read-back with `truncated binary representation`, writing no `.mfp` under a "Building …" line. Composite-type keys use `#`, never `.`, so only a genuine qualified `pkg.Type` reference hits the fallback. The IR still records the dotted name; consumers read the resolved type table / ABI exports, not the IR string.

### Imported native-resource close-op: 4 starved layers, 2 spellings

`ir/binary.rs` drops `native_resources` by contract, so an importer's `IrProject::native_resources` holds only the CURRENT project's `RESOURCE T CLOSE BY` decls — anything deriving resource knowledge from it is inert for an imported package's resources. There are **four** such layers, each failing differently (fix one, the others stay silent):
1. `ir::verify` `resource_closers` → every resource rule skips the type.
2. `code::validation` closer table → no `ActiveCleanup::Resource`, handle **leaks** with no diagnostic.
3. `link_thunk` close-op set → thunk never sets `RESOURCE_CLOSED_BIT`, so once (2) is fixed the drop-path close becomes a **double free**.
4. Re-exported close-op resolution (the two spellings, below).

**Two spellings** a registered close op reaches the importer as:
- `<id>.<package>.<alias>.<op>` — `RESOURCE T CLOSE BY link::op` serializes the internal dotted target, identity-prefixed by `merge_packages`/`prefix_package_symbols`.
- `<package>.<alias-name>` — a re-exported `EXPORT FUNC close AS link::op` serializes the BARE alias; `ir::package::merge_package` qualifies it with the package name but does NOT identity-prefix it.

Storing one makes the other miss silently. `code::resolve_closer_symbol` tries the name, then retries with a leading 16-hex-digit identity segment stripped — match on the RESOLVED SYMBOL, not a name. Also: `build` hands `ir::verify` EMPTY external-signature maps on purpose; don't "fix" by passing them all — verify would then read an imported union as *open* and demand a `CASE ELSE` from an exhaustive match. Verify shape: `libsnd` wraps its closer in a plain `EXPORT FUNC closeSound` (NOT the registered op — double call not rejected); `sqlite3` uses the §5a re-export (rejected). Test against `sqlite3.mfp` (committed under `tests/rt-behavior/native/native-link-import-sqlite-rt/packages/`), not `libsnd` (`examples/audio/packages/` is gitignored).

### Editing an embedded builtin .mfb ripples to EVERY importer's goldens

A builtin package's MFBASIC source (`src/builtins/json_package.mfb`) is embedded via `include_str!` (the `package_source_glue!` macro in `src/builtins/<pkg>.rs`) and inlined into the IR of every project that `IMPORT`s it. So a change to that .mfb — even one param/constant — shifts the `.ir` (and any `.ncode`/byte-identity) golden of EVERY importing fixture, not just the package's own tests. Find them first: `grep -rl '<pkg>_<symbol>\|#<pkg>_<symbol>' tests | grep -E '\.(ir|ncode|nir|ast|hex)$'`. To AVOID the shift, keep the edit **line-neutral** (same total line count → no `"line": N` moves): a statement-for-statement rewrite yields a ~12-line `.ir` diff instead of ~15,455 lines × N fixtures; inline list literals as args (`utf32Decode([cp])`) help hit the count; when you MUST add lines, append new functions at end-of-file and accept the shift. Regenerate with `scripts/sync-goldens.sh <exe> <name-glob...>` (filter-aware, ~4s) and PROVE the delta is only yours: normalize `sed -E 's/"line": [0-9]+/"line": N/g'` on golden vs actual (note ErrorLoc carries source lines as `"value"`, so those shift too). Rebuild after a .mfb edit is ~3s (embedding module + link only), NOT a full compiler rebuild. If sync-goldens is unusable (a foreign `test-accept` running), regen `.ir`/`.ast` by hand: `mfb build -q -ast -ir <fixtureCopy>` + `cp` (host dumps, target-independent); `.ncodesum` still needs per-target `-ncode`+`shasum`.

### Wiring a builtin source companion: 3 augmentation chains

Injecting a `.mfb` source companion for a builtin (csv/net/vector/audio pattern: `augmented_project` pushes `source_file()` when `uses_package`) requires adding `builtins::<pkg>::augmented_project(&augmented)?` to **BOTH** chains — miss one and it fails partially:
1. `src/resolver/mod.rs` (`augment_project`, the build path's pre-monomorph AST chain — every later pass, the shape pass and lowering included, consumes the concrete HIR it produces).
2. `src/resolver/mod.rs` `augment_hir_project` + the package's `augmented_hir_project` — the HIR-domain twin, `#[cfg(test)]` since plan-107-D (it serves the in-process tests that monomorphize a bare project); keep it in step or those tests see an unresolved `Foo[...]`.
(The former source checker's own late-pass chain is gone with the module.)

Constructible source **value records** follow the `vector::` pattern: list them in BOTH `is_builtin_type` AND `builtin_type_fields` (fields must match the source `EXPORT TYPE`) so they construct un/qualified. An arg-type-overloaded member (`play` `String` vs `List OF String`) dispatches via `source_implementation_name(name, arg_types)` in its own `.or_else` in `ir/lower.rs`, like native `implementation_name` — not the 1:1 json/csv/net chain. Internalized names appear in merged IR as `#audio_play`/`#mml_*` (`__x`→`#x`). A SUB companion is fine as a surface target; `RETURN` is forbidden in a SUB (use `EXIT SUB`); editing a `.mfb` needs a `cargo build` (it's `include_str!`d).

## Registry query surface is typed (plan-111-C)

The registry has ONE query per question and it speaks `ParameterType`, not
spellings: `resolve_call_typed`, `call_return_type_typed`, `constant_type_name`
(returns a `ParameterType` despite the name), `general_override_target`. The
string halves (`resolve_call`, `call_return_type`, `argument_types`,
`constant_type`) are deleted. Consequences when adding or debugging a package:

- **`registry::resolve_call` still exists, `#[cfg(test)]`-gated.** It is a
  spelling shim for the ~140 per-package registration assertions that read
  `resolve_call("audio.poll", &s(&[..]), true) == Some("Boolean".to_string())`.
  Do NOT call it from production — a new production caller is the thing the
  ratchet gate (`tests/no_type_strings.rs`) exists to catch.
- **`builtins::call_return_type_name` renders on the way out, and that is
  correct.** After plan-111-G its only production callers are the two in
  `binary_repr/writer.rs` — the `.mfp` ENCODER, where the spelling *is* the wire
  format. It is not a leftover to convert. Anything on the compiler side asks
  `builtins::call_return_type` (typed); a new caller of the name form outside a
  boundary file is what the gate is there to reject.
- **Strict vs lenient matching is asymmetric on purpose.** A **resource**
  parameter demands exact base-resource identity, so a resource UNION does not
  satisfy a concrete resource close-op parameter (`fs::close(<union>)` stays
  rejected — a use-after-free class error). A **value** nominal parameter stays
  coarse, so a variant still widens into its union (`json::stringify(JsonNull)`).
  STATE and the `RES` marker are transparent on either side (bug-427); a scalar
  never satisfies a nominal in strict mode (bug-443). Lenient mode (overload
  dispatch, return inference) is coarse everywhere. Pinned by
  `strict_matching_separates_resource_params_from_value_union_params`.
- **A declared type may SHADOW a builtin spelling** — `TYPE Integer` compiles.
  Any table keyed by a type must build its key with `ParameterType::declared`
  (= `parse`), never `ParameterType::named`, or the record and the scalar land
  in different buckets and the lookup misses. This is not hypothetical: it
  silently dropped `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION`. The
  round-trip property (`parse(k).name() == k`) does NOT catch it — the real
  question is whether the key you BUILD equals the key the LOOKUP passes.

## The LINK C ABI vocabulary is a `ParameterType` variant (plan-113)

A ctype (`CPtr`, `CInt32`, …) is **not** a `String` after the AST. It is
`ParameterType::C(CAbiType)` — a closed 16-variant enum in `src/types.rs` —
and the conversion happens at ONE place, `hir::elaborate_link_block`. Things
that follow, and that a `grep` for `ctype` will not tell you:

- **A `CSTRUCT`-named `ABI` slot stays a `Named`, deliberately.** The 16 are
  closed; the slot namespace is NOT — a slot may name any `CSTRUCT` declared in
  the same `LINK` alias, and six goldens do (`"ctype": "SfFileInfo"`). So
  `slot.ctype.c_abi()` returning `None` means *either* "a CSTRUCT" *or* "a typo",
  and the caller distinguishes them by looking the name up in
  `project.link_cstructs` (`is_cstruct_slot` / `cstruct_of`, which now ask
  `ctype.is_named(&c.name)`). Do not "simplify" a slot ctype to a `CAbiType`.
- **The IR carriers are `ParameterType`, not `CAbiType`, and that is load
  bearing.** `IrAbiSlot.ctype`, `IrCStructField.ctype` and
  `IrLinkFunction.abi_return_ctype` decode from a crafted `.mfp` where the
  spelling is attacker-controlled. `ParameterType::parse` is TOTAL, so an
  unknown spelling decodes to a `Named` and is rejected by the existing
  `NATIVE_ABI_UNKNOWN_CTYPE` check with its own message and location. Making the
  decode fail instead would replace that diagnostic with
  `PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED` — a real behaviour change on the
  package path, and the reason the `AbiDirection` precedent (whose code has no
  downstream validator) does not apply here.
- **The two `is_c_abi_type` reject-lists are 12 and 13 of 16, on purpose.**
  `resolver::is_c_abi_type` omits `CBool`/`CByte`/`CVoid`/`CBuffer`;
  `ir::verify::link`'s local copy omits only `CBool`/`CByte`/`CBuffer` (it
  includes `CVoid`, because a crafted `.mfp` naming it in an MFB signature must
  be rejected). Rewriting either as `t.c_abi().is_some()` compiles fine and
  silently widens `NATIVE_CPTR_ESCAPE`. The negative assertions in
  `is_c_abi_type_recognizes_and_rejects` are the only guard.
- **A C spelling is a legal template-parameter name.** `TYPE Box OF CPtr`
  builds, so `with_vars` / `monomorph`'s two leaf-symbol probes carry a `C` arm
  to keep reclassifying it as a `Var`.
- **`every_known_ctype_lowers` is not redundant with the exhaustive matches.**
  A `match` proves each variant has an arm; that test proves the arm actually
  `lower_link_thunk`s without error, and its `OUT CBuffer` case walks the whole
  buffer-staging path. Its sibling `ctype_list_is_exhaustive` WAS redundant and
  is gone.

## New builtin-package registration seams

Adding a new builtin package (e.g. a resource package like `process`) touches far more than the descriptor. The obvious ones are documented; these are the ones a plan usually MISSES and the compiler/tests catch late:

**Frontend / type resolution**
- `src/codegen/builtins/<pkg>/mod.rs`: a `RegistryPackage::new(...)` assembled from per-member `func_*.rs` and shared `helper_*.rs` chunks, registered by `pub(crate) fn register(r: &mut Registry)`. An OPAQUE resource handle (`Socket`, `AudioInput`, `Process`) lives ONLY here (`add_resource`) — NEVER in a source companion `.mfb` (`EXPORT TYPE` is for value records/enums).
  - **Corrected plan-122-A:** this bullet used to name `src/builtins/<pkg>.rs` and a `BuiltinModule` descriptor. Both are gone — `ls src/builtins` → `No such file or directory` — and the seam moved into `src/codegen/builtins/` on the clean-room registry.
- `crate::codegen::builtins::<pkg>::register(&mut r)` in `registry::build()` (`src/codegen/registry/mod.rs`).
  - **Corrected plan-122-A:** this bullet used to name a `descriptor.rs` REGISTRY plus a `production_registry_holds_migrated_packages` count to bump. There is no `descriptor.rs` anywhere in `src/` (`find src -name descriptor.rs` → no hits) and no count to bump; adding the `register` call is the whole seam.
- `src/codegen/builtins/mod.rs`: `mod <pkg>`, and the name in **`BUILTIN_IMPORTS`** — the single sorted slice `is_builtin_import` tests and the §18 spec sentence is pinned against. Do NOT reintroduce a second copy of that list: it was a `matches!` arm with a hand-written mirror in the test module, the mirror silently lost `tcp` and `udp`, and because a `matches!` cannot be enumerated no test could see it (plan-122-A).
- **`src/resolver/mod.rs` `BUILTIN_TYPES` slice** — a hardcoded list; a bare `AS <Type>` param/return fails `SYMBOL_UNKNOWN_TYPE` without it (the descriptor path `builtins::is_builtin_type` is NOT what the resolver consults).
- `src/docs/spec/language/18_builtin-functions.md` §18 package sentence — pinned by `spec_section_18_package_list_matches_is_builtin_import`.
- **`src/codegen/builtins/mod.rs` `ARGUMENT_CHECKED_PACKAGES`** (`checks_call_arguments`) — the package must be listed or the shared table checker never runs for it (`ir::shape::Walker::check_builtin_call` on the source path, `ir::verify`'s `check_builtin_call_args` on the package path), so invalid calls collapse to `TYPE_UNKNOWN_VALUE` with NO arity/argument diagnostic.

**Resource registration** (`resource.rs` `BUILTIN_RESOURCES`) + its `every_builtin_resource_has_a_close_op` enumeration list. `close_function` is the SCOPE-DROP op — for `process` it's the internal `__drop` (kill+reap), NOT the public `close` (stdin-only), so `close` is not treated as an ownership transfer.

**Runtime-helper family** (only if it has native codegen)
- `runtime/mod.rs`: `RuntimeHelper::<Pkg>` variant + `name()` + `helper_for_call` arm (a post-lowering `is_<pkg>_runtime_call` that ALSO matches the internal `__drop`/synthesized targets, like `is_audio_runtime_call` matches closeInput).
- `runtime/<pkg>_specs.rs` + `runtime/mod.rs` `mod`/`use` + `catalog.rs` `SUPPORTED_HELPER_SPECS` + the family assert list + its `families.len()` count.
- A source overload split by arg shape (e.g. `spawn(args)` vs `spawn(args,cwd,env,envReplace)` → `process.spawnEnv`) goes in `builder_values.rs`'s `runtime_target = match target` block (mirrors `tcp.connect`→`tcp.connectAddr`); the synthesized name must be added to `catalog.rs` `CODE_LAYER_ONLY_CALLS` (else the round-trip test flags it misrouted because `helper_for_call` returns None for it).
- Native lowering dispatch: the big `match spec.call` in `src/target/shared/code/mod.rs` (a `call if call.starts_with("<pkg>.")` arm).
- Per-target libc imports: each target's `plan.rs` `runtime_imports(spec)` match (macOS `_`-prefixed into `libSystem`; the 3 linux arches share `linux_common`).
- Error strings: `data_objects.rs` per-package gate pushes `ERR_*_MESSAGE`s when any of the pkg's calls are used; `error_constants.rs` `_CODE/_MESSAGE/_SYMBOL`; and the `02_error-codes.md` registry row (build.rs generates `errorCode::` from it).

**Record layout**: 96-byte envelope, tag@0 / handle@8 / closed@16 / STATE@24, type-specific tail @32+. Resource tag in `error_constants.rs RESOURCE_TAG_*` + `03_heap-values.md` table. `RESOURCE_RECORD_SIZE_BYTES=96` is the hard cap.

A `Process`/spawn result is a resource → bind with `RES`, not `LET` (`TYPE_RESOURCE_REQUIRES_RES`).

**A pure-source package's WHOLE companion is compiled into every importing binary, called or not.** This is a size decision, not a detail: it is paid by every program that writes `IMPORT <pkg>` even if it never calls a member. Measured by building `IMPORT io` + `IMPORT <pkg>` with no call and diffing against the `IMPORT io` baseline (66,596 bytes on macos-aarch64):

| package | delta over baseline |
|---|---|
| `bits`, `math`, `collections`, `strings`, `os`, `money`, **`term`**, `tcp`, `tls` | **0** — empty or type-declarations-only companion |
| `color` | +165,120 |
| `encoding` | +462,336 |
| `astrings` | +1,254,912 (includes `color`, which it imports since plan-122-E) |

Command: `mfb init /tmp/szp; printf 'IMPORT io\nIMPORT <pkg>\n\nFUNC main AS Integer\n  io::print("hi")\n  RETURN 0\nEND FUNC\n' > /tmp/szp/src/main.mfb; mfb build /tmp/szp; stat -f%z /tmp/szp/build/szp.out`. **Re-measure rather than quoting these** — they move as the packages grow.

**The instrument is COARSE: built `.out` sizes are quantised to 16,512-byte blocks.** Proved by adding 1, 2 and then 20 extra `io::print` statements to the probe and watching the size not move at all, then sweeping to 4000 statements; every delta observed across plan-122 is a multiple of 16,512. So each figure above is `delta / 16512` blocks and carries up to ±16,512 bytes of error. Fine for "is this package's companion big", useless for "did this one member cost anything" — for that, reason about what `RegistryPackage::get_mfb` renders instead. (Record *constants*, for instance, cost an unused importer exactly zero: `get_mfb` emits imports, records, unions, enums, helpers and member bodies, and constants are in none of those — they fold into a constructor at the call site.)

**`term` staying at 0 is load-bearing and is a gate, not an accident.** plan-122-F gave `term::getForeground`/`setForeground` a `color::Color` while keeping `term` free of `add_imports`, so `IMPORT io` + `IMPORT term` still measures exactly 66,596 — identical to `IMPORT io` alone. Had `term` acquired a companion, every TUI binary would have started carrying `color`'s 165,120 bytes.

**The trigger is a non-empty companion, not `add_imports`.** `tcp` declares `add_imports(vec!["net"])` yet costs 0 bytes, because it declares no records/enums/helpers and renders an empty companion. `udp` declares the same import **plus** one record and therefore pays `net`'s full companion. `term` declares records and enums but no `add_imports`, and costs 0. So a **native** (`Body::abi_function`) member can name another package's value type in its signature through a qualified type-id constant — as `tcp::localAddress` returns `net.Address` — **without** dragging that package's companion in. Reach for that when the importer is size-sensitive (every TUI binary, for `term`).

**Writing the native backend (`src/target/shared/code/<pkg>/`)**
- Register operands in a hand-built helper MUST be a numeric virtual register `%v0`,`%v1`,… (`regalloc/mod.rs` decodes a vreg as `strip_prefix("%v")?.parse()` — a NUMBER). A "readable" name like `%vfile` parses to `None`, is treated as a physical register, and `finalize_vreg_body` PANICS via `find_physical_operand` (the zero-physical-register invariant). Allocate names with `Vregs::next()` or hardcode distinct `%vN` (net uses `%v9`..`%v15`).
- The shared allocator DOES spill live vregs across every `bl` (libc call or `_mfb_*`), so a value can live in a vreg across a call (fs `close` holds `file` across the close). Only genuine memory buffers a syscall fills (a `pipe(int[2])` array, a `waitpid` status int) need the explicit `sp`-relative frame reserved by `finalize_vreg_body_with_locals(ins, &[], local_size)`.
- Helper contract: first MFB arg (the record ptr) arrives in `abi::return_register()` (== `x0` == `mfb_return(0)`), 2nd/3rd in `c_arg(1)`/`c_arg(2)`. **This is the NATIVE RUNTIME HELPER convention and it is NOT universal — the `term` core emitters use a different one.** `src/codegen/term/core/term.rs`'s `emit_set_color` reads its MFB arguments from `c_arg(0)`/`c_arg(1)`/`c_arg(2)`, while `emit_get_color` twenty lines away passes the arena allocator's first argument in `abi::return_register()` and its second in `c_arg(1)`. So both registers are in use for "first argument" in one file, for different call kinds. Picking the wrong one compiles and then reads whatever the other path left behind — a wrong value, not a crash. Read the emitter you are editing; do not carry the convention across from this bullet (plan-122-F Phase 1 asked exactly this question before touching an emitter, and the answer was `c_arg(0)`). Result: value in `RESULT_VALUE_REGISTER` (`mfb_return(1)`==x1), tag in `RESULT_TAG_REGISTER` (`mfb_return(0)`==x0), `RESULT_OK_TAG`="0"/`RESULT_ERR_TAG`="1". A libc call: set `c_arg(0..)`, `platform.emit_libc_call("fork", from, imports, ins, rel)`, result in `c_return(0)`; `sign_extend_word` a C `int` return before comparing. `emit_alloc`: size in x0, align in `c_arg(1)`; returns tag in x0, ptr in `mfb_return(1)`. Body ends `abi::label(&done), abi::return_()`, then `finalize_vreg_body*` → `Ok((frame, ins, rel, stack_slots))` (`HelperResult`).
- Overload-split (`spawn(args)` vs 4-arg) → distinct helper name in `builder_values.rs`'s `runtime_target = match target`, and the emit dispatch is the big `match spec.call` in `code/mod.rs` (a `call if call.starts_with("<pkg>.")` arm). `List OF String` element i: entry at `coll+HEADER(40)+i*ENTRY_SIZE(40)`, raw bytes at `dataBase + valueOffset@24` for `valueLength@32` bytes, where `dataBase = coll + HEADER + capacity*ENTRY_SIZE` (bytes are inline, NOT a String header) — so building a C `argv` means a per-element copy+NUL.

## Builtin optional-param Fill padding

Adding an OPTIONAL trailing parameter to a builtin that uses `Implementation::Rewrite(FIXED)` (csv, not a resolver-based package) has two non-obvious wiring requirements — a `DefaultValue::Fill { type_name, expr }` on the `Parameter` alone is NOT enough:

1. **The padding is driven by `builtins::default_argument_padding`, not the descriptor.** IR lowering pads omitted trailing args at `src/ir/lower.rs` by calling `builtins::default_argument_padding(callee, provided)` (the free dispatcher in `src/builtins/mod.rs`), which iterates a HARD-CODED list of per-package `<pkg>::default_argument_padding` fns. If your package isn't in that list, `csv::parse(s)` lowers a 1-arg call to the 4-arg `#csv_parse` → `TYPE_CALL_ARITY_MISMATCH`. Fix: add a `pub(crate) fn <pkg>::default_argument_padding(name, provided) -> &'static [(&str,&str)]` returning the `(type_name, expr)` fills past `provided` (hand coded like regex's, or reuse your `DEFAULT_*` consts), AND add `<pkg>::default_argument_padding(callee, provided)` to the `for pad in [...]` array in `mod.rs`. (`DefaultResolver::default_padding` computes it from the descriptor but returns an owned `Vec`, so it can't feed the `&'static` dispatcher — the dispatcher fns are hand-authored.)

2. **A String `Fill.expr` is the const's RAW value, not parsed source.** lower.rs builds `IrValue::Const { type_: "String", value: expr }` — `value` is used verbatim. So the default delimiter comma is `expr = ","` (Rust `","`, one char), NOT `expr = "\",\""` (which yields the 3-char string `","` and breaks everything). Integer defaults ("0") don't expose this because raw-value and parsed-source coincide for numbers.

Also update the mirrored static tables `call_param_names` (add a `&[...]` per new param) and the `param_names_cover_all_calls` parity test, or the descriptor↔table parity test fails.

## Builtin companion growth churns importer .ir goldens

A builtin package's source companion (`src/builtins/*_package.mfb`) is injected into every project that imports the package, so its function/type IR lands inside each importer's `.ir` (and `.ast`). Therefore **growing the companion invalidates the `.ir`/`.ast` goldens of every fixture that imports that package**, even fixtures whose own source is unchanged.

The trap in a multi-phase feature: you add a companion function in letter B, sync that fixture's goldens, it passes; letter D/E adds MORE companion functions, which silently re-shift the `.ir` of the B-era fixtures you already synced. Per-letter `test-accept` on the just-synced fixture stays green and hides it. Only the FULL acceptance run catches the stale earlier goldens (e.g. 7 `astrings/*.ir` mismatches, all benign companion-growth churn).

Fix: at finalization, re-sync **all** fixtures that import the package (not just the newly-added ones) against the final binary, then run the full acceptance once more. The behavioral goldens (`build.log`/`.run`) are unaffected — only the intermediate-representation goldens churn. This is the "importer-golden shift" the plan predicts; confirm the delta is only `.ir`/`.ast` of importers, never a `build.log`/`.run` behavior change.

## .mfb source-language authoring gotchas

Gotchas that silently mis-parse or mis-type when authoring `.mfb` source (a builtin package like `http_package.mfb`, or any MFBASIC program). Each cost a build-error round-trip:

- **Record literals are POSITIONAL square brackets**: `Type[v1, v2, v3]` (fields in declaration order), NOT curly-brace named `Type{ f: v }`. Empty list field is `[]`. An empty MAP literal needs its type: `Map OF String TO String {}` — a bare `{}` as a call argument is a parse error (so a defaulted map param must be OMITTED by the caller, relying on `default_argument_padding`, not passed as `{}`).
- **Plain-record fields are NOT assignable** (`rec.field = value`). MFBASIC parses `rec.field = value` as a COMPARISON expression, so a scalar field "assignment" silently no-ops (Boolean/Integer are comparable → no error) and a `List`/record field errors with `TYPE_REQUIRES_COMPARABLE`. To build a mutated record: use scalar `MUT` locals and construct positionally at `RETURN Type[...]`, OR `WITH rec { field := value }` for an update. The ONE exception is resource STATE: `s.state.field = value` DOES mutate (alias-write). Assigning to a `MUT` LOCAL (`chunk = tcp::read(...)`) is fine — only record-FIELD writes mis-parse.
- **MATCH CASE bodies are multi-line**, on the line(s) after the `CASE label`. There is NO inline `CASE label : stmt` colon form (that's a parse error) — the plan docs illustrate it but the parser rejects it.
- **A `<call>` bound inside a MATCH CASE mis-types as `Unknown`** at native codegen (`LET x = f()` or `MUT x = f() TRAP …` inside a CASE) — this extends the TRAP-inside-MATCH-CASE Unknown-type problem beyond TRAP to any call-binding. Factor the call into a top-level helper the CASE only CALLS, and assign into a PRE-DECLARED `MUT` (declared outside the MATCH) — `CASE X : r = helper(v)` is fine, `CASE X : LET r = helper(v)` is not.
- **Reserved keywords can't be ENUM members, RECORD FIELDS, or PARAM names** (case-insensitive): `ENUM Align { Stretch, Start, End, Center }` fails because `End` == the `END` keyword. Same for a record field or a function parameter: a field `end AS Integer` blocks member access (`s.end` → "identifier is invalid after `.`"), and `end` can't even be a bare param name; a field `type AS X` fails outright (e.g. `AttrSpan`/`AttrFlag` → renamed `end`→`last`, `type`→`kind`, and the public param `end`→`endIndex`). Also avoid `To`/`Of`/`In`/`Do`/`Then`/`Type`/`End`. Use a distinct identifier. Members are `Name.Member`, MATCH-able. `None` is fine (not a keyword). Only the `.mfb` compiler catches these — invisible from the Rust side.
- **A nested inline `IF c THEN stmt` steals a following `ELSE`.** When an inline one-line `IF c THEN stmt` (no `END IF`) is the last statement inside a block-form `IF`/`ELSE` branch, the parser attaches the outer branch's `ELSE` to the inline `IF`, then reports `MFB_PARSE_EXPECTED_EXPRESSION` / `MFB_PARSE_UNEXPECTED_STATEMENT` at the `ELSE`. Fix: give the nested `IF` a block form with its own `END IF`. (Bare inline `IF ... THEN stmt` is fine as long as no `ELSE` follows in the same branch.) Also: a multi-line function CALL with arguments split across lines does not parse — keep each call on one line.
- **A dependency's PRIVATE (non-EXPORT) type name still collides with a same-named local TYPE in an importer.** The browser `dom` package has an internal `TYPE Frame { tag, attrs, kids }` (never exported); an app that `IMPORT dom` and declares its own `TYPE Frame { node, depth }` fails to build with a confusing `TYPE_UNKNOWN_FIELD: record 'Frame' has no member 'kids'` — the importer's `Frame` resolved against the dependency's merged internal type, not the local one. Merged package type names share a global namespace regardless of `EXPORT`. Rename the local type (`Frame`→`LWalk`) to fix. So: pick distinctive record names in an app, and don't assume a dependency's un-exported types are invisible.
- **Bare vs qualified names**: since plan-110 gave `tcp`, `udp` and `tls` a resource each named `Socket`, a bare `Socket` no longer identifies a type — union variants, `MATCH CASE` patterns, and param/return annotations all spell it package-qualified (`UNION Stream { fs::File tcp::Socket }`, `CASE tcp::Socket(p)`, `AS RES tls::Socket`). Inside a package .mfb, its own types are unqualified (`AS Response`, `AS RES Stream STATE PendingState`).

Descriptor side (Rust, `http.rs`-style shim): a resource-union-STATE function's PARAM type in the descriptor must be the BASE union (`Stream`), not the stateful spelling — the builtin `resolve_call`/`dispatch_resolve` `exact()` path does not subsume the `STATE` suffix the way the user-function compat path does (see the RES system section); the `.mfb` param carries the full `Stream STATE PendingState`. The RETURN keeps the stateful spelling so the bind preserves STATE. A builtin `.mfb` UNION registers in the descriptor `*_TYPES` table as `TypeKind::Opaque` (like `json::Json`). Editing an embedded `.mfb` needs a `touch` of the including `.rs` to force cargo to re-`include_str!` it.

## The data-objects pass is weaker than the code builder

`error: native code string literal '<text>' has no data object while lowering <op>` is never about the literal. It means `data_objects.rs`'s NIR walk did not see a value the **code builder** later folded into existence. The two run the same constant-folding helpers (`static_string_value_with_constants`, `static_type_name_with_types`) over *different* state, and any asymmetry is a build-breaking bug with no rule code, no file, and no line.

Known instances:
- **bug-118** — `WHEN` guards not walked.
- **bug-361A** — `NirMatchPattern::OneOf` (`CASE "B", "C"`) skipped by an `if let`. Fixed with an exhaustive `match` so the next variant is a build error.
- **bug-361B** — the pass started each function with an EMPTY local-type map while the builder records every parameter with its declared type, so `"type=" & typeName(param)` folded to a literal the pass never saw. Fixed by seeding from `function.params`.
- **bug-363** (filed, NOT fixed) — `module_may_emit_float_numeric_error` cannot type a `MemberAccess`, so `3.14159 * c.radius` on a record field emits no `ERR_FLOAT_*` object. Narrow only because the flag is per-module.

**Finding them:** `python3 scripts/check-man-examples.py` compiles all ~1010 man-page example blocks. It uses `target/release/mfb` — a stale release build will report already-fixed failures, so build release first. It wraps a block with no `main` in a synthetic empty `SUB main()`, so reproduce that wrapping by hand when checking a single block.

Fixture note: a `<name>.run` golden may be empty and still load-bearing — its *presence* is what makes `test-accept.sh` build and execute the program (output lands in `build.log`). New fixtures also need seeded empty `.ast`/`.ir` goldens or the harness reports "unexpected actual".

## Record `RES` field (plan-114)

A record field may hold a resource: `TYPE Holder { handle AS RES fs::File }`.
The field is **one 8-byte handle slot** — copying the record copies the pointer
and aliases the same resource, never duplicating it (§15.6). Four things about it
are non-obvious and cost real bugs during plan-114.

**1. The `RES` marker survives only where a type is stored unstripped.**
`typed_list_element_type` and `typed_map_type_parts` **strip** it
(`engine/types/type_utils.rs:345`, `:352`), so a collection payload is a *bare*
resource. `model.record_fields` does **not** strip, so a record field is the only
position where a `ParameterType::Res(_)` reaches a structural walk at the top
level. Consequences:

- `type_is_flat` was `false` for `List OF RES fs.File` all along (the payload is
  bare) — not `true` as one would guess from reading `Res(_)` having no arm.
- The `Res(_)` arm of any type predicate is reachable **only** through a record
  field, so its blast radius is exactly the new feature.

**2. A bare resource nominal is flat in NEITHER sense; `Res(_)` is flat in one.**
`type_is_memcpy_copyable` / `type_is_arena_transferable` (plan-114-B) split
"does a `memcpy` copy this within one thread" from "may this block be relocated
into another thread's arena". `Res(inner)` is `true`/`false`; a **bare** nominal
is `false`/`false`, and making it `true` for the memcpy question **propagates
through `ResultOf`**: `Result OF tcp.Socket` becomes "flat",
`is_freeable_flat_value` claims it, and a `pending_temp` free is emitted for
something that is not a block. Measured as a real `.ncode` diff in
`tests/byte-identity/tcp`, a fixture with no thread in it.

**3. `copy_value_to_current_arena` asks the MEMCPY question, not the arena one.**
Its name is the tell: it copies into the *current* arena, and the `Result` wrap
at `builder_arena_transfer.rs:145` is reached by any `TRAP` in a thread-free
program. Whether the source came from another thread is the *caller's* question,
answered by `collection_payload_needs_transfer_fix` and the thread-send
`size_computable` — the only two consumers of arena-transferability.

**4. The escape analysis needs a type table for records, and `decl_type` is
annotation-only.** `ResOwner::Float(name)` carries a binding name, and the
bug-291 ordering gate consults `decl_type` to ask "can this container own a
resource". `Named("Holder")` cannot answer that, so `ir::lower` threads
`TypeIndex::res_field_record_types()` into `analyze_function_with`. **And
`decl_type` is populated only for an explicit `AS T`** — so
`LET h = Holder[..., f]; RETURN h` (inferred) had no entry, the gate degraded to
`ResOwner::Local`, and the callee closed the handle it had just returned
(`7-703-0004` in the caller). A record *constructor* names its own type, so the
type is recorded from the initializer. A `RES` **parameter** is exempt from the
ordering rule entirely: the caller owns it, this function never produces it, so
there is no production point for the rule's hazard to attach to.

**Design limit worth knowing:** a resource the callee **produces** cannot be
returned inside a record. The ordering rule wants the container declared before
the resource, and a record — unlike an empty `List` — cannot be constructed
before its handle exists. `FUNC wrap(RES f AS fs::File) AS Holder` is the usable
shape. A record with two `RES` fields of *different* resource types is refused:
an owned-list carries one `OwnedListDrop`.
