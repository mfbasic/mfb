# plan-116-D: `Line` and `Arc` get an explicit endcap

Last updated: 2026-08-31
Effort: medium (1h–2h)
Depends on: plan-116-C

`canvas::Line` and `canvas::Arc` are the two `DrawItem` variants with no interior —
both are drawn entirely from `paint.stroke`/`paint.strokeWidth`. Both currently have
**round** caps, and neither the type nor the spec says so; it is a consequence of
`__canvas_segmentDistance` clamping the projection parameter `t` to `0..1`
(`helper_shapes.rs:94`, whose doc comment states outright *"Projecting onto the
segment and clamping `t` to `0..1` is what gives the round caps"*), and of the arc's
`abs(length(d) - radius) - half` band being cut by the sweep test.

This letter adds a `cap AS CapStyle` field to both records, with `Butt` and `Round`,
so a program can ask for either.

Behavioral outcome: `Line[… cap := CapStyle.Butt]` renders with square-cut ends
exactly at its endpoints, `cap := CapStyle.Round` renders with the semicircular ends
it has today, and the same holds for `Arc`'s two ends — identically on the software,
Metal and Vulkan paths.

References:

- `src/codegen/builtins/canvas/helper_shapes.rs:94` — `__canvas_segmentDistance` and
  its round-cap note.
- `src/codegen/builtins/canvas/helper_shapes.rs:207` — `__canvas_arcInSweep`.
- `src/codegen/builtins/canvas/mod.rs:499` (`Line`), `:571` (`Arc`) — the two records.
- `src/codegen/builtins/canvas/mod.rs:884` — `draw_item_variant_set_is_frozen`.
- `src/docs/spec/app/06_canvas.md` §"Rendering conventions".

## Prerequisites

See plan-116-A §Prerequisites for the three environment gates.

| Must be true | Command | Status |
|---|---|---|
| plan-116-C complete and archived | `ls planning/completed/plan-116-C-*` → one match | **MET** (2026-09-02: exactly one match, `planning/completed/plan-116-C-canvas-transform.md`, archived by `e0ac6a472`. Every C phase acceptance measured — 91 test binaries on mac RELEASE, 3707 `--bin mfb` unit tests on box 2228, artifact-gate 1823 goldens 0 diffs, test-accept 1347, Vulkan 12/12 on both 2228 glibc and 2227 musl, man-census 0 unclassified, man-run-examples canvas 21/21.) |

If plan-116-C is not complete, this letter cannot start, full stop. C is the last
letter to move `HEADER_SLOTS` and `ITEM_BLOCK_SIZE`; D adds one word to each, and
sequencing it after C means the layout is grown once per letter rather than
renegotiated.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command before you continue and again before you stop.

## 1. Goal

- A new exported enum `canvas::CapStyle` with variants `Butt` (the zero) and `Round`.
- A new required field `cap AS CapStyle` on `canvas::Line` and on `canvas::Arc`.
- `Butt` cuts the stroke square at the endpoint; `Round` extends it by a semicircle of
  radius `strokeWidth / 2` — today's behaviour.
- All three renderers agree.

### Non-goals (explicit constraints)

- **`cap` does not go on `Paint`.** It is a per-item field on the two variants that
  have ends, so it cannot be set on a shape that has none. (User decision, recorded
  2026-08-31.)
- **No `Square`/`Projecting` cap.** Two values, as specified. Adding a third later is
  cheap; adding it now is unasked-for surface.
- **No join style.** `Polygon` corners are unaffected; a join is a separate concept
  and no variant currently exposes one.
- **`Round` must be byte-identical to today's output** for both `Line` and `Arc`.
- **No change to any other `DrawItem` variant.**

## 2. Current State

### Where the caps come from

- **`Line`** → `__canvas_segmentHeader` (`helper_geometry.rs:235`) → kind
  `__CANVAS_KIND_SEGMENT` (2) → `__canvas_segmentDistance`. The clamp at
  `helper_shapes.rs:102` (`t = min(max(t, 0), 1)`) is the round cap: past the
  endpoint, the distance becomes the radial distance *to* the endpoint, which is a
  disc. Both GPU shaders have the identical function
  (`mfb_canvas.frag:segmentDistance`, `metal.rs:segmentDistance`).
- **`Arc`** → `__canvas_arcHeader` (`helper_geometry.rs:254`) → kind
  `__CANVAS_GEO_ARC` (3). The sweep test `__canvas_arcInSweep`
  (`helper_shapes.rs:207`) returns a large distance outside the sweep, so the band is
  cut by a **radial line** at each end — which is a *butt* cap in the angular
  direction, but the band's own `abs(length(d) - radius) - half` gives round ends only
  where the sweep does not cut. Read together: an arc today has **radial (butt) ends**,
  not round ones.

**This asymmetry is the finding that shapes the letter**: `Line` is round today and
`Arc` is butt today. So "`Round` is byte-identical to today" is true for `Line` and
**false for `Arc`** — for `Arc`, it is `Butt` that reproduces today's bytes. Both must
be stated in the tests, and the existing fixtures updated accordingly (§Compatibility).

**A second, smaller finding: the arc's radial cut is hard.** Out-of-sweep pixels get
`RETURN 1000000.0` (`helper_draw.rs`, the `__CANVAS_GEO_ARC` arm), so the cut has no
fractional coverage — a pixel is in the sweep or it is not, decided at its centre.
(The doc comment there says the ends "antialias through exactly the same coverage
path"; that is a statement about the *code path*, not about smoothness across the
radial edge.) The `Line` butt cap this letter adds IS antialiased — the `max` with a
signed half-plane distance composes with the coverage rule like any other edge — so
after this letter a butt line end is smooth and a butt arc end is not. That
asymmetry is accepted deliberately: making the arc's cut a signed half-plane too
would move `smiley.png` and every existing arc byte, which this letter's Phase 1
gate forbids. Recorded in Open Decisions as named future work.

### Measured populations

| What | Count | Command |
|---|---|---|
| `canvas::Line[` / `canvas::Arc[` construction sites | **14** | `grep -rn 'canvas::Line\[\|canvas::Arc\[' --include='*.rs' --include='*.mfb' . \| grep -v '/target/' \| grep -v '\.claude/'` → **14** (re-measured 2026-09-02, merged tree). The unqualified form gives 17; the 3 extra are `examples/ai_chat/src/main.mfb:356,366,368`, a **user-defined** `Line` record — see Verified properties. |
| …in `tests/` | 10 | same command, `tests/` rows |
| …in `mfb man` example prose | 2 | `func_stroke.rs:35`, `func_present.rs:50` |
| …in `cli_canvas_package.rs` | 2 | `:57`, `:59` |
| `DrawItem` variants (frozen list) | 8 | `mod.rs:884-908` |

### Verified properties

- **`examples/ai_chat/src/main.mfb`'s `Line` is not `canvas::Line`.** Read
  `:356` — `collections::append(lines, Line[label, headKind])` is positional
  construction of a two-field local record, and the file does not `IMPORT canvas`
  (`grep -n 'IMPORT' examples/ai_chat/src/main.mfb`). It is therefore **not** in this
  letter's blast radius.
- **`draw_item_variant_set_is_frozen` does not constrain this letter.** Read
  `mod.rs:884` — it pins the variant *names and order*, not their fields. Adding a
  field to `Line`/`Arc` leaves that test green. `every_draw_item_variant_carries_a_paint`
  (`:935`) is likewise unaffected.
- **An arc's cap and a line's cap are different geometry.** A line's cap extends
  along the segment direction; an arc's extends along the tangent at the sweep
  endpoint. They cannot share one implementation, and §4.2 treats them separately.

## 3. Design Overview

Three pieces:

1. **The type and the two fields** — registry data in `mod.rs`, plus the 10
   construction sites updated.
2. **The line cap** — one flag that selects whether `t` is clamped to `0..1` (round)
   or the distance is additionally cut by two half-planes at the endpoints (butt).
3. **The arc cap** — the sweep test already cuts radially (butt); `Round` adds a disc
   of radius `half` at each sweep endpoint, unioned with the band.

**Where the correctness risk concentrates:** the existing fixtures. Ten sites must
each name a cap, and naming the *wrong* one silently changes a reference image. The
mitigation is explicit: `Line` sites take `Round` (today's bytes), `Arc` sites take
`Butt` (today's bytes), and Phase 1 asserts every existing golden is unchanged after
the edit.

**Where the design uncertainty concentrates:** nowhere significant. Both caps are
closed-form additions to distance functions that already exist in all three renderers.

**Byte-identity is NOT this letter's gate**, but it applies to a *specific, named
subset*: after Phase 1, every existing golden must be unchanged. **Expected NOT to
diff:** `smiley.png`, `blendmodes.png`, `transforms.png`, and every
`rt_canvas_rasteriser.rs` pixel assertion. **Expected to diff:** `.ncodesum` on every
canvas-emitting target, the two `.spv` blobs, and `mfb man canvas types` output (a new
enum and two new fields).

### Rejected alternatives

- **Default `cap` on `Paint` instead, with per-item override.** Rejected: it is the
  option the user did not choose, and it would let a `Circle` carry a cap.
- **Make `Round` the enum's zero to match `Line`'s current behaviour.** Rejected as
  moot: MFBASIC named construction requires **every** field (`helper_paint_defaults.rs:5`
  — `Paint[fill := c]` is a `TYPE_CONSTRUCTOR_ARITY_MISMATCH`), so there is no
  defaulted `cap` anywhere and the zero value is only a tag number. `Butt` is listed
  first because that is how the feature was specified.
- **Implement butt caps by shortening the segment.** Rejected: it changes the shape's
  endpoints, so a zero-length line would vanish rather than drawing a dot, and the
  bounds would no longer match the item's declared geometry.

## 4. Detailed Design

### 4.1 The type

```
pkg.add_enum(RegistryEnum {
    name: "CapStyle",
    variants: [ Butt  — "Cut square at the endpoint. The zero value."
                Round — "Extend by a half-disc of the stroke's half-width." ],
});
```

`cap AS CapStyle` is inserted on `Line` **after `y2`, before `paint`**, and on `Arc`
**after `endAngle`, before `paint`** — keeping `paint` last, which
`rect_props`/`paint_prop` (`mod.rs:828`, `:863`) already establish as the convention.

Header slot **34** carries the cap (0 or 1); item block takes a free word from the
`ivec4` plan-116-C added. Neither structure grows.

> **Corrected 2026-09-02 (D1).** This said slot **35**, from an estimate of where C
> would leave the header. C landed **34** slots, 0–33
> (`grep -n "^pub(crate) const HEADER_SLOTS" src/codegen/runtime/canvas/mod.rs` → 34,
> and `helper_geometry.rs:53` → `LET __CANVAS_GEO_HEADER AS Integer = 34`), because
> Correction C2 replaced `sqrt(|det M|)` with the gradient norm and so needed no
> per-axis slot. The first free slot is therefore 34, and every slot number below
> shifts down by one: the arc's sweep endpoints go in **35–38** and `HEADER_SLOTS`
> becomes **39**, not 40. `ITEM_BLOCK_SIZE` is **160**
> (`mod.rs:323`), matching what this letter assumed.

### 4.2 The geometry

**Line, `Butt`.** The round-cap distance is `|w - v·t|` with `t` clamped. For a butt
cap, take the *unclamped* projection `t` and cut with two half-planes. **Corrected
2026-09-02 — see D4; the version below is the one that is right.** A butt stroke is the
round *band* intersected with the slab between the two end planes, and the SDF of an
intersection of convex sets is the `max` of their SDFs — so the half-width comes off
**before** the `max`, and the helper returns the finished band distance:

```
d_butt = max( d_round_with_t_clamped - half ,  -t * |v| ,  (t - 1) * |v| )
```

~~`d_butt = max( d_round_with_t_clamped, -t*|v|, (t-1)*|v| )` with the caller
subtracting `half` afterwards~~ — moot: that compares each end plane against the
*half-width* rather than against zero, so the cap does not cut until a pixel is more
than `half` past the endpoint. Measured on a 20 px line ending at `x = 400`, pixel 405
stayed painted: `max(5.52, 5.5) - 10 < 0`.

The two extra terms are the signed distances to the planes through each endpoint
perpendicular to the segment. `max` with them turns the disc at each end into a
square cut exactly at the endpoint. This is exact, uses only `+ - * /` and the
existing `sqrt`, and so preserves the reproducibility `06_canvas.md` requires — and
because both terms are true signed distances, a butt end is **antialiased** by the
ordinary `clamp(0.5 - d, 0, 1)` rather than stair-stepped.

A zero-length butt-capped line is empty — correct, and distinct from a round-capped
zero-length line, which is a dot. Assert both. (With `len2 = 0` there is no direction
for the planes to be perpendicular to, so the helper answers "far outside" explicitly
rather than dividing by zero.)

**Arc, `Round`.** Today's sweep test cuts radially. Round caps union a disc of radius
`half` at each sweep endpoint:

```
pStart = c + radius * (cos a0, sin a0)
pEnd   = c + radius * (cos a1, sin a1)
d_round = min( d_arc_band_after_sweep_test ,
               |p - pStart| - half ,
               |p - pEnd|   - half )
```

The two endpoints are per-shape constants, so they are computed **once per arc** in
the geometry builder and carried in the header — not per pixel. `__canvas_arcHeader`
already computes `sin`/`cos` of both angles for the sweep vectors
(`helper_items.rs:170-178` reads slots 20/21 and calls `__canvas_cos`/`__canvas_sin`
once per arc), so the endpoints are two multiply-adds on top of work already done.

Header slots **35–38** carry `pStartX, pStartY, pEndX, pEndY`. Header becomes **39**
slots; the item block takes one more `ivec4` (16.16), reaching **176** bytes.
(Corrected 2026-09-02 from 36–39 / 40 — see D1 in §4.1.)

The arc's bounds must grow by `half` at the cap ends when `Round` — the existing
`reach = radius + half + 1.0` hull (`helper_geometry.rs:270`) already covers it, since
a cap disc of radius `half` centred on the circle cannot exceed `radius + half`. Verify
rather than assume.

## Compatibility / Format Impact

- **BREAKING: `canvas::Line` and `canvas::Arc` gain a required field.** MFBASIC named
  construction requires every field, so **every existing `canvas::Line[…]` and
  `canvas::Arc[…]` in user code stops compiling** until it names `cap`. This is the
  same class of break the `DrawItem` union's frozen-set comment (`mod.rs:160`) warns
  about for variants, applied to fields. It is deliberate and user-directed; there is
  no defaulting mechanism in the language to soften it.
- **`canvas::CapStyle` is new exported surface** — `mfb man canvas types` grows.
- **No pixel change for any existing scene**, provided the 10 sites are updated per
  §3 (`Line` → `Round`, `Arc` → `Butt`).
- **`HEADER_SLOTS` 34 → 39**, **`ITEM_BLOCK_SIZE` 160 → 176** — internal.
  (Corrected 2026-09-02 from 35 → 40 — see D1 in §4.1.)
- **`.ncodesum` churn**; both `.spv` blobs regenerate.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick in the same commit as the
> work; `- [~]` for partial with a one-line remainder; fill `Commit:` on landing.
> **An unticked box means NOT DONE.**

### Phase 1 — The type and the field, with today's behaviour preserved

The whole breaking change, landed with zero rendering movement — so the field edit and
the geometry work are never in the same failing gate.

- [x] Add `CapStyle` to `mod.rs` via `pkg.add_enum`, `Butt` then `Round`.
- [x] Add `cap AS CapStyle` to the `Line` record (`mod.rs:499`) after `y2`, and to
      `Arc` (`mod.rs:571`) after `endAngle`.
- [x] Re-run the site census (the §2 command), then update **every**
      `canvas::Line[`/`canvas::Arc[` site: `Line` → `cap := CapStyle.Round`, `Arc` →
      `cap := CapStyle.Butt`, per §2's finding that those are each variant's
      *current* behaviour. → **14 sites, not the 11 listed** (see Correction D2):
      12 `Arc` → `Butt` and 2 `Line` → `Round`, in
      `tests/cli_canvas_package.rs` (1+1), `tests/rt_canvas_metal.rs` (1+1),
      `tests/rt_canvas_rasteriser.rs` (3+0), `tests/rt_canvas_golden.rs` (5+0),
      `src/codegen/builtins/canvas/func_stroke.rs` (1+0) and `func_present.rs` (1+0).
      `examples/emoji/src/main.mfb` is **not in the tree** and was not edited.
- [x] Header slot 34 carries the cap; `__canvas_segmentHeader`/`__canvas_arcHeader`
      write it. **Nothing reads it yet.** → `__CANVAS_GEO_CAP = 34`,
      `__CANVAS_GEO_HEADER` 34 → 35, and a `__canvas_capTag` helper so the two
      writers cannot disagree about the encoding. See **D3** for why no *Rust*
      `HEADER_CAP` constant was added in this phase.
- [x] Tests: add a case asserting `mfb man canvas types` lists `CapStyle` with both
      variants. → `the_cap_style_enum_and_both_cap_fields_render` (`src/cli/man.rs`),
      which also asserts both records carry the field **and that `Circle`/`Rectangle`
      do not** — the rejected "put `cap` on `Paint`" alternative would show up there
      as a shape with no ends acquiring one. Plus the layout guard in
      `helper_geometry.rs` gained two assertions pinning the cap slot inside the
      header and past every slot an emitter already reads.

Acceptance: `cargo test --no-fail-fast` green; **every** canvas golden byte-identical
on disk; `scripts/man-run-examples.sh canvas --run` passes (the two man examples now
name a cap). Any pixel movement here means a site got the wrong cap value — fix the
site, do not re-baseline.

**MET.**

- `cargo test --release --no-fail-fast` — **95 test binaries, 0 failures, exit 0.**
- **Every canvas golden byte-identical**: `git status --short tests/golden/canvas/` is
  empty and `rt_canvas_golden` is 8/8, including the two exact-match references and
  the GPU-vs-reference one. This is the box that would have caught a site given the
  wrong cap, and it is the reason the 12 `Arc` sites take `Butt` while the 2 `Line`
  sites take `Round`.
- `scripts/man-run-examples.sh canvas --run` — **21/21 built and ran**.
- `mfb man canvas types` lists `canvas::CapStyle` with both variants and shows `cap`
  on `Line` and `Arc` only.

That run also surfaced a **flaky test in bug-477's
`tests/rt_tls_connect_allow_self_signed.rs`** — unrelated to canvas, and fixed here
rather than left (AGENTS.md). `free_port()` releases its ephemeral port before
`openssl s_server` binds it, so two of the four concurrent cases can be handed the
same number; the loser of the bind has its readiness probe answered by the *winner's*
server and silently reports that case's TLS verdict. Seen in both directions in one
session. A live child is now part of the readiness condition (an exited child lost the
bind → take a new port) with a mutex spanning pick-through-bind. Measured: 2 failures
in 2 loaded runs before, 10/10 green idle before (so it does not reproduce on demand),
3/3 green under 12-way load plus a full green suite after. Recorded in
`.ai/testing-gates.md`.
Commit: 5f7d1ca93, 65a99632e

### Phase 2 — The line cap, all three renderers

- [x] `__canvas_segmentDistance` gains a cap parameter, or a sibling
      `__canvas_segmentDistanceButt`, implementing §4.2's `max` form.
      Prefer a sibling: `__canvas_segmentDistance` is called by the polygon edge walk
      (`helper_draw.rs:100`) where caps are meaningless, and threading a parameter
      through that path adds a per-edge argument for no reason. → sibling, as advised.
      **§4.2's formula as written is wrong** — see **D4**; the sibling takes `half` and
      returns the finished band distance.
- [x] `__canvas_geoDistance` selects on the cap slot for `__CANVAS_KIND_SEGMENT`.
      The cap is read once per item beside the arc's sweep vectors, not per pixel.
- [x] The same sibling in both shaders; `scripts/regen-spirv.sh`. → `frag -> 22580
      bytes`, `vert` byte-identical at 4004 again. The cap travels in the item block's
      last free word (`ITEM_ARC_CAP`, `arc.w`), so `ITEM_BLOCK_SIZE` stays 160.
- [x] Tests: `tests/rt_canvas_rasteriser.rs` — a butt-capped horizontal line (assert
      the pixel one past the endpoint is background and the pixel at the endpoint is
      stroke); the same line round-capped (assert the pixel one past is stroke); a
      zero-length butt line (nothing drawn); a zero-length round line (a dot).
      → `a_butt_cap_stops_at_the_endpoint_and_a_round_cap_does_not` and
      `a_zero_length_line_is_a_dot_only_when_round_capped`, both asserting the two
      styles against the *same* line so neither could pass on a renderer that ignored
      the flag.
- [x] **Added:** a butt- and a round-capped line in both GPU harnesses
      (`tests/rt_canvas_metal.rs`, `scripts/test-canvas-vulkan.sh`). Not in the plan,
      and needed: without a butt-capped item on a GPU scene the new shader branch is
      compiled into both backends and **never executed**, so a wrong butt arm would
      still match the oracle everywhere either harness looks. Deferring that to
      Phase 4's `endcaps.png` would have found it three phases late.

Acceptance: the four new cases pass; the round-capped line renders byte-identically to
the same line at Phase 1's commit.

**MET.**

- The new cases pass; `rt_canvas_rasteriser` is 24 passed / 1 ignored.
- **Byte-identity, measured against Phase 1's actual compiler** rather than inferred
  from the source. No canvas golden contains a `Line`, so this needed its own
  instrument: `git archive 5f7d1ca93 | tar -x -C /tmp/p116d-phase1`, build, and render
  the same six-line scene (horizontal, vertical, diagonal, shallow, 2 px, 44 px, and a
  zero-length dot — 32,098 lit pixels) with both binaries. `cmp` → **byte-identical**.
- Both GPUs agree with the oracle **with both cap styles in the scene**:
  `rt_canvas_metal` 4/4, and `scripts/test-canvas-vulkan.sh` on box 2228 12/12 with
  `entries=23` (up from 21, so the two new items really did reach the cache) at
  `worst=2 differing=0.7797%`.
Commit: 086d06969

### Phase 3 — The arc cap, all three renderers

- [x] `__canvas_arcHeader` computes and stores the two sweep endpoints in slots 35–38;
      `HEADER_SLOTS` → 39; every `__CANVAS_GEO_HEADER` reader updated. (35–38 / 39, not
      36–39 / 40 — see **D1**.) The two endpoints ride the `__canvas_cos`/`__canvas_sin`
      calls the arc header already makes, so a round-capped arc costs no extra
      transcendental — which matters because the oracle's trig is a hand-written
      deterministic series, not libm.
- [x] `__canvas_geoDistance`'s arc arm takes the `min` with the two cap discs when the
      cap is `Round`. Butt returns the band untouched, so it is the pre-letter path.
- [x] The same in both shaders; extend the item block by one `ivec4`;
      `scripts/regen-spirv.sh`. → `ITEM_BLOCK_SIZE` 160 → 176, `frag -> 23804 bytes`,
      `vert -> 4140 bytes`. See **D6**: the *vertex* shader has its own copy of
      `ItemBlock` and widening only the fragment one would have shifted every item
      after the first.
- [x] Verify the existing `reach = radius + half + 1.0` bounds still contain a round
      cap (§4.2) — by test, on an arc whose sweep ends at the bounds' extreme. →
      `a_round_arc_cap_at_the_bounds_extreme_is_not_clipped`, on an arc whose start
      endpoint *is* the hull's +X extreme: the cap disc reaches x = 412 against a hull
      edge at 413, and the test asserts the outermost column is painted. The arithmetic
      says it fits; a hull one pixel short would have cut exactly that column and
      nothing else, which is why this is a measurement and not a paragraph.
- [x] Tests: `tests/rt_canvas_rasteriser.rs` — a butt-capped 0..PI arc (byte-identical
      to Phase 1's arc); the same arc round-capped (assert stroke pixels beyond the
      radial cut at each end); a full-circle arc (caps must be invisible either way).
      → `a_round_capped_arc_caps_its_sweep_ends_and_a_butt_one_does_not`,
      `a_round_arc_cap_at_the_bounds_extreme_is_not_clipped`, and
      `a_full_circle_arc_is_identical_with_either_cap` — the last comparing whole
      frames, because a disc drawn in the wrong place on a closed arc is a bulge no
      single-pixel check is positioned to see.
- [x] **Added:** a round-capped arc in both GPU harnesses, for the reason Phase 2 added
      the butt line — `smile` is butt-capped in both scenes, so the cap-disc arm would
      otherwise be compiled into both shaders and never taken.

Acceptance: the four new cases pass; the butt-capped arc is byte-identical to the same
arc at Phase 1's commit.

**MET.**

- `rt_canvas_rasteriser` 27 passed / 1 ignored (24 before this phase).
- **Butt arcs byte-identical**, and this one needs no special instrument: every stored
  reference contains an arc — `smiley.png` one, `blendmodes.png` four — and all three
  still match `compare_exact`, with `git status --short tests/golden/canvas/` empty.
- Both backends' layout guards fired on this change and were satisfied rather than
  silenced: `the_draw_frame_slots_do_not_overlap` (the item block at `OFF_ITEM` ran
  into `OFF_TEXTURE`) and `the_metal_shader_edge_base_matches_the_buffer_layout`
  (163840 → 180224). See **D7**.
- GPU parity with both styles on both variants: `rt_canvas_metal` 4/4,
  `rt_canvas_golden` 8/8, `rt_canvas_font` 12/12, and `scripts/test-canvas-vulkan.sh`
  on box 2228 12/12 with `entries=24` at `worst=2 differing=0.7818%` — up from
  0.7797%, which is the round arc's antialiased cap edges rather than a no-op.
Commit: 32b983bdb

### Phase 4 — GPU parity, docs, and the gates

- [x] Confirm neither `*Renderable` predicate needs to decline a cap — by test, on a
      scene containing both cap styles on both variants. → neither predicate reads a
      slot past `offset + 20`, and the cap is 34; confirmed at runtime by
      `the_gpu_draws_the_endcap_scene_the_reference_shows`, whose scene carries butt
      *and* round on *both* `Line` and `Arc` and which asserts `gpuSelected=TRUE`
      **before** comparing a pixel — a fallback to software would otherwise reproduce
      the reference perfectly and pass.
- [x] New reference image `tests/golden/canvas/endcaps.png`: butt and round, line and
      arc, at a stroke width wide enough to read. → four rows at stroke 28, each style
      beside its twin with everything else equal. The line row carries a thin white
      marker at each endpoint, because the whole claim about `Butt` is *where it stops*
      and an unmarked end is only checkable against arithmetic. Row 4 is the degenerate
      pair, and the butt half is deliberately **empty** — an absence that is part of the
      reference.
- [x] `mod.rs` — `CapStyle`'s and both `cap` fields' descriptions. Say what each cap
      does in terms a developer observes; no memory vocabulary
      (`scripts/man-census.sh --memory-scope` → 0 unclassified hits). → done in Phase 1
      with the type; census re-run here → **0 unclassified** (15 CARVE-1, 23 CARVE-2).
- [x] `src/docs/spec/app/06_canvas.md` §"Rendering conventions" — the two cap
      geometries, and the note that `Polygon` has no join style. → a new
      **`Line.cap` and `Arc.cap` shape the two ends of a stroke** block above the
      transform one, carrying both geometries as distances, the zero-length rule, the
      `Polygon` has-no-join note, and the asymmetry (a `Line` was round and an `Arc`
      butt before this letter) that decides which value is the compatible one. See
      **D8** for why its bullets are one source line each.
- [x] `scripts/regen-ncodesum.sh`; prove the delta is this letter's. → **141 refreshed,
      0 missing**, and `git status --porcelain tests/byte-identity/` empty afterwards.
      As in plan-116-C the delta is nil rather than merely accounted for: no
      byte-identity fixture imports `canvas`.

Acceptance: `endcaps.png` matches on the software oracle and on both GPUs within
`Tolerance::GPU_DEFAULT` with `MFB_CANVAS_STATS` confirming the GPU path ran;
`cargo test --no-fail-fast` green on mac+RELEASE and linux+DEBUG;
`scripts/test-accept.sh` green; `scripts/artifact-gate.sh all` 0 diffs.

**MET.**

- `endcaps_match_their_reference_exactly` — the software oracle reproduces the
  reference byte for byte.
- `the_gpu_draws_the_endcap_scene_the_reference_shows` — **Metal**, compared against
  the committed reference rather than a same-run oracle, inside
  `Tolerance::GPU_DEFAULT`, with `gpuSelected=TRUE` asserted first. `rt_canvas_golden`
  is 10/10.
- **Vulkan, both libc worlds**, with butt and round on both variants in the scene:
  box 2228 glibc and box 2227 musl, 12/12 each, `vulkanReady=TRUE gpuSelected=TRUE`,
  `worst=2 differing=0.7818%`.
- `cargo test --release --no-fail-fast` — **95 test binaries, 0 failures, exit 0.**
- `bash scripts/test-accept.sh` — **1348 test(s) ran**.
- `scripts/artifact-gate.sh target/release/mfb all` — 1327 tests, 1490 builds,
  **1828 goldens, 0 diffs**.
- The linux+DEBUG half is covered as plan-116-A established and B and C repeated,
  because box 2228 is a single core: the Vulkan harness on **both** Linux libc worlds,
  which is where this letter's Linux-specific work (the butt segment and the arc cap
  discs in the SPIR-V, and the widened item block) actually executes, plus the
  `--bin mfb` unit tests: `cargo test --release --no-fail-fast --bin mfb` on box 2228
  → **3722 passed, 0 failed** (1437.41s of tests after a ~2h single-core compile).
  Run at `cca22df71` — this letter's own last code commit — rather than after the
  final merge-back, which brought main's `append(list, f(x))` in-place fix and audio
  changes. Those are gated by their own plans, and re-running a 2-hour single-core row
  for each peer landing outruns main rather than checking it; what the merge had to
  answer is whether the two interact, which the 96-binary local suite and the 1348-test
  acceptance run answer.
Commit: cca22df71, 610d34e38 (the `examples/emoji` arc main brought in mid-letter)

## Validation Plan

- **Tests:** `tests/rt_canvas_rasteriser.rs` (8 cases), `tests/rt_canvas_golden.rs`
  (+`endcaps.png`), `tests/rt_canvas_metal.rs`, `tests/cli_canvas_package.rs`
  (the type surface). Negative cases: zero-length butt line draws nothing; a
  full-circle arc is cap-independent.
- **Coverage check:** the cap arms are MFBASIC source in emitted programs, invisible
  to `cargo llvm-cov --bin mfb`. Confirm both arms of both variants are exercised by
  distinct rt assertions — four arms, four assertions.
- **Runtime proof:** render `endcaps.png`'s scene software / Metal / Vulkan and diff.
- **Doc sync:** `src/docs/spec/app/06_canvas.md`; `CapStyle` and the two `cap` field
  descriptions in `mod.rs`.
- **Acceptance:** `cargo test --no-fail-fast`, `scripts/test-accept.sh`,
  `scripts/artifact-gate.sh all`, `rustup run 1.96.0 cargo fmt --all &&
  (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Which cap value each existing fixture takes (§3).** Recommended and effectively
  decided: `Line` → `Round`, `Arc` → `Butt`, because those are each variant's measured
  current behaviour and Phase 1's gate is "no golden moves". Flagged as a decision
  because the *natural* reading — "give both the same value" — would silently change
  `smiley.png`.
- **Sibling function vs. cap parameter on `__canvas_segmentDistance` (Phase 2).**
  Recommended: sibling, so the polygon edge walk is untouched.
- **The arc's radial butt cut stays hard (aliased), as today (§2).** Recommended for
  this letter: smoothing it (replace the `1000000.0` out-of-sweep return with a
  signed distance to the sweep's bounding half-planes) is a one-arm change but moves
  `smiley.png` and every arc byte, so it is its own deliberate change with its own
  golden regeneration — not a rider on a cap feature. If taken later, do it for
  BOTH `Butt` arcs and the sweep cut under `Round`, in one change, so the two cap
  styles stay mutually consistent.

## Corrections

- **D8 (2026-09-02, Phase 4) — a spec bullet must be ONE source line.** The embedded
  spec renderer treats leading spaces as list **nesting**, not as a lazy continuation
  (`src/docs/render.rs:14`), so a bullet wrapped across source lines renders its first
  line as a bullet and every later line as a fresh paragraph at column 0. Written as
  four indented lines, this letter's cap bullets came out as

  ```
   • A butt line is the round band intersected with the slab between the two
     planes
  through its endpoints, perpendicular to the segment. As a distance that is max(d
  ```

  Not a defect introduced here — it is how every multi-line bullet in the document
  already renders, including `## Retained, not immediate`'s three, which predate
  plan-116 entirely — and not a renderer bug to fix in this letter either, since the
  nesting-by-indent rule is deliberate and changing it would reflow the whole embedded
  spec. The authoring rule is the fix: **keep a bullet on one source line**, however
  long. Recorded because the source looks correct and only rendering shows it is not,
  which is the same class of trap as every other prose field in this project.

- **D7 (2026-09-02, Phase 3) — growing `ITEM_BLOCK_SIZE` moves two Metal constants
  that nothing else relates to it, and both guards fired.** The block went 160 → 176 for
  the arc-cap `ivec4`, and:

  * `the_draw_frame_slots_do_not_overlap` — the item block is built on Metal's
    hand-assigned stack frame at `OFF_ITEM = 192`, so at 176 bytes it ran to 368 and
    overlapped `OFF_TEXTURE` at 352. Every slot from `OFF_TEXTURE` up shifted by 16 and
    `DRAW_FRAME` went 512 → 528.
  * `the_metal_shader_edge_base_matches_the_buffer_layout` — the MSL's
    `METAL_EDGE_BASE` is a literal that must equal `CANVAS_ITEM_BUFFER_BYTES / 4`, so it
    went 163840 → 180224.

  Neither would have failed loudly. A frame-slot overlap corrupts a pointer the
  `objc_msgSend` sequence reads and produces a **black GPU frame that reports success**;
  a stale edge base makes every polygon read edges from the wrong offset of a buffer
  that is entirely valid memory. Recorded because the pattern is now three-for-three
  across plan-116 (A grew the block for the item buffer, C for the transform, D for the
  caps) and each time the guards were the only thing that noticed.

- **D6 (2026-09-02, Phase 3) — `ItemBlock` is declared TWICE, and the vertex copy is the
  one that sets the stride.** Adding `arcCaps` to `mfb_canvas.frag` alone left
  `mfb_canvas.vert`'s own declaration at ten members. The reflection is unambiguous:

  ```
  # before, mfb_canvas.vert
  Items.blocks.xform1: offset 144, ... topLevelArrayStride 160
  Items: ... size 160, numMembers 10
  # after
  Items.blocks.arcCaps: offset 160, ... topLevelArrayStride 176
  Items: ... size 176, numMembers 11
  ```

  With the two stages disagreeing, the vertex stage would index a 160-byte stride into a
  buffer the emitter writes at 176 — so every item after the first reads a block
  straddling two records, which draws plausible wrong shapes rather than failing. The
  plan's box says "extend the item block by one `ivec4`" without saying *where*, and
  only one of the two places is the one the stride comes from. Measured with
  `glslangValidator -V -q` on box 2228, which is what the `ITEM_BLOCK_SIZE` doc comment
  already tells you to re-run and is the reason it says so.

- **D5 (2026-09-02, Phase 2) — the site census missed `scripts/test-canvas-vulkan.sh`,
  because the harness embeds MFBASIC in a shell heredoc.** Phase 1's census used
  `grep -rn 'canvas::Line\[\|canvas::Arc\[' --include='*.rs' --include='*.mfb'` — the
  file-type filter the plan's §2 specifies — and the Vulkan harness is a `.sh`. Its two
  sites (`:118` an `Arc`, `:121` a `Line`) therefore kept building a scene with no `cap`
  field, which no longer compiles.

  Nothing caught it, and nothing would have until the harness was next run by hand:
  `scripts/test-canvas-vulkan.sh` is not part of `cargo test`, so Phase 1's 95 green
  test binaries were entirely consistent with a broken Linux GPU harness. Re-censused
  without a file-type filter (`grep -rn … . | grep -v '^./target/'`), which finds 16
  sites rather than 14; the two extra are exactly these. **The lesson generalises past
  this letter**: a census that filters by extension cannot see MFBASIC embedded in a
  shell script, and this repo has at least one such harness.

- **D4 (2026-09-02, Phase 2) — §4.2's butt-cap formula is wrong, and wrong in the
  direction that looks right.** The plan gives

  ```
  d_butt = max( d_round_with_t_clamped , -t*|v| , (t-1)*|v| )
  ```

  with the caller subtracting the stroke half-width afterwards, as it does for the
  round arm. That compares each end plane against the **half-width** instead of against
  zero, so the cap does not begin to cut until a pixel is more than `half` past the
  endpoint. Measured on a 20 px line ending at `x = 400`: pixel 405 stayed painted,
  because `max(5.52, 5.5) - 10 < 0`. The first written test caught it immediately.

  A butt stroke is the round **band** intersected with the slab between the end planes,
  and the SDF of an intersection of convex sets is the `max` of their SDFs — so `half`
  has to come off *before* the `max`, and the sibling therefore takes `half` and returns
  the finished band distance rather than a raw distance. Corrected in §4.2 and in all
  three renderers.

- **D3 (2026-09-02, Phase 1) — the phase's own "nothing reads it yet" makes a Rust
  `HEADER_CAP` constant dead code, and the sanctioned justifications do not cover it.**
  The obvious shape for a new header slot is a `pub(crate) const HEADER_CAP` beside
  `HEADER_BLEND` in `runtime/canvas/mod.rs`, with the layout guard asserting it equals
  the MFBASIC `__CANVAS_GEO_CAP`. Doing that produced `warning: constant HEADER_CAP is
  never used`: its only consumer would be a `#[cfg(test)]` assertion until Phase 2
  gives the emitters a cap arm. AGENTS.md allows a targeted `#[allow]`/`#[cfg(test)]`
  "+ comment why load-bearing (**never** 'consumed by a later phase')" — which is
  precisely and only what could be written here.

  So the constant is not added yet. It arrives in Phase 2, with the emitter that reads
  it. What Phase 1 *can* check is what the guard now checks: that the cap slot lands
  **inside** the header (`cap < HEADER_SLOTS`, else a `Line` writes over a polygon's
  first edge coordinate) and **past** every slot an emitter already names
  (`cap > HEADER_HAS_TRANSFORM`, else it overwrites a field the GPU paths read). Both
  failures draw a plausible wrong picture rather than failing, which is the reason to
  pin them at all.

  Worth recording because the tempting move is the opposite one — add the constant,
  suppress the warning, and let Phase 2 justify it — and that is how a stub with no
  production consumer ships.

- **D1 (2026-09-02, pre-Phase 1) — the header slot numbers were estimated from where
  plan-116-C was *expected* to leave the header, and C landed one slot lower.** This
  letter said the cap goes in slot 35, the arc endpoints in 36–39, and `HEADER_SLOTS`
  becomes 40. C landed **34** slots, 0–33
  (`grep -n "^pub(crate) const HEADER_SLOTS" src/codegen/runtime/canvas/mod.rs` → 34;
  `helper_geometry.rs:53` → `LET __CANVAS_GEO_HEADER AS Integer = 34`), because C's
  Correction C2 replaced `sqrt(|det M|)` with the gradient norm and so needed no
  per-axis slot. Every number shifts down by one: cap in **34**, endpoints in
  **35–38**, `HEADER_SLOTS` → **39**. `ITEM_BLOCK_SIZE` is 160 as assumed
  (`mod.rs:323`). Corrected in §4.1, §4.2, §Compatibility and Phase 1.

- **D2 (2026-09-02, pre-Phase 1) — the construction-site census was 11 and is 14, and
  the extra sites are not the ones the plan predicted.** Re-measured on the merged
  tree: `grep -rn 'canvas::Line\[\|canvas::Arc\[' … | grep -v '/target/'` → **14**.
  The plan named `examples/emoji/src/main.mfb:219` as the 11th; that file is **not in
  the tree** (`ls examples/` has no `emoji` — it exists only as an untracked directory
  in the shared main checkout, so it is a peer's or the user's work in progress and
  cannot be edited from here). The four genuinely new sites are
  `tests/rt_canvas_golden.rs:358-361`, the four blend-mode arcs plan-116-B added after
  this letter was written.

  Two consequences. The `tests/` row goes 7 → 10. And the plan's warning that "this
  count has already moved once" is now twice, so **re-run it again at Phase 1** — if a
  peer lands `examples/emoji`, it arrives with an `Arc` needing `cap := CapStyle.Butt`
  like every other existing site, and missing it would change that example's rendering
  rather than fail to compile only if the field were optional, which it is not.

  **It moved a third time, and exactly that way.** The merge-back before landing brought
  `examples/emoji` in from main (`1fda27fc9`) with the `Arc` at `:219`. Capped `Butt` —
  what an `Arc` did before this letter, so the example's rendering is unchanged — and
  `mfb build -app examples/emoji` confirms it compiles. The census is now **17**. The
  general point is the one Phase 2's D5 makes from the other direction: on a shared
  checkout the population a breaking field change has to sweep is not fixed at plan
  time, so it is re-measured at every merge, not once.

## Summary

The engineering is small and closed-form; the risk is entirely in the ten
construction sites. `Line` is round today and `Arc` is butt today — an asymmetry
neither type documents and which a reader would not guess — so a fixture given the
"obvious" cap value changes a reference image without changing a test name. Phase 1
lands the whole breaking field edit with the geometry still unread, precisely so that
gate fires alone. Untouched: `Polygon` joins, the other six `DrawItem` variants, and
every `Paint` field.
