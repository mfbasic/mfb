# bug-684: presenting a scene that names a group leaks 48 bytes per present

Last updated: 2026-09-23
Effort: small (<1h)
Severity: LOW
Class: Memory-safety

Status: Open
Regression Test: `tests/canvas/rt_canvas_present_leak.rs` — a fourth row, alongside
the three bug-683 added

A program that animates through `canvas::setGroup` — rebuilding a group's items every
frame and presenting a scene that is one `canvas::Group` node naming it — grows by
**48 bytes per present**, forever. Neither half does this alone.

Found while validating bug-683's fix, as the `setGroup` audit its Blast Radius asked
for. It is **not** bug-683's mechanism and is not fixed by it: `canvas::present`'s own
two paths are now exactly flat (measured below), and `canvas::setGroup` on its own is
exactly flat. Only the combination grows.

**The single correct behavior a fix produces:** a program animating by replacing a
group's items and presenting a scene that names it holds steady-state memory, as the
same program animating by presenting the items directly already does.

References:

- `src/codegen/builtins/canvas/gen_group.rs` — `emit_retire_current_items` (`:475`),
  `__canvas_groupReclaim`'s drain (`:770-796`), and the `CANVAS_GROUP_REFS` word.
- `.ai/canvas-threading.md` §13 "The gate runs at the top of `present`, not where the
  scene ring reclaims" — the drain is deliberately unconditional in `__canvas_present`
  precisely so an unchanged scene still frees group memory. That is the first thing to
  re-check: the scene here **is** unchanged every frame, so this is the case that
  design point exists for.
- `bugs/completed/bug-683-*` — the scene ring, fixed; this survives that fix.

## Failing Reproduction

`--debug` build, run headless. `arena.0.live_bytes` is bytes the main arena never got
back, at exit (plan-130-C).

```basic
SUB main()
  app::setMode(app::Mode.Canvas)
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0]])
  MUT frame AS Integer = 0
  WHILE frame < N
    MUT items AS List OF canvas::DrawItem = []
    MUT i AS Integer = 0
    WHILE i < 40
      LET r AS canvas::DrawItem = canvas::Rectangle[x := toFloat(frame * 40 + i), y := 0.0, w := 10.0, h := 10.0, paint := paint]
      items = collections::append(items, r)
      i = i + 1
    END WHILE
    canvas::setGroup("panel", items)
    IF present THEN canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0]])
    os::sleep(16)
    frame = frame + 1
  END WHILE
END SUB
```

| mode | N=120 | N=240 | N=480 | rate |
| --- | --- | --- | --- | --- |
| `setGroup` only, no per-frame present | 52,992 | 52,992 | — | **flat ✓** |
| `setGroup` + present of the Group scene | 58,752 | 64,512 | 76,032 | **48 B/present ✗** |

- Observed: exactly linear, 48 bytes and **one block** per present
  (`alloc_calls - free_calls` grows by exactly 1 per frame: 160 at N=120, 520 at
  N=480).
- Expected: flat, as the `setGroup`-only row is.
- **Identical under `MFB_CANVAS_SYNC=1` and free-running** — so it is not a retirement
  or frame-gate effect. That is the sharpest clue: every schedule-dependent mechanism
  in this area behaves differently under the two, and this does not.

Contrast rows, both measured on the same build, both flat, which is what localizes
this to the group path rather than the scene ring:

| contrast | N=120 | N=240 | N=480 |
| --- | --- | --- | --- |
| present an unchanged 40-item scene (`MFB_CANVAS_SYNC`) | 37,072 | 37,072 | 37,072 |
| present a changing 40-item scene (`MFB_CANVAS_SYNC`) | 53,536 | 53,536 | 53,536 |

| Environment | | Result |
| --- | --- | --- |
| macos-aarch64 | release, software raster, `--debug` | fails ✗ |
| Linux / Windows | not measured | `gen_group.rs` is shared codegen, so expect ✗ everywhere |

## Root Cause

**Not yet established** — this document records a measurement, not a diagnosis, and
the diagnosis is the first task. What is already ruled out:

- **Not the scene ring.** Both `present` paths are flat on their own (the contrast
  rows), and bug-683's fix is what made them so.
- **Not `setGroup`'s retire/reclaim.** `emit_retire_current_items` frees the prior
  retirement unconditionally before overwriting, so it holds at most one buffer per
  slot; the `setGroup`-only row confirms it empirically.
- **Not the retirement frame gate,** in either the scene ring or the group table: the
  rate is identical under `MFB_CANVAS_SYNC` and free-running.

48 bytes and one block is a small, fixed-size allocation. The candidates worth
checking first, in order:

1. The group **name** copy. `setGroup` copies its name into the arena
   (`copy_flat_block(&ParameterType::String, …)`, `gen_group.rs:184`); a `String`
   block for `"panel"` is about this size. The retire/drain path handles
   `CANVAS_GROUP_RETIRED_NAME` separately from the items, and the extra present is
   what changes which of the two drains run.
2. Whatever `present` allocates *per `Group` node* while resolving it —
   `canvas::groupResolve` and the `CANVAS_GROUP_REFS` accounting run on the present
   path and not on the `setGroup`-only path, which is exactly the difference between
   the two rows.

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

- `gen_group.rs` — the likely site; **fixed by this bug**.
- `gen_present.rs` — flat as measured; touch only if the diagnosis lands here.
- Every `Mode.Canvas` program that animates through `setGroup`. At 48 B/frame and
  60 Hz that is ~250 MB a month of continuous running — slow enough that it is a
  correctness defect rather than an outage, and slow enough that nothing would ever
  have noticed it without a byte-exact counter.
- Programs that call `setGroup` once and present a static scene — **unaffected**.

## Phases

### Phase 1 — failing test

- [ ] Add the `setGroup` + Group-scene row to `rt_canvas_present_leak.rs`, and the
      `setGroup`-only contrast beside it as a positive pin.
- [ ] Confirm it fails at 48 B/present and the contrast passes.

Acceptance: one row red for the measured reason, one green.
Commit: —

### Phase 2 — diagnose

- [ ] Identify the leaked 48-byte block. `MFB_CANVAS_STATS`' `groupBytes=` charges the
      item block only, so a block it does not account for is the first place to look.
- [ ] Record the mechanism in Root Cause above, replacing the candidate list.

Acceptance: the block is named, with the allocation site and the path that drops it.
Commit: —

### Phase 3 — fix

- [ ] Free it on the path that owns it, behind whatever gate that path already has.

Acceptance: the Phase 1 row passes; the drain gate is unchanged.
Commit: —

### Phase 4 — validation

- [ ] `cargo test --test 'rt_canvas_*'` green, every golden reference image unchanged.
- [ ] Re-run the reproduction: both rows flat at N=120/240/480.
Commit: —

## Validation Plan

- Regression test: the row from Phase 1.
- Runtime proof: the table above, flat.
- Full suite: `cargo test`, canvas goldens in particular.

## Summary

Small, deterministic and well-bracketed: two contrast rows say it is neither the scene
ring nor `setGroup` alone, and the schedule-independence says it is not a frame gate.
The risk is not in the fix but in over-reaching for it — the group table's retirement
exists to prevent a use-after-free the renderer would hit under load, and 48 bytes is
not worth relaxing it.
