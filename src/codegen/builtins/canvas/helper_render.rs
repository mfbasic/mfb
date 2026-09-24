//! `__canvas_renderScene` — the entry point `canvas::present` calls after a publish
//! that actually changed something.
//!
//! The renderer is written in MFBASIC source rather than emitted per architecture,
//! which is the same choice `json`, `regex` and `crypto` make for their algorithmic
//! cores. For a rasteriser it is doubly right: it is the **oracle** the GPU backends
//! (plan-98-E/F) are compared against, so it must produce identical pixels on every
//! target, and one source implementation gives that for free where five hand-written
//! assembly ports would each be a place for the oracle to disagree with itself.

use crate::codegen::registry::{RegistryHelper, RegistryPackage};
use crate::codegen::runtime::canvas::{
    CANVAS_MAX_FRAME_ITEMS, MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_EDGES,
    METAL_MAX_FRAME_GLYPH_SAMPLES, METAL_MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_ITEMS,
    VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
};
use std::sync::LazyLock;

/// Render the currently-installed scene into the surface buffer.
///
/// Reads the published scene rather than taking the item list as an argument: the
/// published copy is the one the renderer must draw, and re-reading it here keeps
/// "what was installed" and "what is drawn" the same object even when a later resize
/// or damage event re-renders without a `present`.
///
/// A layered scene and a flat one render through the same per-item path; the only
/// difference is that layers composite in order, which falls out of drawing them in
/// order onto one buffer.
///
/// Items are looked up by index alongside their published hashes, so an item and its
/// cache key stay paired. A hash the publish did not supply reads as `0`, which is a
/// key like any other: it costs a confirmation compare, never a wrong reuse.
#[rustfmt::skip]
const RENDER_SCENE: &str =
r#"FUNC __canvas_renderScene(offsets AS List OF Integer, damage AS List OF Integer, width AS Integer, height AS Integer) AS Nothing
  LET full AS Boolean = __canvas_damageIsFull(damage, width, height)
  MUT buffer AS List OF Byte = []
  IF full THEN
    buffer = canvas::newSurface(width, height)
  ELSE
    ' The previous frame's pixels, with only the damaged rectangle cleared back to the
    ' surface's own opaque black. Everything outside it is already correct -- that is
    ' the entire claim a partial redraw makes, and the reason the damage rectangle has
    ' to include where a moved item *was* as well as where it is.
    buffer = __CANVAS_KEPT
    LET x0 AS Integer = collections::getOr(damage, 0, 0)
    LET y0 AS Integer = collections::getOr(damage, 1, 0)
    LET x1 AS Integer = collections::getOr(damage, 2, 0)
    LET y1 AS Integer = collections::getOr(damage, 3, 0)
    MUT y AS Integer = y0
    WHILE y < y1
      LET row AS Integer = y * width * 4
      MUT x AS Integer = x0
      WHILE x < x1
        LET at AS Integer = row + x * 4
        buffer = collections::set(buffer, at, toByte(0))
        buffer = collections::set(buffer, at + 1, toByte(0))
        buffer = collections::set(buffer, at + 2, toByte(0))
        buffer = collections::set(buffer, at + 3, toByte(255))
        x = x + 1
      END WHILE
      y = y + 1
    END WHILE
  END IF
  ' plan-116-G: indexed rather than `FOR EACH`, because each draw entry now carries an
  ' accumulated group translation alongside its geometry offset. The two travel in
  ' parallel globals written by `__canvas_sceneOffsets` -- one list per fact, indexed
  ' together. A scene with no groups leaves every entry at 0.0 and draws exactly what it
  ' drew before this letter.
  MUT di AS Integer = 0
  LET drawCount AS Integer = len(offsets)
  WHILE di < drawCount
    LET offset AS Integer = collections::getOr(offsets, di, 0)
    LET gdx AS Float = collections::getOr(__CANVAS_DRAW_DX, di, 0.0)
    LET gdy AS Float = collections::getOr(__CANVAS_DRAW_DY, di, 0.0)
    IF full OR __canvas_boundsMeetOffset(offset, damage, gdx, gdy) THEN
      buffer = __canvas_drawGeometry(buffer, width, height, offset, gdx, gdy)
    END IF
    di = di + 1
  END WHILE
  ' Kept only for damage mode, the one reader (`__canvas_damageFor` answers a full
  ' redraw otherwise): the assignment is a whole-frame copy -- 20 MB at 2880x1800,
  ' measured at 6 ms a frame -- paid on every frame for nothing (bug-686).
  IF __canvas_damageEnabled() THEN
    __CANVAS_KEPT = buffer
    __CANVAS_KEPT_W = width
    __CANVAS_KEPT_H = height
  END IF
  __canvas_presentSurface(buffer, width, height)
END FUNC"#;

/// The GPU renderer (plan-98-E), and the scene walk both backends share.
///
/// `__canvas_sceneOffsets` is the same traversal `__canvas_renderScene` does — flat
/// items then layers in order, one hash index across both — reduced to what a
/// backend actually needs: the cache offset of each item's geometry, in draw order.
/// Generating it costs nothing extra when the Metal path declines, because
/// `__canvas_geometryFor` is the cache and the software walk that follows hits it.
///
/// `__canvas_metalRenderable` is the honesty gate: the renderer draws a scene only
/// when every item is one it reproduces, and hands the rest back to the oracle rather
/// than drawing it wrongly.
///
/// Phase 2's SDF fragment shader evaluates the same distance functions the software
/// rasteriser does, so every kind now passes — including `__CANVAS_GEO_NONE`, which
/// both backends draw as nothing. What remains are the frame buffer's region caps —
/// quads, polygon edges, gradient stops and glyph/picture texels, each a frame SUM.
/// There is no per-polygon edge cap since bug-686: the old one (256 edges, justified by
/// a 4 KB `setFragmentBytes:` payload that plan-116-A removed) sent every detailed
/// polygon to software, and its twin in the emitter drew such a polygon as nothing.
///
/// It reads the geometry header by slot rather than through a helper because that is
/// what the header is for: a fixed 22-float layout both backends index directly
/// (`__canvas_headerFor`).
///
/// `__canvas_vulkanRenderable` is the Vulkan predicate, and it declines a *different*
/// set — which is why it is a second function rather than a share of the first.
/// Vulkan's edges live in a descriptor-bound storage buffer, so there is no per-item
/// limit at all; the limit is the buffer, and the buffer serves the whole frame,
/// because a command buffer is recorded once and executed once. So Metal caps each
/// polygon and Vulkan caps their sum. The two really are different conditions, and a
/// scene can be GPU-renderable on one backend and not the other.
///
/// **A glyph run is bounded rather than refused**, and on both backends the bound is a
/// **frame** total: each glyph's bitmap is copied into a region of one buffer that
/// serves the whole frame, and its block names its slice (bug-670 moved Metal onto
/// Vulkan's design — its per-glyph `setFragmentBytes:` payload, capped at 4 KiB, was
/// overwritten before the instanced draw read it). Neither truncates: a clipped glyph is
/// a different glyph and would read as a rasteriser bug.
///
/// Both predicates declined `__CANVAS_GEO_TEXT` outright for as long as neither backend
/// could draw one, and that was not caution — the version before it *accepted* a kind
/// neither shader knew, and Metal returned a frame with the text simply missing and no
/// error anywhere. 4,536 pixels wrong, reported as success. That is the lie these
/// predicates exist to prevent, and it is why the bound is checked here rather than
/// discovered in the emitter.
///
/// Sharing the Metal predicate was the tempting shortcut when this shader still
/// declined every polygon, and it would have rendered them as nothing while reporting
/// success — the same lie both predicates exist to prevent. It was measured, not
/// assumed: the scene that found it differed from the oracle on 4,610 pixels, all of
/// them the one triangle.
///
/// The frame caps below are not literals in this text: each `LET __CANVAS_*MAX*` line
/// carries an `@NAME@` token that `RENDER_METAL` replaces with the Rust constant of that
/// name (`RENDER_CAPS`), so the predicate and the emitter it guards read ONE definition
/// (bug-686). They used to be two, joined only by a test.
#[rustfmt::skip]
const RENDER_METAL_TEMPLATE: &str =
r#"' plan-116-G: the accumulated group translation of each draw entry, parallel to the
' offsets list `__canvas_sceneOffsets` returns, and whether any group was expanded.
'
' Parallel globals rather than a widened return, because that return is consumed by
' four callers -- the render walk, both `*Renderable` predicates and the damage pass --
' which all index it one entry per item. Widening it to a strided record would touch
' every one of them to express something three of them never read.
MUT __CANVAS_DRAW_DX AS List OF Float = []
MUT __CANVAS_DRAW_DY AS List OF Float = []

' The hash of each DRAW entry, which is not the same list as the scene's hashes once a
' group is expanded: one `Group` node becomes N children.
'
' The damage diff pairs hashes with offsets by index, so it needs the expanded list or
' its two sides are different lengths and every group scene falls back to a full redraw.
' For a group-free scene this records exactly what `canvas::installedHashes()` holds --
' the value is passed straight through -- so nothing about damage changes for a scene
' that uses no groups.
MUT __CANVAS_DRAW_HASHES AS List OF Integer = []

LET __CANVAS_GROUP_MAX_DEPTH AS Integer = 64

' One draw entry, or a whole group's worth of them.
'
' The recursion section 4.4 describes, done where the draw list is built rather than as
' a separate pass over the published scene: it is the same walk either way, and doing it
' here means the offsets list a renderer receives is already flat, so no consumer of it
' has to know groups exist.
FUNC __canvas_appendDraw(offsets AS List OF Integer, item AS DrawItem, hash AS Integer, gdx AS Float, gdy AS Float, depth AS Integer) AS List OF Integer
  MUT out AS List OF Integer = offsets
  MATCH item
    CASE Group(g)
      ' Depth is counted per PATH, so a diamond -- two parents naming one child -- is
      ' legal and costs one level rather than two.
      '
      ' This stop is SILENT, and that is not the missing half of the depth rule. The
      ' raise happens on the WORKER, in `__canvas_groupSignature` inside `present`,
      ' because this function runs on the GRAPHICS THREAD -- a `FAIL` here has no
      ' `present` call to reach and no user frame to unwind to. By the time a scene is
      ' being drawn it has already passed the worker's check, so this is unreachable;
      ' it is here because "unreachable" plus "recursion" plus "a table another thread
      ' can edit" is not a combination to leave without a bound.
      IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
        RETURN out
      END IF
      ' An unresolved name draws nothing and does NOT raise: `canvas::groupItems`
      ' answers an empty list for the -1 a miss returns, so the loop runs zero times and
      ' no branch is needed to make an absent group a silent no-op.
      FOR EACH child IN canvas::groupItems(canvas::groupResolve(g.name))
        out = __canvas_appendDraw(out, child, __canvas_hashItem(child), gdx + g.dx, gdy + g.dy, depth + 1)
      NEXT
      RETURN out
    CASE ELSE
      LET offset AS Integer = __canvas_geometryFor(item, hash)
      out = collections::append(out, offset)
      __CANVAS_DRAW_DX = collections::append(__CANVAS_DRAW_DX, gdx)
      __CANVAS_DRAW_DY = collections::append(__CANVAS_DRAW_DY, gdy)
      ' The accumulated offset is folded into the recorded hash, not just carried
      ' beside it. The damage diff asks "is entry i the same as it was", and for an item
      ' inside a group the answer depends on WHERE the group put it: the geometry is
      ' identical when a node's `dx`/`dy` change, so a hash that ignored the offset
      ' reported "nothing changed" and the moved group was never repainted -- measured
      ' as `frames=1 skipped=1 damage=none` for a group moved 500px.
      '
      ' bug-484: a picture also folds in its image's pixel-block address. `canvas::setBytes`
      ' repaints without a `present`, so the scene's published hash -- computed by the
      ' worker when the scene was presented -- still names the OLD pixels, and the damage
      ' diff would report "nothing changed" and skip exactly the frame `setBytes` asked
      ' for. The geometry was just built on this thread from the live image, so its header
      ' holds the current block.
      MUT drawHash AS Integer = hash
      IF toInt(__canvas_geoAt(offset, 0)) = __CANVAS_GEO_PICTURE THEN
        drawHash = __canvas_hashFloat(__canvas_hashFloat(drawHash, __canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_HI)), __canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_LO))
      END IF
      __CANVAS_DRAW_HASHES = collections::append(__CANVAS_DRAW_HASHES, __canvas_hashFloat(__canvas_hashFloat(drawHash, gdx), gdy))
      RETURN out
  END MATCH
END FUNC

' The resolved-groups signature of a scene: `(slot, revision)` for every group node it
' reaches, depth-first, in scene order (section 4.4).
'
' This is what lets `present` see a group's CONTENTS change when the scene list it is
' handed is byte-identical to the published one. `publishScene` compares the raw bytes
' of the item list, and a `Group` node's bytes are two floats and a string pointer --
' all three unchanged by a `setGroup` under the same name. Without this the second
' `present` is skipped and the program draws the old group forever, with nothing raised.
'
' It runs on the WORKER, inside `present`, which is also why the depth limit is enforced
' here: this is the only place with a user call to fail back to.
FUNC __canvas_groupSignature(items AS List OF DrawItem, depth AS Integer) AS List OF Integer
  MUT sig AS List OF Integer = []
  FOR EACH item IN items
    MATCH item
      CASE Group(g)
        IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
          FAIL error(77050024, "canvas group nesting exceeded " & toString(__CANVAS_GROUP_MAX_DEPTH) & " levels -- this is a cycle or a bug")
        END IF
        LET slot AS Integer = canvas::groupResolve(g.name)
        sig = collections::append(sig, slot)
        sig = collections::append(sig, canvas::groupRevision(slot))
        FOR EACH inner IN __canvas_groupSignature(canvas::groupItems(slot), depth + 1)
          sig = collections::append(sig, inner)
        NEXT
      CASE ELSE
        sig = sig
    END MATCH
  NEXT
  RETURN sig
END FUNC

FUNC __canvas_intListEquals(a AS List OF Integer, b AS List OF Integer) AS Boolean
  IF len(a) <> len(b) THEN
    RETURN FALSE
  END IF
  MUT i AS Integer = 0
  WHILE i < len(a)
    IF collections::getOr(a, i, 0) <> collections::getOr(b, i, 0) THEN
      RETURN FALSE
    END IF
    i = i + 1
  END WHILE
  RETURN TRUE
END FUNC

' plan-116-H: the GPU draw list, and the block layout it indexes into.
'
' `__canvas_sceneDraws` is the sibling of `__canvas_sceneOffsets`. Both walk the same
' resolved tree; they differ in what they produce and, critically, in **how often a
' shared group is written**.
'
' `__CANVAS_DRAWS` is four integers per entry -- `(itemBase, itemCount, dx, dy)`, the
' offsets in 16.16 -- and `__CANVAS_DRAW_BLOCKS` is the flat list of geometry offsets the
' bases index into, one entry per item block the frame uploads.
'
' **The decision section 4.3 asks Phase 1 to make: a shared group's blocks are written
' ONCE and referenced, not once per reference.** Three reasons, and the first is the one
' that settles it:
'
'   1. It is the only shape in which a per-draw offset earns its place. The whole
'      apparatus this letter adds -- a Vulkan push constant, Metal `setVertexBytes:` --
'      exists so that two draws of one group differ only by a translation. If the blocks
'      were duplicated per reference, the offset could simply be baked into each copy
'      when it is written, and none of that machinery would be needed.
'   2. It bounds what reuse costs. `__CANVAS_MAX_FRAME_ITEMS` is 4096; a UI drawing one
'      200-item panel at thirty positions costs 200 blocks under sharing and 6000 under
'      duplication -- over the cap, so the frame would decline to software and the
'      feature would be slowest exactly where it is most used.
'   3. It is this letter's stated goal (1), reuse, expressed in the buffer.
'
' The consequence for the predicates is that **two different things are capped**: the
' number of BLOCKS (which a diamond does not double) and the number of DRAWS (which it
' does). They are summed separately below.
'
' A group is memoised by slot index, so the second reference to a group finds its blocks
' already laid out. Its own non-group items form one contiguous run; a nested group inside
' it is not part of that run -- it becomes its own memoised run plus a draw entry at the
' composed offset, which is what "a group ends the current run and starts a new one after
' it" means once the group can itself contain groups.
MUT __CANVAS_DRAWS AS List OF Integer = []
MUT __CANVAS_DRAW_BLOCKS AS List OF Integer = []

' bug-686: what `__canvas_sceneOffsets` resolved, per scene INDEX (the installed items,
' then every layer's items -- the order `canvas::installedHashes()` is in): the item's
' geometry offset, or -1 for a `Group` node. `__canvas_sceneDraws` lays the draw list out
' from this instead of resolving every item a second time.
MUT __CANVAS_TOP_OFFSETS AS List OF Integer = []
' The scene's items, in the same order -- fetched only on a frame where some item could
' not be resolved from its hash alone (a miss, a picture, a group), and empty otherwise.
' A frame whose items are all cached never copies the scene out of the ring.
MUT __CANVAS_FRAME_ITEMS AS List OF DrawItem = []
' Slot index -> the base of that group's own run in `__CANVAS_DRAW_BLOCKS`, and its
' length. Parallel lists rather than a Map because a Map of Integer to Integer would
' allocate per frame and this is walked once per group node.
MUT __CANVAS_DRAW_MEMO_SLOT AS List OF Integer = []
MUT __CANVAS_DRAW_MEMO_BASE AS List OF Integer = []
MUT __CANVAS_DRAW_MEMO_COUNT AS List OF Integer = []

' The instance index each block-list entry starts at.
'
' `base` and `count` in a draw entry are INSTANCES -- blocks in the item buffer -- and
' those are not one per block-list entry. A `Text` item is one entry here and N quads
' there, one per glyph, each taking its own block (plan-98-G). Every other kind is 1.
'
' Kept as a parallel list rather than folded into the block list because the block list
' is what the emitter's publish walk iterates, and that walk is unchanged: it still sees
' one entry per item and still lets the glyph path publish N blocks from one of them.
MUT __CANVAS_DRAW_INST AS List OF Integer = []
MUT __CANVAS_DRAW_NEXT_INST AS Integer = 0

' How many item-buffer blocks one geometry record occupies.
' How many INSTANCES one geometry block publishes. This has to agree, case for case,
' with what the emitters actually write into the item buffer -- `emit_split_or_publish`
' and `emit_glyph_publish` in src/codegen/runtime/canvas/vulkan.rs -- because the draw
' list's bases are running sums of this function while the item buffer's contents are
' the emitter's. A single case that disagrees shifts every base after it, and the last
' entry then draws instances that were never published: uninitialised buffer, which
' reaches the screen as opaque black. The failure therefore appears at the END of a
' scene, nowhere near the item that actually disagreed.
FUNC __canvas_blockInstances(offset AS Integer) AS Integer
  ' A text run is N quads, one instance each -- slot 20 is HEADER_AUX0, the same field
  ' `emit_glyph_publish` loops over.
  IF toInt(__canvas_geoAt(offset, 0)) = __CANVAS_GEO_TEXT THEN
    RETURN toInt(__canvas_geoAt(offset, 20))
  END IF
  ' A blended item that BOTH strokes and fills is published as two records -- fill with
  ' the stroke switched off, then stroke with the fill made transparent -- because one
  ' blended draw cannot composite the two against each other correctly. The three
  ' conditions are `emit_split_or_publish`'s, in its order: a non-Normal blend mode,
  ' a positive strokeHalf, and a non-zero fill alpha. A Line or an Arc arrives here
  ' fill-only with a negative strokeHalf (`__canvas_strokeAsFill`) and takes the
  ' single-record path.
  IF toInt(__canvas_geoAt(offset, 26)) <> 0 THEN
    IF __canvas_geoAt(offset, 7) > 0.0 THEN
      IF toInt(__canvas_geoAt(offset, 11)) > 0 THEN
        RETURN 2
      END IF
    END IF
  END IF
  RETURN 1
END FUNC

FUNC __canvas_memoLookup(slot AS Integer) AS Integer
  MUT i AS Integer = 0
  WHILE i < len(__CANVAS_DRAW_MEMO_SLOT)
    IF collections::getOr(__CANVAS_DRAW_MEMO_SLOT, i, -1) = slot THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  RETURN -1
END FUNC

' Append one draw entry: four integers, offsets in 16.16 so the whole list stays
' `List OF Integer` and reaches an emitter without a second parallel Float list.
SUB __canvas_pushOneDraw(base AS Integer, count AS Integer, dx AS Float, dy AS Float)
  IF count <= 0 THEN
    EXIT SUB
  END IF
  ' `base`/`count` arrive as block-list indices and leave as INSTANCES, which is what a
  ' `vkCmdDraw`'s `firstInstance`/`instanceCount` and Metal's `baseInstance` want. The
  ' two differ exactly where a `Text` item sits, so the conversion cannot be a constant
  ' factor and has to be summed.
  LET instBase AS Integer = collections::getOr(__CANVAS_DRAW_INST, base, 0)
  MUT instCount AS Integer = 0
  MUT k AS Integer = base
  WHILE k < base + count
    instCount = instCount + __canvas_blockInstances(collections::getOr(__CANVAS_DRAW_BLOCKS, k, 0))
    k = k + 1
  END WHILE
  IF instCount <= 0 THEN
    EXIT SUB
  END IF
  ' EIGHT words per entry, not four. The emitter addresses an entry with a shift, so the
  ' stride has to be a power of two; the three trailing words are reserved rather than
  ' packed into the others, because a mode smuggled into the high bits of `count` is the
  ' kind of encoding that survives exactly until someone draws 65536 instances.
  '
  ' This width is agreed in THREE places and there is no gate that checks they agree:
  ' here, `__canvas_drawsText` in helper_surface.rs, and `emit_draw_list_pass` in
  ' src/codegen/runtime/canvas/vulkan.rs (`shift_left_immediate(.., 6)` = 8 x 8 bytes).
  ' When they disagreed -- this SUB writing four words while the emitter strode by eight --
  ' the emitter read every other entry and took the following entry's `base` as a blend
  ' mode, which indexes the pipeline table out of range and hands Vulkan a junk
  ' VkPipeline. That does not fail cleanly: it SIGSEGVs inside the driver's JIT-compiled
  ' code, with a backtrace containing no MFBASIC frame at all.
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instBase)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instCount)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, toInt(dx * 65536.0))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, toInt(dy * 65536.0))
  ' The BlendMode every block in this run shares. It travels with the entry because the
  ' pipeline is bound per draw and the draws are issued in a second pass, so a binding
  ' left in the publish walk would apply the LAST item's mode to the whole frame.
  LET mode AS Integer = toInt(__canvas_geoAt(collections::getOr(__CANVAS_DRAW_BLOCKS, base, 0), 26))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, mode)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
END SUB

' Whether two blocks can share one draw call.
'
' A draw binds ONE pipeline and issues ONE instanced call, so everything that must
' differ between pipelines forces a split (**H8**). Two conditions, both read straight
' out of the geometry the blocks already point at rather than re-visiting the scene:
'
'   * the BlendMode (slot 26) selects the pipeline, so a change ends the run;
'   * a `Text` block is its own draw entirely -- its quads are N draws rather than N
'     instances -- so it neither joins the previous run nor starts one.
'
' Section 4.1 named only the group boundary and Text. Blend mode is the third, and
' omitting it draws a Multiply item with whichever pipeline happened to be bound, which
' is a wrong colour rather than a missing shape.
FUNC __canvas_drawsJoin(a AS Integer, b AS Integer) AS Boolean
  IF toInt(__canvas_geoAt(a, 0)) = __CANVAS_GEO_TEXT THEN
    RETURN FALSE
  END IF
  IF toInt(__canvas_geoAt(b, 0)) = __CANVAS_GEO_TEXT THEN
    RETURN FALSE
  END IF
  RETURN toInt(__canvas_geoAt(a, 26)) = toInt(__canvas_geoAt(b, 26))
END FUNC

' Emit draw entries for one contiguous block range, split wherever a pipeline change or
' a `Text` block forces one.
'
' Splitting HERE rather than in each emitter is deliberate: the list is what tells a
' backend where its draw calls are, so a rule applied afterwards would live in two
' assemblers and could differ between them -- the failure family
' `.ai/canvas-threading.md` section 10 records.
SUB __canvas_pushDraw(base AS Integer, count AS Integer, dx AS Float, dy AS Float)
  IF count <= 0 THEN
    EXIT SUB
  END IF
  MUT runStart AS Integer = base
  MUT i AS Integer = base + 1
  WHILE i < base + count
    LET prev AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i - 1, 0)
    LET here AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i, 0)
    IF NOT __canvas_drawsJoin(prev, here) THEN
      __canvas_pushOneDraw(runStart, i - runStart, dx, dy)
      runStart = i
    END IF
    i = i + 1
  END WHILE
  __canvas_pushOneDraw(runStart, base + count - runStart, dx, dy)
END SUB

' Lay out one group's own (non-group) items once, returning its memo index.
FUNC __canvas_memoGroup(slot AS Integer, hashes AS List OF Integer, depth AS Integer) AS Integer
  LET found AS Integer = __canvas_memoLookup(slot)
  IF found >= 0 THEN
    RETURN found
  END IF
  ' Claim the memo row BEFORE walking the children. A group that reaches itself would
  ' otherwise recurse forever here rather than tripping the depth limit -- and this walk
  ' runs on the graphics thread, where a raise has nowhere to go (plan-116-G G22).
  LET base AS Integer = len(__CANVAS_DRAW_BLOCKS)
  __CANVAS_DRAW_MEMO_SLOT = collections::append(__CANVAS_DRAW_MEMO_SLOT, slot)
  __CANVAS_DRAW_MEMO_BASE = collections::append(__CANVAS_DRAW_MEMO_BASE, base)
  __CANVAS_DRAW_MEMO_COUNT = collections::append(__CANVAS_DRAW_MEMO_COUNT, 0)
  LET row AS Integer = len(__CANVAS_DRAW_MEMO_SLOT) - 1

  MUT count AS Integer = 0
  FOR EACH child IN canvas::groupItems(slot)
    MATCH child
      CASE Group(g)
        ' A nested group is NOT part of this group's own run -- it gets its own memoised
        ' run and its own draw entry, at the composed offset.
        LET nested AS Integer = 0
      CASE ELSE
        LET childOffset AS Integer = __canvas_geometryFor(child, __canvas_hashItem(child))
        __CANVAS_DRAW_BLOCKS = collections::append(__CANVAS_DRAW_BLOCKS, childOffset)
        __CANVAS_DRAW_INST = collections::append(__CANVAS_DRAW_INST, __CANVAS_DRAW_NEXT_INST)
        __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(childOffset)
        count = count + 1
    END MATCH
  NEXT
  __CANVAS_DRAW_MEMO_COUNT = collections::set(__CANVAS_DRAW_MEMO_COUNT, row, count)
  RETURN row
END FUNC

' Emit the draw entries for one group reference at an accumulated offset: its own run,
' then its nested groups recursively.
SUB __canvas_drawGroup(slot AS Integer, hashes AS List OF Integer, gdx AS Float, gdy AS Float, depth AS Integer)
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
    EXIT SUB
  END IF
  LET row AS Integer = __canvas_memoGroup(slot, hashes, depth)
  __canvas_pushDraw(collections::getOr(__CANVAS_DRAW_MEMO_BASE, row, 0), collections::getOr(__CANVAS_DRAW_MEMO_COUNT, row, 0), gdx, gdy)
  FOR EACH child IN canvas::groupItems(slot)
    MATCH child
      CASE Group(g)
        __canvas_drawGroup(canvas::groupResolve(g.name), hashes, gdx + g.dx, gdy + g.dy, depth + 1)
      CASE ELSE
        ' A non-group child contributes no draw entry of its own: its block is already
        ' inside the run this group's memo laid out.
        LET skipped AS Integer = 0
    END MATCH
  NEXT
END SUB

FUNC __canvas_sceneDraws() AS List OF Integer
  __CANVAS_DRAWS = []
  __CANVAS_DRAW_BLOCKS = []
  __CANVAS_DRAW_INST = []
  __CANVAS_DRAW_NEXT_INST = 0
  __CANVAS_DRAW_MEMO_SLOT = []
  __CANVAS_DRAW_MEMO_BASE = []
  __CANVAS_DRAW_MEMO_COUNT = []
  LET hashes AS List OF Integer = []
  MUT runBase AS Integer = 0
  MUT runCount AS Integer = 0
  ' bug-686: the offsets `__canvas_sceneOffsets` already resolved, one per scene index --
  ' the installed items AND every layer's items. This walk used to resolve every item
  ' again (a second full pass of geometry lookups per frame), and it walked only
  ' `canvas::installedItems()`, so a scene installed with `presentLayers` published no
  ' blocks at all and Metal drew an empty frame.
  LET count AS Integer = len(__CANVAS_TOP_OFFSETS)
  ' Bound once, not written inline as `getOr`'s default: a module-function call in a
  ' LATER operand makes the global list operand a snapshot (`.ai/collections.md`,
  ' bug-496), and snapshotting the whole scene per item made this walk O(n^2).
  LET none AS DrawItem = __canvas_noItem()
  MUT index AS Integer = 0
  WHILE index < count
    LET itemOffset AS Integer = collections::getOr(__CANVAS_TOP_OFFSETS, index, 0 - 1)
    IF itemOffset < 0 THEN
      ' A group node: it ends the current run, then lays out (or reuses) its own.
      MATCH collections::getOr(__CANVAS_FRAME_ITEMS, index, none)
        CASE Group(g)
          __canvas_pushDraw(runBase, runCount, 0.0, 0.0)
          runCount = 0
          __canvas_drawGroup(canvas::groupResolve(g.name), hashes, g.dx, g.dy, 0)
          runBase = len(__CANVAS_DRAW_BLOCKS)
        CASE ELSE
          LET unreachable AS Integer = 0
      END MATCH
    ELSE
      ' Straight into the GLOBAL (site S2 of the in-place self-update table,
      ' `.ai/collections.md`), an amortised O(1) write -- binding it to a local first
      ' copied the whole list per item and made this walk O(n^2).
      __CANVAS_DRAW_BLOCKS = collections::append(__CANVAS_DRAW_BLOCKS, itemOffset)
      __CANVAS_DRAW_INST = collections::append(__CANVAS_DRAW_INST, __CANVAS_DRAW_NEXT_INST)
      __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(itemOffset)
      IF runCount = 0 THEN
        runBase = len(__CANVAS_DRAW_BLOCKS) - 1
      END IF
      runCount = runCount + 1
    END IF
    index = index + 1
  END WHILE
  __canvas_pushDraw(runBase, runCount, 0.0, 0.0)
  RETURN __CANVAS_DRAWS
END FUNC

' A stand-in `DrawItem` for an index `__CANVAS_FRAME_ITEMS` does not hold. A group named
' by nothing resolves to no slot and draws nothing, so reaching it is harmless.
FUNC __canvas_noItem() AS DrawItem
  RETURN Group[name := "", dx := 0.0, dy := 0.0]
END FUNC

' Every item of the installed scene, in `canvas::installedHashes()` order: the flat items,
' then each layer's.
FUNC __canvas_flatScene() AS List OF DrawItem
  MUT out AS List OF DrawItem = canvas::installedItems()
  FOR EACH layer IN canvas::installedLayers()
    FOR EACH item IN layer.items
      out = collections::append(out, item)
    NEXT
  NEXT
  RETURN out
END FUNC

FUNC __canvas_sceneOffsets() AS List OF Integer
  MUT offsets AS List OF Integer = []
  LET hashes AS List OF Integer = canvas::installedHashes()
  __CANVAS_DRAW_DX = []
  __CANVAS_DRAW_DY = []
  __CANVAS_DRAW_HASHES = []
  __CANVAS_TOP_OFFSETS = []
  __CANVAS_FRAME_ITEMS = []
  ' bug-682/bug-686: the one point in a frame where NO geometry offset is live -- the
  ' previous frame's are dead and this frame's do not exist yet -- so it is the only place
  ' the cache may drop a slot or move a float. Nothing after this point in the frame does.
  __canvas_geoBeginFrame()

  ' bug-686, pass 1: resolve every item from its published hash alone. On a scene whose
  ' items did not change this is the whole walk -- one map probe per item, and the scene
  ' is never copied out of the ring.
  LET count AS Integer = len(hashes)
  MUT probes AS List OF Integer = []
  MUT unresolved AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i < count
    LET probe AS Integer = __canvas_geoProbe(collections::getOr(hashes, i, 0))
    probes = collections::append(probes, probe)
    IF probe < 0 THEN
      unresolved = TRUE
    END IF
    i = i + 1
  END WHILE
  IF unresolved THEN
    __CANVAS_FRAME_ITEMS = __canvas_flatScene()
  END IF

  ' Bound once -- see `__canvas_sceneDraws`: a call in `getOr`'s default would snapshot
  ' the whole global scene list per item.
  LET none AS DrawItem = __canvas_noItem()

  ' Pass 2: lay the frame out. A resolved item is one entry at no offset; anything else
  ' goes through `__canvas_appendDraw`, which builds a missing item's geometry, re-reads
  ' a picture's image, and expands a group into its children.
  i = 0
  WHILE i < count
    LET hash AS Integer = collections::getOr(hashes, i, 0)
    LET probe AS Integer = collections::getOr(probes, i, 0 - 1)
    IF probe >= 0 THEN
      offsets = collections::append(offsets, probe)
      __CANVAS_DRAW_DX = collections::append(__CANVAS_DRAW_DX, 0.0)
      __CANVAS_DRAW_DY = collections::append(__CANVAS_DRAW_DY, 0.0)
      ' Exactly what `__canvas_appendDraw` records for an item at no group offset. A
      ' resolved item is never a picture (`__canvas_geoProbe` refers those back), so the
      ' pixel-block fold it adds does not apply.
      __CANVAS_DRAW_HASHES = collections::append(__CANVAS_DRAW_HASHES, __canvas_hashFloat(__canvas_hashFloat(hash, 0.0), 0.0))
      __CANVAS_TOP_OFFSETS = collections::append(__CANVAS_TOP_OFFSETS, probe)
    ELSE
      LET item AS DrawItem = collections::getOr(__CANVAS_FRAME_ITEMS, i, none)
      LET before AS Integer = len(offsets)
      offsets = __canvas_appendDraw(offsets, item, hash, 0.0, 0.0, 0)
      MATCH item
        CASE Group(g)
          __CANVAS_TOP_OFFSETS = collections::append(__CANVAS_TOP_OFFSETS, 0 - 1)
        CASE ELSE
          __CANVAS_TOP_OFFSETS = collections::append(__CANVAS_TOP_OFFSETS, collections::getOr(offsets, before, 0 - 1))
      END MATCH
    END IF
    i = i + 1
  END WHILE
  RETURN offsets
END FUNC

' The frame's item buffer holds one block per drawn QUAD, and both backends index it by
' instance -- so this is the one cap that is neither Metal's nor Vulkan's, it is the
' shared transport's (plan-116-A, `CANVAS_MAX_FRAME_ITEMS`). A shape is one quad; a glyph
' run is one per glyph, because each glyph is its own quad with its own block.
'
' Counting the run's whole glyph count over-estimates by the glyphs whose cache entry the
' eviction pass dropped -- those draw nothing and take no block. Over-estimating declines
' a hair early, which is the safe direction: under-estimating would let the emitter write
' past the mapping.
'
' bug-686: Metal's item buffer is larger than Vulkan's, so each backend has its own
' cap -- `__CANVAS_MAX_FRAME_ITEMS` is Vulkan's, `__CANVAS_METAL_MAX_FRAME_ITEMS`
' Metal's. Every value on these lines is generated from the Rust constant it names.
LET __CANVAS_MAX_FRAME_ITEMS AS Integer = @CANVAS_MAX_FRAME_ITEMS@
LET __CANVAS_METAL_MAX_FRAME_ITEMS AS Integer = @METAL_MAX_FRAME_ITEMS@

' The glyph samples one frame may carry on Metal, summed over its runs -- a frame-wide
' region like Vulkan's (bug-670), and eight times its size since bug-686, because a
' picture's texels ride it too. `__canvas_runSamples` below counts a run's share.
LET __CANVAS_METAL_MAX_FRAME_GLYPH_SAMPLES AS Integer = @METAL_MAX_FRAME_GLYPH_SAMPLES@

LET __CANVAS_METAL_MAX_FRAME_EDGES AS Integer = @METAL_MAX_FRAME_EDGES@

' The gradient stops one frame may carry, summed over its items -- the cap on the
' gradient region of each backend's shared buffer (plan-116-F). Vulkan's is
' `__CANVAS_MAX_FRAME_GRADIENT_STOPS`; Metal's was the same number until bug-686
' found 2,100 two-stop gradients declined by it.
LET __CANVAS_MAX_FRAME_GRADIENT_STOPS AS Integer = @MAX_FRAME_GRADIENT_STOPS@
LET __CANVAS_METAL_MAX_FRAME_GRADIENT_STOPS AS Integer = @METAL_MAX_FRAME_GRADIENT_STOPS@

FUNC __canvas_metalRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  MUT samples AS Integer = 0
  MUT quads AS Integer = 0
  MUT gradientStops AS Integer = 0
  MUT pictures AS Set OF Integer = Set OF Integer { }
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      samples = samples + __canvas_runSamples(offset)
    END IF
    ' bug-484: a picture's texels ride the same frame-wide region as glyph coverage, one
    ' word each, so they are counted against the same cap -- the region overflowing would
    ' make one picture read another's texels, a plausible wrong image.
    '
    ' bug-686: counted once per distinct pixel BLOCK, because the Metal emitter uploads
    ' each block once per frame and points every later picture of it at those texels
    ' (`emit_picture_lookup`). The key is the block address the emitter looks up, rebuilt
    ' from the same two header slots. Vulkan still copies per item, so its predicate still
    ' counts per item.
    IF kind = __CANVAS_GEO_PICTURE THEN
      LET block AS Integer = toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_HI)) * __CANVAS_GEO_PICTURE_SPLIT + toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_LO))
      IF NOT collections::contains(pictures, block) THEN
        pictures = collections::add(pictures, block)
        samples = samples + __canvas_pictureSamples(offset)
      END IF
    END IF
    ' The cap counts PUBLISHED RECORDS, so it asks the same function the draw list
    ' asks. A blended item that both strokes and fills publishes two
    ' (`emit_split_or_publish`); counting it as one let a scene near the cap write past
    ' the mapping, which is the direction this predicate exists to prevent.
    quads = quads + __canvas_blockInstances(offset)
    ' plan-116-F Phase 4: a gradient's stops take a slice of one frame-wide region, so
    ' what the frame can hold is a SUM and not a per-item bound -- the same shape the
    ' edge cap has. A count below two is not a gradient and contributes nothing.
    LET stops AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_GRADIENT_COUNT, 0.0))
    IF stops >= 2 THEN
      gradientStops = gradientStops + stops
    END IF
    ' bug-686: no per-polygon cap. A polygon of any size draws, bounded only by the
    ' frame's edge region below; the emitter indexes a large one by band so the shader
    ' does not walk every edge per pixel.
    IF kind = __CANVAS_GEO_POLYGON THEN
      total = total + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    END IF
  NEXT
  IF quads > __CANVAS_METAL_MAX_FRAME_ITEMS THEN
    RETURN FALSE
  END IF
  ' The glyph region serves the whole frame, so overflowing it would make one glyph
  ' read another's coverage -- declined rather than drawn wrongly (bug-670).
  IF samples > __CANVAS_METAL_MAX_FRAME_GLYPH_SAMPLES THEN
    RETURN FALSE
  END IF
  ' The FRAME cap, new in plan-116-A and the one scene class Metal newly declines: its
  ' edges used to ride an unbounded per-item `setFragmentBytes:` payload copied into the
  ' command buffer, and they now take a slice of one region that serves the whole frame,
  ' exactly as Vulkan's always have. Software is the oracle, so a declined scene is at
  ' least as correct -- truncating it would draw a DIFFERENT shape.
  ' The gradient region is one buffer serving the whole frame, so overflowing it
  ' would make one item's stops read another's -- a plausible wrong ramp rather
  ' than a failure. Software is the oracle, so declining is at worst slow.
  IF gradientStops > __CANVAS_METAL_MAX_FRAME_GRADIENT_STOPS THEN
    RETURN FALSE
  END IF
  ' plan-116-H Phase 3: Metal no longer declines a scene containing a group either. The
  ' emitter walks `__canvas_sceneDraws` and sends each entry's offset to BOTH shader
  ' stages -- `setVertexBytes:` at index 1 and `setFragmentBytes:` at index 3 -- so a
  ' group's children land where the group puts them rather than at the origin.
  '
  ' With both backends taught, `__CANVAS_DRAW_HAS_GROUP` has no reader and is deleted
  ' rather than left set for a caller that might want it. The rule it encoded is the
  ' part worth keeping, and it lives in `__canvas_vulkanRenderable` and in
  ' `.ai/canvas-threading.md` section 10: a predicate cannot decline by looking for a
  ' `Group` item, because the walk has already expanded every group away by then.
  RETURN total <= __CANVAS_METAL_MAX_FRAME_EDGES
END FUNC

FUNC __canvas_renderMetal(offsets AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF NOT __canvas_metalRenderable(offsets) THEN
    RETURN FALSE
  END IF
  ' bug-686 Phase 4: in a window, present the GPU frame straight to the window's
  ' `CAMetalLayer` -- no CPU surface, no readback, no swizzle, no `CGImage` blit, which
  ' at a full-screen surface were the whole frame budget. Only when nothing needs the
  ' frame's pixels on the CPU: damage mode keeps them (`__CANVAS_KEPT`) and a `--debug`
  ' dump writes them. `metalPresentScene` answers FALSE when there is nowhere to present
  ' (headless: no window layer, decided before Metal is touched) or no drawable this
  ' frame, and the frame then takes the readback path below -- so every headless test
  ' still compares exactly the pixels it always did.
  IF NOT __canvas_damageEnabled() AND NOT __canvas_dumpRequested() THEN
    IF canvas::metalPresentScene(width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS) THEN
      __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
      __canvas_presentedDirect()
      RETURN TRUE
    END IF
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  ' plan-116-H: the BLOCK list, not the software walk's offsets. A shared group appears
  ' once in it; `__CANVAS_DRAWS` says who draws which slice and at what offset. Same
  ' arguments as the Vulkan twin, and deliberately so -- the two backends walking
  ' different lists is how a group ends up correct on one and at the origin on the other.
  canvas::metalDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS)
  ' Damage mode only -- see `__canvas_renderScene`: a whole-frame copy otherwise paid
  ' for nothing (bug-686).
  IF __canvas_damageEnabled() THEN
    __CANVAS_KEPT = buffer
    __CANVAS_KEPT_W = width
    __CANVAS_KEPT_H = height
  END IF
  ' Counted HERE, before presenting: `__canvas_presentSurface` is what writes the stats
  ' line, so a counter bumped by the caller afterwards lags a frame and reads 0 on the
  ' only frame a headless test renders.
  __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
  __canvas_presentSurface(buffer, width, height)
  RETURN TRUE
END FUNC

LET __CANVAS_VULKAN_MAX_FRAME_EDGES AS Integer = @VULKAN_MAX_FRAME_EDGES@
LET __CANVAS_VULKAN_MAX_GLYPH_SAMPLES AS Integer = @VULKAN_MAX_FRAME_GLYPH_SAMPLES@

' The coverage samples one glyph run puts in the frame's glyph region -- the sum of its
' cached bitmaps' areas. A run carries cache indices rather than bitmaps, so this is the
' only place the two caches meet, and it is why the predicate takes the shape it does:
' the question "does this frame fit" cannot be answered from the geometry alone.
FUNC __canvas_runSamples(offset AS Integer) AS Integer
  LET glyphs AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  MUT total AS Integer = 0
  MUT g AS Integer = 0
  WHILE g < glyphs
    LET entry AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + g * 3, 0.0))
    IF entry >= 0 THEN
      LET base AS Integer = entry * 5
      total = total + collections::getOr(__CANVAS_GLYPH_META, base + 2, 0) * collections::getOr(__CANVAS_GLYPH_META, base + 3, 0)
    END IF
    g = g + 1
  END WHILE
  RETURN total
END FUNC

' The words one picture puts in the frame's glyph region: its image's texel count, from
' the aux pair its header carries (bug-484).
FUNC __canvas_pictureSamples(offset AS Integer) AS Integer
  RETURN toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0)) * toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 21, 0.0))
END FUNC

FUNC __canvas_vulkanRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  MUT samples AS Integer = 0
  MUT quads AS Integer = 0
  MUT gradientStops AS Integer = 0
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      samples = samples + __canvas_runSamples(offset)
    END IF
    ' bug-484: as in `__canvas_metalRenderable` -- texels share the glyph region's cap.
    IF kind = __CANVAS_GEO_PICTURE THEN
      samples = samples + __canvas_pictureSamples(offset)
    END IF
    ' The cap counts PUBLISHED RECORDS, so it has to ask the same function the draw
    ' list asks. A blended item that both strokes and fills publishes two
    ' (`emit_split_or_publish`), and counting it as one let a scene near the cap write
    ' past the mapping -- the direction this predicate exists to prevent.
    quads = quads + __canvas_blockInstances(offset)
    ' plan-116-F Phase 4: a gradient's stops take a slice of one frame-wide region, so
    ' what the frame can hold is a SUM and not a per-item bound -- the same shape the
    ' edge cap has. A count below two is not a gradient and contributes nothing.
    LET stops AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_GRADIENT_COUNT, 0.0))
    IF stops >= 2 THEN
      gradientStops = gradientStops + stops
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      total = total + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    END IF
  NEXT
  IF quads > __CANVAS_MAX_FRAME_ITEMS THEN
    RETURN FALSE
  END IF
  IF samples > __CANVAS_VULKAN_MAX_GLYPH_SAMPLES THEN
    RETURN FALSE
  END IF
  ' The gradient region is one buffer serving the whole frame, so overflowing it
  ' would make one item's stops read another's -- a plausible wrong ramp rather
  ' than a failure. Software is the oracle, so declining is at worst slow.
  IF gradientStops > __CANVAS_MAX_FRAME_GRADIENT_STOPS THEN
    RETURN FALSE
  END IF
  ' plan-116-H Phase 2: Vulkan no longer declines a scene containing a group. The
  ' emitter walks `__canvas_sceneDraws` and pushes each entry's offset as a push
  ' constant that BOTH shader stages consume, so a group's children land where the
  ' group puts them rather than at the origin.
  '
  ' Phase 3 taught Metal the same offset, so `__CANVAS_DRAW_HAS_GROUP` -- the flag both
  ' predicates used to decline on -- has no reader left and is gone. If a future backend
  ' needs to decline a group scene it has to set a flag on the WALK again rather than
  ' look for a `Group` item here: by the time a predicate sees this list the walk has
  ' expanded every group away, so searching for one finds nothing and the backend draws
  ' every group's children at the ORIGIN -- a plausible wrong picture reported as
  ' success (`.ai/canvas-threading.md` section 10).
  '
  ' No cap needs a per-reference multiplier here. A group's blocks are recorded ONCE
  ' and referenced by base, in both walks: a diamond referencing one leaf twice reports
  ' `entries=1 blocks=1` and two draw entries that share base 0 with different offsets.
  ' The caps above sum over recorded blocks, which is exactly what the item buffer
  ' holds.
  RETURN total <= __CANVAS_VULKAN_MAX_FRAME_EDGES
END FUNC

FUNC __canvas_renderVulkan(offsets AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF NOT __canvas_vulkanRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  ' plan-116-H: the BLOCK list, not the software walk's offsets. A shared group appears
  ' once in it; `__CANVAS_DRAWS` says who draws which slice and at what offset.
  canvas::vulkanDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS)
  ' Damage mode only -- see `__canvas_renderScene`: a whole-frame copy otherwise paid
  ' for nothing (bug-686).
  IF __canvas_damageEnabled() THEN
    __CANVAS_KEPT = buffer
    __CANVAS_KEPT_W = width
    __CANVAS_KEPT_H = height
  END IF
  ' Counted HERE, before presenting: `__canvas_presentSurface` is what writes the stats
  ' line, so a counter bumped by the caller afterwards lags a frame and reads 0 on the
  ' only frame a headless test renders.
  __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
  __canvas_presentSurface(buffer, width, height)
  RETURN TRUE
END FUNC"#;

/// The frame caps `RENDER_METAL_TEMPLATE` shares with the native emitters, by the
/// `@NAME@` token each `LET` line carries. One entry per token; a token with no entry
/// would reach the MFBASIC compiler as a syntax error, and
/// `every_cap_token_in_the_render_source_is_generated` pins that none is left.
const RENDER_CAPS: &[(&str, usize)] = &[
    ("CANVAS_MAX_FRAME_ITEMS", CANVAS_MAX_FRAME_ITEMS),
    ("METAL_MAX_FRAME_ITEMS", METAL_MAX_FRAME_ITEMS),
    (
        "METAL_MAX_FRAME_GLYPH_SAMPLES",
        METAL_MAX_FRAME_GLYPH_SAMPLES,
    ),
    ("METAL_MAX_FRAME_EDGES", METAL_MAX_FRAME_EDGES),
    ("MAX_FRAME_GRADIENT_STOPS", MAX_FRAME_GRADIENT_STOPS),
    (
        "METAL_MAX_FRAME_GRADIENT_STOPS",
        METAL_MAX_FRAME_GRADIENT_STOPS,
    ),
    ("VULKAN_MAX_FRAME_EDGES", VULKAN_MAX_FRAME_EDGES),
    (
        "VULKAN_MAX_FRAME_GLYPH_SAMPLES",
        VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    ),
];

/// `RENDER_METAL_TEMPLATE` with every cap token replaced by its Rust constant.
static RENDER_METAL: LazyLock<String> = LazyLock::new(|| {
    RENDER_CAPS
        .iter()
        .fold(RENDER_METAL_TEMPLATE.to_string(), |text, (name, value)| {
            text.replace(&format!("@{name}@"), &value.to_string())
        })
});

/// The per-item content hashes for a scene, in item order.
///
/// A layered scene flattens into one hash list in draw order, which is exactly the
/// order `__canvas_renderScene` walks — so one index serves both shapes and the
/// renderer needs no shape-specific hash lookup.
#[rustfmt::skip]
const HASH_SCENE: &str =
r#"' bug-686: `carried` is `canvas::carriedHashes(items)`, taken before the publish --
' the installed hash for every item whose bytes did not change, `-1` for the rest.
' `canvas::sceneHashes` keeps the carried ones and hashes the rest natively in one pass
' over the list, except a `Text` or a `Group`, which it leaves at `-1` for
' `__canvas_hashItem`'s MFBASIC arms below. So a scene of the common kinds costs the
' worker one native call and never extracts an item from the list. `items` and
' `carried` have the same length by construction; `sceneHashes` treats a short
' `carried` as all `-1`.
FUNC __canvas_hashScene(items AS List OF DrawItem, carried AS List OF Integer) AS List OF Integer
  MUT out AS List OF Integer = canvas::sceneHashes(items, carried)
  LET count AS Integer = len(out)
  MUT i AS Integer = 0
  WHILE i < count
    IF collections::getOr(out, i, 0) < 0 THEN
      ' Never read: every index below `count` is present. A group naming nothing
      ' would draw nothing, so it is also a harmless value if it ever were.
      LET none AS DrawItem = Group[name := "", dx := 0.0, dy := 0.0]
      LET item AS DrawItem = collections::getOr(items, i, none)
      out = collections::set(out, i, __canvas_hashItem(item))
    END IF
    i = i + 1
  END WHILE
  RETURN out
END FUNC

FUNC __canvas_hashLayers(layers AS List OF DrawLayer) AS List OF Integer
  MUT out AS List OF Integer = []
  FOR EACH layer IN layers
    FOR EACH item IN layer.items
      out = collections::append(out, __canvas_hashItem(item))
    NEXT
  NEXT
  RETURN out
END FUNC"#;

/// Start the graphics thread on the first present, and settle sync mode and the
/// renderer with it.
///
/// The guard makes this one `os::getEnvOr` per program rather than per frame, and it
/// keeps the environment read in MFBASIC — where it is already portable — instead of
/// putting a per-platform `getenv` on the spawn path.
///
/// **The renderer.** A program in a real window draws on the GPU (Metal on macOS,
/// Vulkan on Linux, where a pipeline exists); a headless run — any of the three
/// `MFB_*_HEADLESS` switches the tests use — draws in software.
/// `MFB_CANVAS_GPU` overrides both ways: `0` forces software, any other value
/// forces the GPU. The software renderer is the exact-match oracle the goldens and
/// the GPU backends are measured against, and every test that renders runs
/// headless, so it stays the default *there*; a window is where frame rate is
/// felt — a full-window canvas redraw in software runs at about 8 frames a second
/// on Apple silicon, against about 55 on Metal. A GPU that is unavailable, or a
/// scene it declines, still falls back to software per frame
/// (`__canvas_renderFrame`).
#[rustfmt::skip]
const ENSURE_GRAPHICS: &str =
r#"MUT __CANVAS_GFX_READY AS Boolean = FALSE

FUNC __canvas_ensureGraphics() AS Nothing
  IF NOT __CANVAS_GFX_READY THEN
    canvas::setSyncMode(len(os::getEnvOr("MFB_CANVAS_SYNC", "")) > 0)
    canvas::setGpuMode(__canvas_wantGpu())
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC

FUNC __canvas_wantGpu() AS Boolean
  LET choice AS String = os::getEnvOr("MFB_CANVAS_GPU", "")
  IF len(choice) > 0 THEN RETURN choice <> "0"
  IF len(os::getEnvOr("MFB_MACAPP_HEADLESS", "")) > 0 THEN RETURN FALSE
  IF len(os::getEnvOr("MFB_GTKAPP_HEADLESS", "")) > 0 THEN RETURN FALSE
  IF len(os::getEnvOr("MFB_WINAPP_HEADLESS", "")) > 0 THEN RETURN FALSE
  RETURN TRUE
END FUNC"#;

// `__canvas_renderLoop`/`__canvas_renderFrame` are one body in both builds except for
// one line: a skipped frame writes the stats line only in a `--debug` build
// (plan-130-E). The shared text is split at that line so it exists once.
macro_rules! render_loop_head {
    () => {
        r#"FUNC __canvas_renderLoop() AS Nothing
  WHILE canvas::waitForRedraw()
    __canvas_renderFrame()
    canvas::frameDone()
  END WHILE
END FUNC

FUNC __canvas_renderFrame() AS Nothing
  LET size AS Size = __canvas_surfaceSize()
  __canvas_phaseMark(0)
  ' The geometry is built once, here, and the offsets are then handed to whichever
  ' backend draws them. It has to happen before the damage diff rather than inside the
  ' renderer: an item's damaged rectangle is its geometry's bounds, so there is no
  ' diff to compute until the geometry exists.
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  __canvas_phaseMark(1)
  ' plan-116-H Phase 1: build the GPU draw list beside the software one. Nothing
  ' consumes it yet -- both backends still decline a group scene -- but it is built every
  ' frame so `MFB_CANVAS_STATS` can report it, which is the only way a test can see a
  ' structure that exists on the graphics thread and is handed straight to an emitter.
  '
  ' bug-686: it lays the list out from the offsets `__canvas_sceneOffsets` just resolved
  ' rather than resolving every item again.
  LET draws AS List OF Integer = __canvas_sceneDraws()
  __canvas_phaseMark(2)
  ' plan-116-G, G9: hold the graphics thread INSIDE a frame, for tests that need a
  ' worker action to land mid-render.
  '
  ' Here rather than anywhere else in the frame because this is the point after which
  ' the renderer is committed: `__canvas_sceneOffsets` has resolved every group name and
  ' copied out each group's items, so a `removeGroup` arriving during the hold is
  ' exactly the race the drain gate exists for -- the block is retired while a frame is
  ' demonstrably still working from it.
  '
  ' Off unless the variable is set, and off the production path in the same sense the
  ' other four affordances in `.ai/canvas-threading.md` section 11 are. Before this,
  ' every "mid-render" row was reached through `MFB_CANVAS_RESIZE_W`/`_H` firing while
  ' the worker slept, which is resize-specific -- so the rows that needed a different
  ' worker action were either untestable (R1) or tested by luck.
  LET holdMs AS Integer = toInt(os::getEnvOr("MFB_CANVAS_FRAME_HOLD_MS", "0"))
  IF holdMs > 0 THEN
    os::sleep(holdMs)
  END IF
  ' plan-116-G: the EXPANDED hashes, written by the walk above, not
  ' `canvas::installedHashes()`. The damage diff pairs hashes with offsets by index, and
  ' expanding one `Group` node into N children makes those two lists different lengths
  ' -- which `__canvas_damageFor` detects and answers with a full redraw, so a group
  ' scene would silently never take the partial path. For a group-free scene this list
  ' is `installedHashes()` passed through unchanged.
  LET hashes AS List OF Integer = __CANVAS_DRAW_HASHES
  LET damage AS List OF Integer = __canvas_damageFor(hashes, offsets, size.width, size.height)
  __CANVAS_DAMAGE = damage
  __canvas_phaseMark(3)
  IF len(damage) = 0 THEN
    ' Nothing changed, so nothing is presented. The loop still calls `frameDone`, so a
    ' `present` waiting under MFB_CANVAS_SYNC is released -- a skipped frame is a frame
    ' that finished, not one that was lost.
    __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
"#
    };
}

macro_rules! render_loop_tail {
    () => {
        r#"    RETURN
  END IF
  __CANVAS_FRAMES = __CANVAS_FRAMES + 1
  IF NOT __canvas_damageIsFull(damage, size.width, size.height) THEN
    __CANVAS_PARTIAL = __CANVAS_PARTIAL + 1
  END IF
  __canvas_rememberScene(hashes, offsets)

  ' A GPU backend renders the whole frame whatever the damage is: it draws into its own
  ' texture and reads the result back, so there is no kept surface for it to preserve.
  ' That is the plan's "capability-absent path falls back to full-frame", and it is the
  ' honest reading here -- the capability the plan named (`VK_KHR_incremental_present`,
  ' Metal dirty rects) belongs to presenting a swapchain drawable, which this renderer
  ' does not do at all (Correction 24).
  IF canvas::useGpu() AND canvas::metalReady() THEN
    IF __canvas_renderMetal(offsets, size.width, size.height) THEN
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  IF canvas::useGpu() AND canvas::vulkanReady() THEN
    IF __canvas_renderVulkan(offsets, size.width, size.height) THEN
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  __canvas_renderScene(offsets, damage, size.width, size.height)
  __canvas_phaseMark(4)
END FUNC"#
    };
}

/// The graphics thread's whole life: wait, render, repeat — and the renderer seam.
///
/// `__canvas_renderFrame` is the one place that will choose a renderer (plan-98-E).
/// It is deliberately a runtime branch rather than a build-time one, because the
/// choice is a runtime fact: whether a Metal device exists, and whether the program
/// asked for it (`canvas::metalAvailable` and `canvas::useGpu` answer those).
///
/// There are two GPU arms — Metal on macOS, Vulkan on Linux — and each is taken only
/// when all three of its conditions hold: the program asked for a GPU
/// (`canvas::useGpu`, which despite its name is the one renderer-selection flag and
/// is set by `__canvas_wantGpu`), a pipeline exists (`canvas::metalReady` /
/// `canvas::vulkanReady`), and the *scene* is one that renderer draws correctly.
///
/// The two are mutually exclusive in practice — `metalReady` is FALSE off macOS and
/// `vulkanReady` FALSE off Linux — so the order between them never decides anything;
/// they are written as separate `IF`s rather than an `ELSE` so a future platform with
/// both is a matter of which is listed first, not a restructure.
///
/// That third condition is what keeps `MFB_CANVAS_GPU=1` honest: a backend that
/// drew a circle as its bounding box would still *report* success, which is exactly
/// the lie Correction 3 rejected. Since Phase 2 the shader reproduces every
/// primitive, so the only scene still declined is one carrying a polygon with more
/// edges than a `setFragmentBytes:` payload holds.
///
/// The software path stays the default for headless runs, which is every test that
/// renders: it is the oracle the GPU path is measured against, so it cannot become
/// the thing being measured. A real window asks for the GPU (`__canvas_wantGpu`).
///
/// It never returns. The wait is a real condition wait, so a static scene costs
/// nothing — no timer, no poll, no spin (`.ai/canvas-threading.md` §4: time is
/// deliberately not a redraw trigger).
///
/// It renders the *installed* scene rather than being handed one, which is what lets
/// a repaint no `present` caused — a resize, an expose — draw the right picture.
const RENDER_LOOP: &str = concat!(render_loop_head!(), render_loop_tail!());

/// bug-686 Phase 0: where a frame's time goes, reported on the `MFB_CANVAS_STATS` line.
///
/// `__canvas_renderFrame` marks five points: the frame's start, then the end of the scene
/// walk, of the draw list, of the damage diff and of the render (the predicate, the
/// backend's own work, readback and present). A normal build's mark is an empty `SUB`;
/// a `--debug` build's accumulates the nanoseconds between consecutive marks per phase,
/// and `__canvas_phaseText` reports each total in milliseconds. Cumulative, like
/// `generations=`, because the interesting quantity is the delta between frames.
const PHASE_TIMERS: &str = r#"SUB __canvas_phaseMark(phase AS Integer)
END SUB"#;

#[rustfmt::skip]
const PHASE_TIMERS_DEBUG: &str = r#"MUT __CANVAS_PHASE_AT AS Integer = 0
MUT __CANVAS_PHASE_NS AS List OF Integer = [0, 0, 0, 0]

SUB __canvas_phaseMark(phase AS Integer)
  LET now AS Integer = canvas::frameNanos()
  IF phase > 0 AND __CANVAS_PHASE_AT > 0 THEN
    LET spent AS Integer = collections::getOr(__CANVAS_PHASE_NS, phase - 1, 0) + (now - __CANVAS_PHASE_AT)
    __CANVAS_PHASE_NS = collections::set(__CANVAS_PHASE_NS, phase - 1, spent)
  END IF
  __CANVAS_PHASE_AT = now
END SUB

FUNC __canvas_phaseText() AS String
  RETURN " phaseOffsetsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 0, 0) / 1000000) & " phaseDrawsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 1, 0) / 1000000) & " phaseDamageMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 2, 0) / 1000000) & " phaseRenderMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 3, 0) / 1000000)
END FUNC"#;

/// [`RENDER_LOOP`] for a `--debug` build: a skipped frame also writes the
/// `MFB_CANVAS_STATS` line.
const RENDER_LOOP_DEBUG: &str = concat!(
    render_loop_head!(),
    "    __canvas_writeStats()\n",
    render_loop_tail!()
);

/// bug-686 Phase 4: the two `--debug`-only facts `__canvas_renderMetal` needs about a
/// directly presented frame — normal-build bodies.
///
/// A normal build has no frame dump (plan-130-E), so nothing ever needs a GPU frame's
/// pixels on the CPU for it, and nothing writes a stats line. Both bodies are therefore
/// the answer, not a stub: `FALSE`, and nothing to do.
#[rustfmt::skip]
const DIRECT_PRESENT: &str =
r#"FUNC __canvas_dumpRequested() AS Boolean
  RETURN FALSE
END FUNC

SUB __canvas_presentedDirect()
END SUB"#;

/// [`DIRECT_PRESENT`] for a `--debug` build.
///
/// `MFB_CANVAS_DUMP` names a file every frame's RGBA is written to, so while it is set
/// the frame must be read back — the direct present has no CPU pixels to write. Read
/// once and cached, like `__canvas_damageEnabled`, rather than a `getenv` per frame
/// (0 unresolved, 1 off, 2 on).
///
/// A directly presented frame never reaches `__canvas_presentSurface`, which is where
/// the stats line is written for every other rendered frame, so it writes it here —
/// after `__CANVAS_GPU_FRAMES` is bumped, for the reason `__canvas_renderMetal` gives.
///
/// `__CANVAS_DIRECT_FRAMES` is the stats line's `directFrames=`: how many of the
/// `gpuFrames=` never came back to the CPU. `gpuFrames=` alone cannot tell the two
/// GPU paths apart — both draw the same picture — and the real-window test in
/// `scripts/test-macapp.sh` needs to know which one it captured.
#[rustfmt::skip]
const DIRECT_PRESENT_DEBUG: &str =
r#"MUT __CANVAS_DUMP_MODE AS Integer = 0
MUT __CANVAS_DIRECT_FRAMES AS Integer = 0

FUNC __canvas_dumpRequested() AS Boolean
  IF __CANVAS_DUMP_MODE = 0 THEN
    __CANVAS_DUMP_MODE = 1
    IF len(os::getEnvOr("MFB_CANVAS_DUMP", "")) > 0 THEN
      __CANVAS_DUMP_MODE = 2
    END IF
  END IF
  RETURN __CANVAS_DUMP_MODE = 2
END FUNC

SUB __canvas_presentedDirect()
  __CANVAS_DIRECT_FRAMES = __CANVAS_DIRECT_FRAMES + 1
  __canvas_writeStats()
END SUB"#;

/// plan-116-J: close the resources a retired group buffer owned.
///
/// **Not "close what the retired buffer named" — close what no LIVE buffer names**, and
/// the difference is the commonest canvas program there is (**J14**). One long-lived font
/// and a group rebuilt each frame names the same font in the retired buffer *and* in the
/// buffer that replaced it; closing on the plain rule makes its text vanish one frame
/// later, silently, because `fontHandle` then answers `0` and `0` is "no such object".
///
/// Identity is compared through `canvas::imageHandle`/`canvas::fontHandle`, which return
/// the backend id as an `Integer`. That is only possible because plan-116-I added them —
/// two aliases of one resource cannot be compared as `RES` values — and it is why their
/// read order matters here too: they test the closed flag **before** loading the handle,
/// so a concurrent destroy cannot yield a stale non-zero id that would keep a resource
/// alive that nothing names.
///
/// `0` is skipped on both sides: an already-closed resource has nothing to close, and it
/// must not match a live one either.
const CLOSE_RETIRED: &str = r#"SUB __canvas_closeRetired(gone AS List OF DrawItem, scene AS List OF DrawItem)
  IF len(gone) = 0 THEN
    EXIT SUB
  END IF
  FOR EACH item IN gone
    MATCH item
      CASE Picture(p)
        LET ih AS Integer = canvas::imageHandle(p.image)
        IF ih <> 0 AND NOT __canvas_anythingNamesImage(scene, ih) THEN
          canvas::destroyImage(p.image)
        END IF
      CASE Text(t)
        LET fh AS Integer = canvas::fontHandle(t.font)
        IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
          canvas::destroyFont(t.font)
        END IF
      CASE ELSE
    END MATCH
  NEXT
END SUB

' Everything still live: the scene about to be published, and EVERY group's live items.
'
' Not just the replacing slot's. Two other holders reach the same resource and both are
' reachable from ordinary programs: another group naming it, and the scene naming it
' directly. `present` does not take ownership, so a Picture built before the setGroup
' reaches the scene with nothing for the move checker to object to -- and closing it there
' makes the scene's item draw nothing, silently, one frame later.
'
' Every slot rather than the ones the scene references: an unreferenced group is not
' drawn today and may be drawn tomorrow, so its items are live regardless.
FUNC __canvas_anythingNamesImage(scene AS List OF DrawItem, handle AS Integer) AS Boolean
  IF __canvas_listNamesImage(scene, handle) THEN
    RETURN TRUE
  END IF
  MUT i AS Integer = 0
  LET slots AS Integer = canvas::groupSlots()
  WHILE i < slots
    IF __canvas_listNamesImage(canvas::groupItems(i), handle) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC

FUNC __canvas_anythingNamesFont(scene AS List OF DrawItem, handle AS Integer) AS Boolean
  IF __canvas_listNamesFont(scene, handle) THEN
    RETURN TRUE
  END IF
  MUT i AS Integer = 0
  LET slots AS Integer = canvas::groupSlots()
  WHILE i < slots
    IF __canvas_listNamesFont(canvas::groupItems(i), handle) THEN
      RETURN TRUE
    END IF
    i = i + 1
  END WHILE
  RETURN FALSE
END FUNC

FUNC __canvas_listNamesImage(live AS List OF DrawItem, handle AS Integer) AS Boolean
  FOR EACH item IN live
    MATCH item
      CASE Picture(p)
        IF canvas::imageHandle(p.image) = handle THEN
          RETURN TRUE
        END IF
      CASE ELSE
    END MATCH
  NEXT
  RETURN FALSE
END FUNC

FUNC __canvas_listNamesFont(live AS List OF DrawItem, handle AS Integer) AS Boolean
  FOR EACH item IN live
    MATCH item
      CASE Text(t)
        IF canvas::fontHandle(t.font) = handle THEN
          RETURN TRUE
        END IF
      CASE ELSE
    END MATCH
  NEXT
  RETURN FALSE
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_ensureGraphics",
        ENSURE_GRAPHICS,
    ));
    for helper in
        RegistryHelper::debug_split("canvas_phaseTimers", PHASE_TIMERS, PHASE_TIMERS_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in RegistryHelper::debug_split("canvas_renderLoop", RENDER_LOOP, RENDER_LOOP_DEBUG) {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always("canvas_hashScene", HASH_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderScene", RENDER_SCENE));
    pkg.add_helper(RegistryHelper::always(
        "canvas_renderMetal",
        RENDER_METAL.as_str(),
    ));
    for helper in
        RegistryHelper::debug_split("canvas_directPresent", DIRECT_PRESENT, DIRECT_PRESENT_DEBUG)
    {
        pkg.add_helper(helper);
    }
    pkg.add_helper(RegistryHelper::always("canvas_closeRetired", CLOSE_RETIRED));
}

#[cfg(test)]
mod tests;
