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
  ' parallel globals written by `__canvas_sceneOffsets`, the arrangement
  ' `__CANVAS_GEO_LIVE` already uses in that function -- one list per fact, indexed
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
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
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
/// both backends draw as nothing. **One condition remains**: a polygon's edges cross
/// as a `setFragmentBytes:` payload, which Metal caps at 4 KB, so a polygon past
/// `__CANVAS_METAL_MAX_EDGES` is declined. Clamping it instead would render a
/// *different polygon* and read as a geometry bug.
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
/// **A glyph run is bounded rather than refused**, and the two bounds have different
/// shapes for a real reason. Metal's bitmap rides `setFragmentBytes:`, copied into the
/// command buffer per draw, so its cap is 4 KiB **per glyph** — about 64x64, a glyph at
/// roughly 200 px. Vulkan's bitmaps are copied into one buffer that serves the whole
/// recording, so its cap is a **frame** total. Neither truncates: a clipped glyph is a
/// different glyph and would read as a rasteriser bug.
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
#[rustfmt::skip]
const RENDER_METAL: &str =
r#"' plan-116-G: the accumulated group translation of each draw entry, parallel to the
' offsets list `__canvas_sceneOffsets` returns, and whether any group was expanded.
'
' Parallel globals rather than a widened return, because that return is consumed by
' four callers -- the render walk, both `*Renderable` predicates and the damage pass --
' which all index it one entry per item. Widening it to a strided record would touch
' every one of them to express something three of them never read.
MUT __CANVAS_DRAW_DX AS List OF Float = []
MUT __CANVAS_DRAW_DY AS List OF Float = []
MUT __CANVAS_DRAW_HAS_GROUP AS Boolean = FALSE

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
      __CANVAS_DRAW_HAS_GROUP = TRUE
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
      __CANVAS_GEO_LIVE = collections::append(__CANVAS_GEO_LIVE, offset)
      __CANVAS_DRAW_DX = collections::append(__CANVAS_DRAW_DX, gdx)
      __CANVAS_DRAW_DY = collections::append(__CANVAS_DRAW_DY, gdy)
      ' The accumulated offset is folded into the recorded hash, not just carried
      ' beside it. The damage diff asks "is entry i the same as it was", and for an item
      ' inside a group the answer depends on WHERE the group put it: the geometry is
      ' identical when a node's `dx`/`dy` change, so a hash that ignored the offset
      ' reported "nothing changed" and the moved group was never repainted -- measured
      ' as `frames=1 skipped=1 damage=none` for a group moved 500px.
      __CANVAS_DRAW_HASHES = collections::append(__CANVAS_DRAW_HASHES, __canvas_hashFloat(__canvas_hashFloat(hash, gdx), gdy))
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
  MUT out AS List OF Integer = __CANVAS_DRAWS
  out = collections::append(out, instBase)
  out = collections::append(out, instCount)
  out = collections::append(out, toInt(dx * 65536.0))
  out = collections::append(out, toInt(dy * 65536.0))
  ' The BlendMode every block in this run shares. It travels with the entry because the
  ' pipeline is bound per draw and the draws are issued in a second pass, so a binding
  ' left in the publish walk would apply the LAST item's mode to the whole frame.
  out = collections::append(out, toInt(__canvas_geoAt(collections::getOr(__CANVAS_DRAW_BLOCKS, base, 0), 26)))
  out = collections::append(out, 0)
  out = collections::append(out, 0)
  out = collections::append(out, 0)
  __CANVAS_DRAWS = out
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
        MUT blocks AS List OF Integer = __CANVAS_DRAW_BLOCKS
        blocks = collections::append(blocks, childOffset)
        __CANVAS_DRAW_BLOCKS = blocks
        MUT inst AS List OF Integer = __CANVAS_DRAW_INST
        inst = collections::append(inst, __CANVAS_DRAW_NEXT_INST)
        __CANVAS_DRAW_INST = inst
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
  LET hashes AS List OF Integer = canvas::installedHashes()
  MUT index AS Integer = 0
  MUT runBase AS Integer = 0
  MUT runCount AS Integer = 0
  FOR EACH item IN canvas::installedItems()
    MATCH item
      CASE Group(g)
        ' A group ends the current run.
        __canvas_pushDraw(runBase, runCount, 0.0, 0.0)
        runCount = 0
        __canvas_drawGroup(canvas::groupResolve(g.name), hashes, g.dx, g.dy, 0)
        runBase = len(__CANVAS_DRAW_BLOCKS)
      CASE ELSE
        LET itemOffset AS Integer = __canvas_geometryFor(item, collections::getOr(hashes, index, 0))
        MUT blocks AS List OF Integer = __CANVAS_DRAW_BLOCKS
        blocks = collections::append(blocks, itemOffset)
        __CANVAS_DRAW_BLOCKS = blocks
        MUT inst AS List OF Integer = __CANVAS_DRAW_INST
        inst = collections::append(inst, __CANVAS_DRAW_NEXT_INST)
        __CANVAS_DRAW_INST = inst
        __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(itemOffset)
        IF runCount = 0 THEN
          runBase = len(__CANVAS_DRAW_BLOCKS) - 1
        END IF
        runCount = runCount + 1
    END MATCH
    index = index + 1
  NEXT
  __canvas_pushDraw(runBase, runCount, 0.0, 0.0)
  RETURN __CANVAS_DRAWS
END FUNC

FUNC __canvas_sceneOffsets() AS List OF Integer
  MUT offsets AS List OF Integer = []
  LET hashes AS List OF Integer = canvas::installedHashes()
  MUT index AS Integer = 0
  __CANVAS_DRAW_DX = []
  __CANVAS_DRAW_DY = []
  __CANVAS_DRAW_HASHES = []
  __CANVAS_DRAW_HAS_GROUP = FALSE
  ' Published as it goes, not at the end. The geometry cache is smaller than a large
  ' scene, so resolving item 300 can evict item 1 -- while this frame is still holding
  ' item 1's offset and has not drawn it yet. `__canvas_glyphEvict` reads this list to
  ' know that those glyphs are live; without it a 300-item scene lost six of them,
  ' silently, because their cache indices were renumbered out from under the offsets
  ' this function had already returned.
  __CANVAS_GEO_LIVE = []
  ' The result lands in a local first: `__canvas_geometryFor` can run an eviction pass
  ' that reassigns `__CANVAS_GEO_LIVE`, and appending to a global whose operand was
  ' resolved before the call writes into the block that pass released
  ' (`.ai/collections.md`). `__canvas_appendDraw` keeps that discipline.
  FOR EACH item IN canvas::installedItems()
    offsets = __canvas_appendDraw(offsets, item, collections::getOr(hashes, index, 0), 0.0, 0.0, 0)
    index = index + 1
  NEXT
  FOR EACH layer IN canvas::installedLayers()
    FOR EACH item IN layer.items
      offsets = __canvas_appendDraw(offsets, item, collections::getOr(hashes, index, 0), 0.0, 0.0, 0)
      index = index + 1
    NEXT
  NEXT
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
LET __CANVAS_MAX_FRAME_ITEMS AS Integer = 4096

LET __CANVAS_METAL_MAX_EDGES AS Integer = 256

LET __CANVAS_METAL_MAX_GLYPH_SAMPLES AS Integer = 4096

' The largest bitmap in a glyph run. Metal's cap is PER GLYPH, not per frame, because
' its bitmaps ride `setFragmentBytes:` -- the same payload its edges ride -- and that is
' copied into the command buffer per draw. Vulkan's is per frame for the opposite
' reason: one buffer serves the whole recording.
FUNC __canvas_runLargestGlyph(offset AS Integer) AS Integer
  LET glyphs AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  MUT worst AS Integer = 0
  MUT g AS Integer = 0
  WHILE g < glyphs
    LET entry AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_HEADER + g * 3, 0.0))
    IF entry >= 0 THEN
      LET base AS Integer = entry * 5
      LET samples AS Integer = collections::getOr(__CANVAS_GLYPH_META, base + 2, 0) * collections::getOr(__CANVAS_GLYPH_META, base + 3, 0)
      IF samples > worst THEN
        worst = samples
      END IF
    END IF
    g = g + 1
  END WHILE
  RETURN worst
END FUNC

LET __CANVAS_METAL_MAX_FRAME_EDGES AS Integer = 16384

' The gradient stops one frame may carry, summed over its items -- the cap on the
' third region of both backends' shared buffer, and the same number for both
' because the region is sized identically on each (plan-116-F).
LET __CANVAS_MAX_FRAME_GRADIENT_STOPS AS Integer = 4096

FUNC __canvas_metalRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  MUT quads AS Integer = 0
  MUT gradientStops AS Integer = 0
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      IF __canvas_runLargestGlyph(offset) > __CANVAS_METAL_MAX_GLYPH_SAMPLES THEN
        RETURN FALSE
      END IF
      quads = quads + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    ELSE
      quads = quads + 1
    END IF
    ' plan-116-F Phase 4: a gradient's stops take a slice of one frame-wide region, so
    ' what the frame can hold is a SUM and not a per-item bound -- the same shape the
    ' edge cap has. A count below two is not a gradient and contributes nothing.
    LET stops AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_GRADIENT_COUNT, 0.0))
    IF stops >= 2 THEN
      gradientStops = gradientStops + stops
    END IF
    IF kind = __CANVAS_GEO_POLYGON THEN
      ' The PER-ITEM cap, kept exactly as it was. plan-116-A moved Metal's edges into a
      ' frame buffer, so this one is no longer forced by the transport -- but declining
      ' the same scenes Metal declined before is that letter's gate, and unifying the
      ' two backends' caps is later work, taken deliberately or not at all.
      IF toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0)) > __CANVAS_METAL_MAX_EDGES THEN
        RETURN FALSE
      END IF
      total = total + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    END IF
  NEXT
  IF quads > __CANVAS_MAX_FRAME_ITEMS THEN
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
  IF gradientStops > __CANVAS_MAX_FRAME_GRADIENT_STOPS THEN
    RETURN FALSE
  END IF
  ' plan-116-G: decline any scene that contained a group, until plan-116-H teaches the
  ' backends the per-draw offset. Read from the walk's own flag rather than by looking
  ' for a `Group` item, because by the time a predicate sees this list the walk has
  ' already expanded every group away -- searching for one would find nothing and the
  ' GPU would draw every group's children at the ORIGIN, which is a plausible wrong
  ' picture reported as success (`.ai/canvas-threading.md` section 10).
  IF __CANVAS_DRAW_HAS_GROUP THEN
    RETURN FALSE
  END IF
  RETURN total <= __CANVAS_METAL_MAX_FRAME_EDGES
END FUNC

FUNC __canvas_renderMetal(offsets AS List OF Integer, width AS Integer, height AS Integer) AS Boolean
  IF NOT __canvas_metalRenderable(offsets) THEN
    RETURN FALSE
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  canvas::metalDrawScene(buffer, width, height, __CANVAS_GEO_DATA, offsets, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV)
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
  ' Counted HERE, before presenting: `__canvas_presentSurface` is what writes the stats
  ' line, so a counter bumped by the caller afterwards lags a frame and reads 0 on the
  ' only frame a headless test renders.
  __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
  __canvas_presentSurface(buffer, width, height)
  RETURN TRUE
END FUNC

LET __CANVAS_VULKAN_MAX_FRAME_EDGES AS Integer = 16384
LET __CANVAS_VULKAN_MAX_GLYPH_SAMPLES AS Integer = 1048576

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

FUNC __canvas_vulkanRenderable(offsets AS List OF Integer) AS Boolean
  MUT total AS Integer = 0
  MUT samples AS Integer = 0
  MUT quads AS Integer = 0
  MUT gradientStops AS Integer = 0
  FOR EACH offset IN offsets
    LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
    IF kind = __CANVAS_GEO_TEXT THEN
      samples = samples + __canvas_runSamples(offset)
      quads = quads + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
    ELSE
      quads = quads + 1
    END IF
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
  ' plan-116-G: decline any scene that contained a group, until plan-116-H teaches the
  ' backends the per-draw offset. Read from the walk's own flag rather than by looking
  ' for a `Group` item, because by the time a predicate sees this list the walk has
  ' already expanded every group away -- searching for one would find nothing and the
  ' GPU would draw every group's children at the ORIGIN, which is a plausible wrong
  ' picture reported as success (`.ai/canvas-threading.md` section 10).
  IF __CANVAS_DRAW_HAS_GROUP THEN
    RETURN FALSE
  END IF
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
  __CANVAS_KEPT = buffer
  __CANVAS_KEPT_W = width
  __CANVAS_KEPT_H = height
  ' Counted HERE, before presenting: `__canvas_presentSurface` is what writes the stats
  ' line, so a counter bumped by the caller afterwards lags a frame and reads 0 on the
  ' only frame a headless test renders.
  __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
  __canvas_presentSurface(buffer, width, height)
  RETURN TRUE
END FUNC"#;

/// The per-item content hashes for a scene, in item order.
///
/// A layered scene flattens into one hash list in draw order, which is exactly the
/// order `__canvas_renderScene` walks — so one index serves both shapes and the
/// renderer needs no shape-specific hash lookup.
#[rustfmt::skip]
const HASH_SCENE: &str =
r#"FUNC __canvas_hashScene(items AS List OF DrawItem) AS List OF Integer
  MUT out AS List OF Integer = []
  FOR EACH item IN items
    out = collections::append(out, __canvas_hashItem(item))
  NEXT
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

/// Start the graphics thread on the first present, and settle sync mode with it.
///
/// The guard makes this one `os::getEnvOr` per program rather than per frame, and it
/// keeps the environment read in MFBASIC — where it is already portable — instead of
/// putting a per-platform `getenv` on the spawn path.
#[rustfmt::skip]
const ENSURE_GRAPHICS: &str =
r#"MUT __CANVAS_GFX_READY AS Boolean = FALSE

FUNC __canvas_ensureGraphics() AS Nothing
  IF NOT __CANVAS_GFX_READY THEN
    canvas::setSyncMode(len(os::getEnvOr("MFB_CANVAS_SYNC", "")) > 0)
    canvas::setGpuMode(len(os::getEnvOr("MFB_CANVAS_GPU", "")) > 0)
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC"#;

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
/// is set by `MFB_CANVAS_GPU`), a pipeline exists (`canvas::metalReady` /
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
/// The software path stays the default regardless: it is the oracle the GPU path is
/// measured against, so it cannot become the thing being measured.
///
/// It never returns. The wait is a real condition wait, so a static scene costs
/// nothing — no timer, no poll, no spin (`.ai/canvas-threading.md` §4: time is
/// deliberately not a redraw trigger).
///
/// It renders the *installed* scene rather than being handed one, which is what lets
/// a repaint no `present` caused — a resize, an expose — draw the right picture.
#[rustfmt::skip]
const RENDER_LOOP: &str =
r#"FUNC __canvas_renderLoop() AS Nothing
  WHILE canvas::waitForRedraw()
    __canvas_renderFrame()
    canvas::frameDone()
  END WHILE
END FUNC

FUNC __canvas_renderFrame() AS Nothing
  LET size AS Size = __canvas_surfaceSize()
  ' The geometry is built once, here, and the offsets are then handed to whichever
  ' backend draws them. It has to happen before the damage diff rather than inside the
  ' renderer: an item's damaged rectangle is its geometry's bounds, so there is no
  ' diff to compute until the geometry exists.
  LET offsets AS List OF Integer = __canvas_sceneOffsets()
  ' plan-116-H Phase 1: build the GPU draw list beside the software one. Nothing
  ' consumes it yet -- both backends still decline a group scene -- but it is built every
  ' frame so `MFB_CANVAS_STATS` can report it, which is the only way a test can see a
  ' structure that exists on the graphics thread and is handed straight to an emitter.
  '
  ' Cheap despite walking the tree a second time: both walks resolve geometry through
  ' `__canvas_geometryFor`, which IS the cache, so the second one hits it.
  LET draws AS List OF Integer = __canvas_sceneDraws()
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
  IF len(damage) = 0 THEN
    ' Nothing changed, so nothing is presented. The loop still calls `frameDone`, so a
    ' `present` waiting under MFB_CANVAS_SYNC is released -- a skipped frame is a frame
    ' that finished, not one that was lost.
    __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
    __canvas_writeStats()
    RETURN
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
      RETURN
    END IF
  END IF
  IF canvas::useGpu() AND canvas::vulkanReady() THEN
    IF __canvas_renderVulkan(offsets, size.width, size.height) THEN
      RETURN
    END IF
  END IF
  __canvas_renderScene(offsets, damage, size.width, size.height)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_helper(RegistryHelper::always(
        "canvas_ensureGraphics",
        ENSURE_GRAPHICS,
    ));
    pkg.add_helper(RegistryHelper::always("canvas_renderLoop", RENDER_LOOP));
    pkg.add_helper(RegistryHelper::always("canvas_hashScene", HASH_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderScene", RENDER_SCENE));
    pkg.add_helper(RegistryHelper::always("canvas_renderMetal", RENDER_METAL));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::{
        CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE, CANVAS_DRAW_ENTRY_SHIFT,
        CANVAS_DRAW_ENTRY_WORDS, CANVAS_MAX_FRAME_ITEMS, GEO_KIND_POLYGON, GEO_KIND_TEXT,
        HEADER_AUX0, MAX_EDGES,
        MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_EDGES, METAL_MAX_GLYPH_SAMPLES,
        VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    };

    /// The body of a `FUNC`/`SUB` in the injected MFBASIC source, by name.
    fn body(name: &str) -> &'static str {
        let start = RENDER_METAL
            .find(&format!("__canvas_{name}("))
            .unwrap_or_else(|| panic!("__canvas_{name} is not in RENDER_METAL"));
        let rest = &RENDER_METAL[start..];
        let end = rest
            .find("\nEND ")
            .unwrap_or_else(|| panic!("__canvas_{name} has no END"));
        &rest[..end]
    }

    /// The draw list is BUILT here in MFBASIC and WALKED by the native emitters, so its
    /// entry width lives on both sides of a boundary the Rust compiler cannot see across
    /// -- this side is a `&str`.
    ///
    /// plan-116-H13: these two disagreed (the emitter at eight words, this source still
    /// appending four) and the result was not a wrong picture. The emitter strode 64
    /// bytes through a 32-byte array, so it read every other entry and took the following
    /// entry's `base` as a blend mode -- indexing the pipeline table out of range and
    /// handing Vulkan a junk `VkPipeline`. It SIGSEGVs inside the driver's JIT-compiled
    /// code with no MFBASIC frame in the backtrace, on one remote box, in a harness that
    /// is not part of `cargo test`. This assertion is the only cheap way to catch it.
    #[test]
    fn the_draw_entry_width_agrees_with_the_emitter() {
        let appends = body("pushOneDraw").matches("collections::append(out,").count();
        assert_eq!(
            appends, CANVAS_DRAW_ENTRY_WORDS,
            "__canvas_pushOneDraw appends {appends} words but the emitter strides \
             CANVAS_DRAW_ENTRY_WORDS = {CANVAS_DRAW_ENTRY_WORDS}",
        );
        assert_eq!(
            1usize << CANVAS_DRAW_ENTRY_SHIFT,
            CANVAS_DRAW_ENTRY_WORDS * 8,
            "the byte stride shift and the word count disagree",
        );
        assert_eq!(
            1usize << CANVAS_DRAW_ENTRY_COUNT_SHIFT,
            CANVAS_DRAW_ENTRY_WORDS,
            "the element-count shift and the word count disagree",
        );
        assert!(
            CANVAS_DRAW_ENTRY_MODE < CANVAS_DRAW_ENTRY_WORDS * 8,
            "the blend mode is read from outside the entry",
        );
    }

    /// `__canvas_blockInstances` predicts how many instances each block publishes, and
    /// the draw list's bases are its running sum -- but what is actually WRITTEN into the
    /// item buffer is `emit_split_or_publish`'s decision. One case that disagrees shifts
    /// every base after it, and the final entry then draws instances that were never
    /// published: uninitialised buffer, which reaches the screen as opaque black.
    ///
    /// That is why the symptom appears at the END of a scene rather than at the item that
    /// disagreed -- plan-116-H13 chased it through the shaders, the draw list and the ABI
    /// before finding it here. The blend split is the case that went missing, so it is
    /// the case pinned by name.
    #[test]
    fn block_instances_keeps_the_blend_split_case() {
        let rule = body("blockInstances");
        assert!(
            rule.contains("RETURN 2"),
            "the blend split case is gone: an item that both strokes and fills under a \
             non-Normal blend mode publishes TWO records in emit_split_or_publish, and \
             this function must predict 2 for it",
        );
        for (slot, what) in [("26", "blend mode"), ("7", "strokeHalf"), ("11", "fill alpha")] {
            assert!(
                rule.contains(&format!(", {slot})")),
                "the split rule no longer reads slot {slot} ({what}); \
                 emit_split_or_publish tests all three",
            );
        }
        assert!(
            rule.contains(", 20)"),
            "a text run's instance count is HEADER_AUX0 = slot 20, the field \
             emit_glyph_publish loops over",
        );
    }

    /// Find `LET <name> AS Integer = <n>` in the injected MFBASIC source.
    fn declared(name: &str) -> usize {
        let needle = format!("LET {name} AS Integer = ");
        let start = RENDER_METAL
            .find(&needle)
            .unwrap_or_else(|| panic!("{name} is not declared in RENDER_METAL"))
            + needle.len();
        let rest = &RENDER_METAL[start..];
        let end = rest.find('\n').unwrap_or(rest.len());
        rest[..end].trim().parse().expect("a decimal literal")
    }

    /// The predicates are written in MFBASIC and the emitters in Rust, so the limits
    /// exist twice — with no compiler between them. Drift is not a style problem: if
    /// the MFBASIC cap ever exceeds the Rust one, the predicate admits a scene the
    /// emitter's buffer cannot hold.
    ///
    /// Since plan-116-A every one of these caps guards a *heap* buffer written through
    /// a mapped pointer (the frame buffer), where Metal's edges used to be a stack
    /// array. That makes drift worse rather than better: a stack overrun tends to fault
    /// near the frame that caused it, and a write past a mapped GPU buffer lands in
    /// whatever the driver happened to put next.
    #[test]
    fn the_two_gpu_edge_budgets_match_the_emitters() {
        assert_eq!(declared("__CANVAS_METAL_MAX_EDGES"), MAX_EDGES);
        assert_eq!(
            declared("__CANVAS_VULKAN_MAX_FRAME_EDGES"),
            VULKAN_MAX_FRAME_EDGES
        );
        assert_eq!(
            declared("__CANVAS_METAL_MAX_FRAME_EDGES"),
            METAL_MAX_FRAME_EDGES,
            "the predicate admits a frame whose polygon edges the Metal frame buffer's \
             edge region cannot hold",
        );
        assert_eq!(
            declared("__CANVAS_MAX_FRAME_ITEMS"),
            CANVAS_MAX_FRAME_ITEMS,
            "the predicate admits a frame with more drawn quads than the item buffer \
             has blocks — on BOTH backends, since they share this one. The emitters \
             stop publishing at capacity, so the surplus items would silently not be \
             drawn",
        );
        assert_eq!(
            declared("__CANVAS_VULKAN_MAX_GLYPH_SAMPLES"),
            VULKAN_MAX_FRAME_GLYPH_SAMPLES,
            "the predicate admits a frame whose glyph bitmaps the buffer's glyph \
             region cannot hold",
        );
        assert_eq!(
            declared("__CANVAS_METAL_MAX_GLYPH_SAMPLES"),
            METAL_MAX_GLYPH_SAMPLES,
            "the predicate admits a glyph bigger than `setFragmentBytes:` will carry",
        );
        // plan-116-F. One number for both backends, because the gradient region is
        // sized identically on each -- and the failure it prevents is worse than a
        // dropped item: past the cap an item's first-stop index runs off the region,
        // and the shader reads whatever the buffer holds there as a colour ramp.
        assert_eq!(
            declared("__CANVAS_MAX_FRAME_GRADIENT_STOPS"),
            MAX_FRAME_GRADIENT_STOPS,
            "the predicate admits a frame whose gradient stops the buffer's third \
             region cannot hold, so one item's stops would be read as another's",
        );
    }

    /// The predicates read the geometry header by slot. `offset + 20` is
    /// `HEADER_AUX0` — the polygon's edge count, and for a glyph run its glyph count —
    /// and a renumbered header would leave them summing an arc's start angle instead,
    /// which is a plausible-looking number rather than an error.
    ///
    /// The count is a census, so it moves whenever the predicates gain or lose a read;
    /// it went 4 → 7 in plan-116-A. Enumerated so the next reader can tell a legitimate
    /// growth from a slot that drifted:
    ///
    /// | | Metal | Vulkan |
    /// |---|---|---|
    /// | the glyph-run walk (`__canvas_runLargestGlyph` / `__canvas_runSamples`) | 1 | 1 |
    /// | the per-item `MAX_EDGES` decline | 1 | — (no per-item limit) |
    /// | the frame edge sum | 1 (new) | 1 |
    /// | the frame quad count, a glyph run's glyphs | 1 (new) | 1 (new) |
    #[test]
    fn the_predicates_read_the_edge_count_slot() {
        assert_eq!(HEADER_AUX0, 20);
        assert_eq!(
            RENDER_METAL
                .matches(&format!("offset + {HEADER_AUX0}"))
                .count(),
            7,
            "every glyph-run walk, edge sum, edge decline and quad count in both \
             predicates should read HEADER_AUX0"
        );
    }

    /// A glyph run is a kind the predicates now *admit* rather than refuse, so its
    /// spelling has to be as pinned as the polygon's — and by the same argument. It
    /// was not always: both predicates declined `__CANVAS_GEO_TEXT` outright until
    /// the backends could draw one, and the version before *that* accepted it while
    /// neither shader knew the kind, returning a frame with the text missing and
    /// calling it success.
    #[test]
    fn the_text_kind_is_spelled_once() {
        assert_eq!(GEO_KIND_TEXT, "6");
        assert!(RENDER_METAL.contains("= __CANVAS_GEO_TEXT THEN"));
    }

    /// Both predicates test the same kind the emitters and the shaders branch on.
    #[test]
    fn the_polygon_kind_is_spelled_once() {
        assert_eq!(GEO_KIND_POLYGON, "4");
        assert!(RENDER_METAL.contains("= __CANVAS_GEO_POLYGON THEN"));
    }
}
