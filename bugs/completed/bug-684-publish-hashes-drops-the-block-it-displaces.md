# bug-684: `canvas::publishHashes` drops the hash block it displaces

Last updated: 2026-09-23
Effort: small (<1h)
Severity: LOW
Class: Memory-safety

Status: FIXED
Regression Test: `tests/canvas/rt_canvas_present_leak.rs` —
`animating_a_group_under_an_unchanged_scene_holds_steady_memory`, with
`..._under_a_changing_scene_...` beside it as the positive pin

`canvas::publishHashes` overwrote the installed per-item hash pointer without retiring
or freeing the block it displaced. On the ordinary path the next `canvas::publishScene`
retired whatever was in the hashes slot, so the block was collected one publish late by
a call that had no idea it was covering for this one. `__canvas_present` has a path
where that never happens — a group whose contents changed under a **byte-identical**
scene makes the frame skip fire and the group signature move, so hashes are published
every frame and a scene is published never — and there the block was lost on every
present.

The leak is one hash block per present, and it scales with the **scene's item count**,
because the hash list holds one entry per item. The title's original "48 bytes a frame"
was the one-item scene the audit happened to measure; a 151-item scene loses
1,248 bytes a present.

Found while validating bug-683's fix, as the `setGroup` audit its Blast Radius asked
for. It is **not** bug-683's mechanism: that was the scene ring, and both of its paths
are exactly flat after it. This is the hashes slot, which bug-683 left with no owner of
its own.

It is also **not** in `gen_group.rs`, which this document's first draft guessed twice.
`canvas::setGroup` is sound and is untouched by the fix; the group is only what puts the
program on the leaking path, by being the thing that changes while the scene does not.

**The single correct behavior a fix produces:** a program animating by replacing a
group's items and presenting a scene that names it holds steady-state memory, as the
same program animating by presenting the items directly already does.

References:

- `src/codegen/builtins/canvas/func_scene_hashes.rs` — `lower_publish_hashes`, the
  store that dropped the pointer.
- `src/codegen/builtins/canvas/func_present.rs` — `__canvas_present`'s
  `IF installed OR moved`, the two independent reasons to call something a frame and the
  reason only one of them publishes a scene.
- `src/codegen/builtins/canvas/gen_present.rs` — bug-683's retirement list, which this
  fix reuses, and `PUBLISH_RETIRES` / `HASHES_RETIRES`, the one-slot-one-owner split.
- `.ai/canvas-threading.md` §3 — why a displaced block is retired rather than freed.
- `bugs/completed/bug-683-*` — the scene ring; this survives that fix.

## Failing Reproduction

`--debug` build, run headless, `MFB_CANVAS_SYNC=1`. `arena.0.live_bytes` is bytes the
main arena never got back, at exit (plan-130-C).

The scene is 150 static rectangles plus one `Group` node, and the group's items are
rebuilt every frame. `VARY` is the single variable: at 0 the `Group` node's `dx` is
constant, so the scene's bytes never change and `publishScene` skips; at 1 the `dx`
moves with the frame, so `publishScene` publishes and its retire reclaims the hashes.

```basic
canvas::setGroup("panel", itemsFor(frame))      ' the group changes every frame
scene = [ ...150 identical rectangles..., canvas::Group[name := "panel", dx := toFloat(frame * VARY), dy := 0.0] ]
canvas::present(scene)
os::sleep(16)
```

| VARY | N=120 | N=240 | rate |
| --- | --- | --- | --- |
| 0 — scene bytes unchanged, `publishScene` skips | 262,704 | 412,464 | **1,248 B/present ✗** |
| 1 — scene bytes change, `publishScene` publishes | — | — | **flat ✓** |

- Observed: exactly linear, one block per present.
- Expected: flat, as the `VARY=1` row is.
- **Identical under `MFB_CANVAS_SYNC=1` and free-running.** That is the clue that it is
  not a retirement or frame-gate effect — every schedule-dependent mechanism in this
  area behaves differently under the two, and this does not.

The audit that found it used a one-item scene and measured the smaller figure:

| mode | N=120 | N=240 | N=480 | rate |
| --- | --- | --- | --- | --- |
| `setGroup` only, no per-frame present | 52,992 | 52,992 | — | flat ✓ |
| `setGroup` + present of a one-item Group scene | 58,752 | 64,512 | 76,032 | 48 B/present ✗ |

Contrast rows on the same build, both flat, which is what ruled out the scene ring:

| contrast | N=120 | N=240 | N=480 |
| --- | --- | --- | --- |
| present an unchanged 40-item scene (`MFB_CANVAS_SYNC`) | 37,072 | 37,072 | 37,072 |
| present a changing 40-item scene (`MFB_CANVAS_SYNC`) | 53,536 | 53,536 | 53,536 |

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64 | release, software raster, `--debug` | fails ✗ |
| Linux / Windows | not measured | `func_scene_hashes.rs` is shared codegen, so expect ✗ everywhere |

## Root Cause

`canvas::publishHashes` (`src/codegen/builtins/canvas/func_scene_hashes.rs`) overwrote
the installed hash pointer and dropped the old one:

```rust
let copy = builder.copy_flat_block(&hash_list_type(), &incoming)?;
builder.emit(abi::store_u64(&copy, &scene, CANVAS_SCENE_HASHES_OFFSET));
```

No retire, no free. That was survivable only by **accident**: on the ordinary path the
*next* `publishScene` retired whatever was in the hashes slot along with the items, so
the block a `publishHashes` displaced was collected one publish late by a call that had
no idea it was cleaning up after it.

`__canvas_present` (`func_present.rs`) has a path where that never happens:

```basic
LET installed AS Boolean = canvas::publishScene(items)
LET sig AS List OF Integer = __canvas_groupSignature(items, 0)
LET moved AS Boolean = NOT __canvas_intListEquals(sig, __CANVAS_LAST_GROUP_SIG)
IF installed OR moved THEN
  canvas::publishHashes(__canvas_hashScene(items))
```

Two independent reasons to call it a frame, and only one of them publishes a scene. A
`Group` node is two floats and a name — all three unchanged by a `setGroup` under the
same name — so a program that rebuilds a group every frame under a byte-identical scene
has `installed` FALSE (the frame skip) and `moved` TRUE (the signature moved). Hashes
are published every frame; a scene is published never; nothing retires anything.

The leak is therefore **one hash block per present**, and it scales with the scene's
*item count* rather than being a fixed 48 bytes — the hash list is one entry per item.
48 B/present was the one-item scene the audit happened to measure; a 151-item scene
leaks 1,248 B/present, measured.

### Why it is retired and not freed

The renderer reads the installed hashes through `canvas::installedHashes` from
`__canvas_sceneDraws` and `__canvas_sceneOffsets`, at arbitrary points inside a frame,
so the block being displaced may be one a render in flight is reading — the same reason
the scene ring retires (`.ai/canvas-threading.md` §3). It goes on bug-683's retirement
list, behind the same unchanged frame gate.

### One slot, one owner

The fix also had to move a responsibility. `publishScene` used to retire all three
slots — items, hashes and layers — but it only *overwrites* items and layers. Leaving
its hashes retirement in place while adding one to `publishHashes` put the same block on
the list twice, because nothing clears the slot between them: the drain then freed it
twice. That is a SIGSEGV on the worker, and it is what the first attempt at this fix
did.

So the rule is now explicit in the code: **the call that overwrites a slot is the call
that retires what was in it.** `publishScene`/`publishLayers` own items and layers;
`publishHashes` owns hashes. Both publish paths always call `publishHashes` when they
publish (`__canvas_present` and `__canvas_presentLayers`), so nothing is left unowned.

### And a second defect in bug-683's own node

`emit_retire_displaced` wrote only the node fields its caller named. With all three
always named that was total coverage by luck; a hashes-only retirement left `items` and
`layers` holding whatever `_mfb_arena_alloc` last had in that block, and the drain reads
all three and frees any that is non-zero — computing a collection size from arbitrary
bytes and handing it to `_mfb_arena_free`. Also a SIGSEGV, also measured. The node now
zeroes every block field before filling the ones being retired.

## Goal

- The `setGroup` + Group-scene row is flat, like the `setGroup`-only row beside it.
- A fourth row in `rt_canvas_present_leak.rs` covers it and fails today.

### Non-goals (must NOT change)

- **The drain gate**, in the scene ring or the group table. A displaced group buffer
  may be one the renderer is mid-copy of (`.ai/canvas-threading.md` §13); freeing it
  earlier trades 48 bytes for a use-after-free.
- **The unconditional `nextReclaimableGroup` at the top of `__canvas_present`.** §13
  is explicit that a memory bound depending on the scene changing is not a bound.
- **What anything draws.** `rt_canvas_golden` and `rt_canvas_rasteriser` must not move
  a pixel.
- Do not "fix" this by making the frame skip publish anyway.

## Blast Radius

- `func_scene_hashes.rs` — `lower_publish_hashes`; **fixed by this bug.** It now drains
  the retirement list and retires the hash block it displaces, with a cold path that
  gives the fresh copy back and raises if the node cannot be allocated.
- `gen_present.rs` — **changed by this bug**, twice. `PUBLISH_RETIRES` loses the hashes
  slot (one slot, one owner), and `emit_retire_displaced` zeroes a node's block fields
  before filling them. Both were latent SIGSEGVs the moment a second caller retired a
  subset of the slots, and both were hit while developing this fix.
- `gen_group.rs` — **audited twice, not the site, untouched.** `canvas::setGroup` alone
  is exactly flat.
- `canvas::presentLayers` — shares `publishHashes` through `__canvas_presentLayers`, so
  it shared the leak and shares the fix. Covered by the existing layers row.
- Every `Mode.Canvas` program that animates a group under a scene whose own bytes do not
  change. The rate is the scene's item count × 8 bytes a present, so a 150-item scene at
  60 Hz is ~4.5 MB a minute — fast enough to matter, and invisible without a byte-exact
  counter because the first version of the regression test tolerated three times it.
- Programs that present a changing scene, or that call `setGroup` once —
  **unaffected**, and pinned by the contrast row.

## Phases

### Phase 1 — failing test

- [x] Add the Group-scene row to `rt_canvas_present_leak.rs`, and the changing-scene
      contrast beside it as a positive pin.
- [x] Confirm it fails and the contrast passes.

The contrast chosen is **not** the `setGroup`-only program this document first proposed.
The sharper one varies the `Group` node's `dx`: same group churn, same hashes publish,
but the scene's own bytes move so `publishScene` publishes and its retire is what
reclaims the hashes. That isolates the single variable — whether a scene publish
happened — instead of removing the present entirely.

Two test-harness corrections were needed before the row could see the bug at all, and
both were mistakes in the file bug-683 added:

- **The growth budget was a flat 512 KB.** Over a 120-present delta that tolerates
  4,369 bytes per present, which swallows this leak whole — both new rows passed against
  the unfixed compiler on the first attempt. The budget is now a fixed allowance plus a
  **per-present** term, which is the shape a per-present leak actually needs.
- **The per-present term cannot be tight for a free-running row.** Those legitimately
  hold every scene published since the last frame tick, and how many that is depends on
  machine load: the layers row measured 0 bytes of growth idle and 1,129 per present
  with five of these in parallel. The budget is now per row — tight for the
  `MFB_CANVAS_SYNC` rows, loose for the two free-running ones, which are catching
  16–41 KB per present and lose nothing to the headroom. The group rows run under
  `MFB_CANVAS_SYNC` for exactly this reason; the leak is the same rate either way.

The row's scene carries 150 static items beside the `Group` node, which is what gives
the leaked hash block its size: 1,248 B/present, against a 64 B/present budget.

Acceptance: met — RED at 1,248 B/present, contrast GREEN on the same unfixed compiler.
Commit: 8d563b133

### Phase 2 — diagnose

- [x] Identify the leaked block: the **hash list**, allocated by
      `canvas::publishHashes` and dropped when `publishScene` skipped.
- [x] Record the mechanism in Root Cause above, replacing the candidate list.

Neither of the two candidates this document guessed was right. The name copy and the
`CANVAS_GROUP_REFS` accounting are both sound; `gen_group.rs` is untouched by the fix.
Two probes settled it: a pure-MFBASIC global-collection reassignment loop (flat at 2,000
iterations, ruling out `__CANVAS_LAST_GROUP_SIG = sig`), and the `dx`-varying contrast
above, which is flat and differs only in whether a scene publish happened.

Acceptance: met.
Commit: 8d563b133

### Phase 3 — fix

- [x] Retire it on the path that owns it, behind the gate that path already has.
- [x] Move the hashes retirement off `publishScene` so each slot has exactly one owner.
- [x] Zero a retirement node's block fields before filling them.

Acceptance: met — the Phase 1 row passes, the contrast stays green, and the frame gate
is byte-for-byte the one bug-683 left.
Commit: 8d563b133

### Phase 4 — validation

- [x] `cargo test --test 'rt_canvas_*'` green, every golden reference image unchanged
      (`rt_canvas_golden` 6/6, `rt_canvas_rasteriser` 60/60 — no pixel moved).
- [x] Re-run the reproduction: both rows flat.
- [x] The four `app_mouse_surface` `.ncode` goldens regenerated; the dump diff is
      confined to `publishHashes` (which grows), the two publish bodies (which lose
      their hashes retire and gain the node zeroing), and the scene layout annotation.

| probe | N=200 | N=400 |
| --- | --- | --- |
| Group scene, unchanged bytes (was 528 B/present at 60 items) | 37,184 | 37,088 |
| Group scene, changing bytes (contrast) | 37,568 | 37,568 |

Commit: 8d563b133

## Validation Plan

- Regression test: the row from Phase 1.
- Runtime proof: the table above, flat.
- Full suite: `cargo test`, canvas goldens in particular.

## Summary

The leak itself was three lines. What made it worth the effort was everything it was
NOT: not the scene ring, not `setGroup`, not a frame gate, and not either of the two
mechanisms this document's first draft guessed. Two contrast probes and one
schedule-independence check are what got from "48 bytes somewhere near groups" to a
named block with a named owner.

The fix's real risk was ownership, not the gate. The moment a second call started
retiring, "retire everything" became "retire it twice", and a partially-filled node
became "free whatever the allocator last left here". Both are SIGSEGVs, both were hit
in development, and both are now stated as rules in the code rather than held by luck.

## STATUS: FIXED (8d563b133)

`canvas::publishHashes` retires what it displaces, onto bug-683's list and behind the
same unchanged frame gate. The Group-scene row is flat where it lost 1,248 bytes a
present, and the changing-scene contrast beside it is flat as it always was.

Two things in bug-683's fresh code had to change with it, and both were latent faults
rather than adjustments: `PUBLISH_RETIRES` dropped the hashes slot so that each slot has
exactly one owner, and `emit_retire_displaced` now zeroes a node's block fields before
filling them. Either one left alone is a crash, not a leak.

### Deviations

- **The regression test's budget was rebuilt**, because as bug-683 left it the test could
  not have caught this: a flat 512 KB allowance over a 120-present delta tolerates 4,369
  bytes per present. It is now a fixed allowance plus a per-present term, and the
  per-present term is **per row** — tight for `MFB_CANVAS_SYNC` rows whose steady state
  is exact, loose for free-running rows whose in-flight retirement pile scales with
  machine load. That second point is measured, not assumed: the layers row reported 0
  bytes of growth idle and 1,129 per present under parallel load.
- **The contrast is a `dx`-varying scene, not a `setGroup`-only program.** It isolates
  the one variable that matters — whether a scene publish happened — instead of removing
  the present.
- **`gen_group.rs` is untouched**, against this document's own first guess.

### What to watch

The one-slot-one-owner rule is now load-bearing and is not checked by anything
mechanical. A third writer of any scene slot — or a fourth field on a retirement node —
has to retire exactly what it overwrites and initialise every field it does not. Both
failure modes are silent at compile time and present as a use-after-free on the worker
one frame later.
