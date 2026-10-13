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
    CANVAS_DRAW_ENTRY_WORDS, CANVAS_DRAW_PASS_CONTINUES, CANVAS_DRAW_PASS_MORE,
    CANVAS_DRAW_PASS_WORD, METAL_MAX_FRAME_DASH_WORDS, METAL_MAX_FRAME_EDGES,
    METAL_MAX_FRAME_GLYPH_SAMPLES, METAL_MAX_FRAME_GRADIENT_STOPS, METAL_MAX_FRAME_ITEMS,
    VULKAN_MAX_FRAME_DASH_WORDS, VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    VULKAN_MAX_FRAME_GRADIENT_STOPS, VULKAN_MAX_FRAME_ITEMS,
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
    ' plan-172-G: the entry's group matrix, current for the bounds test and the draw.
    __canvas_enterGroupMatrix(collections::getOr(__CANVAS_DRAW_MAT, di, 0))
    IF full OR __canvas_boundsMeetOffset(offset, damage, gdx, gdy) THEN
      buffer = __canvas_drawGeometry(buffer, width, height, offset, gdx, gdy)
    END IF
    di = di + 1
  END WHILE
  __canvas_enterGroupMatrix(0)
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
/// `__canvas_metalRenderable` and `__canvas_vulkanRenderable` are the honesty gate, each
/// `__canvas_framePasses` with its backend's region sizes: a backend draws a frame only
/// when it reproduces every item of it, and hands the rest back to the oracle rather
/// than drawing it wrongly.
///
/// Phase 2's SDF fragment shader evaluates the same distance functions the software
/// rasteriser does, so every kind passes — including `__CANVAS_GEO_NONE`, which both
/// backends draw as nothing. What a backend cannot do is hold more than its frame
/// BUFFER holds: quads, polygon edges, gradient stops, dash words and glyph/picture
/// texels each take a region of it. That is the size of one PASS and not of a frame
/// (bug-765): a frame whose blocks do not fit is drawn as consecutive runs of its draw
/// list (`__canvas_passNext`), each the longest that fits every region, into one target
/// that the first clears and the rest load, and is presented or read back once. The one
/// frame still declined is one holding a block that does not fit a region by itself.
/// There is no per-polygon edge cap since bug-686: the old one (256 edges, justified by
/// a 4 KB `setFragmentBytes:` payload that plan-116-A removed) sent every detailed
/// polygon to software, and its twin in the emitter drew such a polygon as nothing.
///
/// It reads the geometry header by slot rather than through a helper because that is
/// what the header is for: a fixed layout both backends index directly
/// (`__canvas_headerFor`).
///
/// One sum serves both backends, handed the region sizes of the one drawing the
/// frame. They were two predicates' own sums while the backends' limits differed in KIND (Metal
/// capped each polygon, Vulkan their sum); since bug-688 both keep every per-item
/// payload in a region of one buffer, of the same sizes, and two copies of one sum were
/// two places for a count to differ from the emitter's.
///
/// **A glyph run is bounded rather than refused**: each glyph's bitmap is copied into
/// a region of the buffer and its block names its slice (bug-670 moved Metal onto
/// Vulkan's design — its per-glyph `setFragmentBytes:` payload, capped at 4 KiB, was
/// overwritten before the instanced draw read it). Neither backend truncates: a clipped
/// glyph is a different glyph and would read as a rasteriser bug.
///
/// A kind a shader does not know must be declined HERE, not discovered in the emitter.
/// Both backends declined `__CANVAS_GEO_TEXT` outright for as long as neither could
/// draw one, and that was not caution — the version before it *accepted* a kind neither
/// shader knew, and Metal returned a frame with the text simply missing and no error
/// anywhere: 4,536 pixels wrong, reported as success.
///
/// The region sizes below are not literals in this text: each `LET __CANVAS_*MAX*`
/// line carries an `@NAME@` token that `RENDER_METAL` replaces with the Rust constant
/// of that name (`RENDER_CAPS`), so the pass walk and the emitter it fills a buffer for
/// read ONE definition (bug-686). They used to be two, joined only by a test.
#[rustfmt::skip]
const RENDER_METAL_TEMPLATE: &str =
r#"' plan-116-G: the accumulated group translation of each draw entry, parallel to the
' offsets list `__canvas_sceneOffsets` returns, and whether any group was expanded.
'
' Parallel globals rather than a widened return, because that return is consumed by
' the render walk and the damage pass, which index it one entry per item. Widening it
' to a strided record would touch both to express something neither reads from it.
MUT __CANVAS_DRAW_DX AS List OF Float = []
MUT __CANVAS_DRAW_DY AS List OF Float = []

' plan-172-G: the matrix of each draw entry, parallel to the two lists above: 0 for an
' entry its groups only translate, otherwise one more than its row in
' `__CANVAS_GROUP_ROWS`. `__CANVAS_DRAW_DX` / `_DY` stay the TRANSLATION of the composed
' affine in both cases, so a consumer that knows nothing of matrices still places a
' translate-only entry exactly as it did.
'
' A row is eight floats: the composed linear part forward (`a`, `b`, `c`, `d`), then its
' inverse. One row per group node that has a matrix, per frame; an index rather than
' six floats an entry because `canvas::sceneLayout` moves one word per entry per list.
MUT __CANVAS_DRAW_MAT AS List OF Integer = []
MUT __CANVAS_GROUP_ROWS AS List OF Float = []

' `__canvas_composeGroup`'s second answer: the composed translation.
MUT __CANVAS_COMPOSED_X AS Float = 0.0
MUT __CANVAS_COMPOSED_Y AS Float = 0.0

' A new row for the linear part `a b c d`, answering its matrix number (row + 1) -- or
' -1 when it has no area, which a composed affine can reach only by scaling something
' out of existence, and such a group draws nothing.
FUNC __canvas_groupRow(a AS Float, b AS Float, c AS Float, d AS Float) AS Integer
  LET det AS Float = a * d - b * c
  IF NOT (__canvas_absF(det) >= 0.000000000001) THEN
    RETURN 0 - 1
  END IF
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, a)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, b)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, c)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, d)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, d / det)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, (0.0 - b) / det)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, (0.0 - c) / det)
  __CANVAS_GROUP_ROWS = collections::append(__CANVAS_GROUP_ROWS, a / det)
  RETURN len(__CANVAS_GROUP_ROWS) / 8
END FUNC

' plan-172-G: one group node under its parents. The parents' composed affine arrives as
' its translation (`gdx`, `gdy`) and its matrix number (`mat`, 0 for none); the node is
' `T(position) . transform`. The answer is the composed matrix number, with the composed
' translation left in `__CANVAS_COMPOSED_X` / `_Y`.
'
' Both walks call this -- the software and damage walk (`__canvas_appendDraw`) and the
' GPU walk (`__canvas_drawGroup`) -- so they cannot disagree about what a nested group
' means.
'
' A node whose transform is not in effect (all zero, or no area: the two cases
' `__canvas_transformActive` answers FALSE for) only translates. Under translate-only
' parents that is the sum this walk always computed, `gdx + position.x`, to the bit --
' which is what keeps every group scene written before this letter drawing the same
' frame. Under a parent with a matrix, the node's translation is carried through it.
FUNC __canvas_composeGroup(gdx AS Float, gdy AS Float, mat AS Integer, position AS vector::Float2, t AS Transform) AS Integer
  IF mat < 0 THEN
    RETURN mat
  END IF
  IF mat = 0 THEN
    IF NOT __canvas_transformActive(t) THEN
      __CANVAS_COMPOSED_X = gdx + position.x
      __CANVAS_COMPOSED_Y = gdy + position.y
      RETURN 0
    END IF
    __CANVAS_COMPOSED_X = gdx + (position.x + t.tx)
    __CANVAS_COMPOSED_Y = gdy + (position.y + t.ty)
    RETURN __canvas_groupRow(t.a, t.b, t.c, t.d)
  END IF
  LET row AS Integer = (mat - 1) * 8
  LET pa AS Float = collections::getOr(__CANVAS_GROUP_ROWS, row, 1.0)
  LET pb AS Float = collections::getOr(__CANVAS_GROUP_ROWS, row + 1, 0.0)
  LET pc AS Float = collections::getOr(__CANVAS_GROUP_ROWS, row + 2, 0.0)
  LET pd AS Float = collections::getOr(__CANVAS_GROUP_ROWS, row + 3, 1.0)
  IF NOT __canvas_transformActive(t) THEN
    __CANVAS_COMPOSED_X = pa * position.x + pc * position.y + gdx
    __CANVAS_COMPOSED_Y = pb * position.x + pd * position.y + gdy
    RETURN mat
  END IF
  LET nx AS Float = position.x + t.tx
  LET ny AS Float = position.y + t.ty
  __CANVAS_COMPOSED_X = pa * nx + pc * ny + gdx
  __CANVAS_COMPOSED_Y = pb * nx + pd * ny + gdy
  RETURN __canvas_groupRow(pa * t.a + pc * t.b, pb * t.a + pd * t.b, pa * t.c + pc * t.d, pb * t.c + pd * t.d)
END FUNC

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
'
' plan-172-K: `path` and `index` say WHERE the item is -- `index` is its place in the
' list that holds it, and `path` names that list: 0 for the scene itself, otherwise
' `__canvas_autoPath` of the group node that was expanded. Nothing here reads them; they
' are handed on, and an `AutoPaint` is told them (`__canvas_autoAppend`), because its
' state belongs to that place and not to its contents or its `id`.
FUNC __canvas_appendDraw(offsets AS List OF Integer, item AS DrawItem, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
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
      ' plan-172-G: this node under its parents. A composed affine with no area draws
      ' nothing, so its children are not walked.
      LET childMat AS Integer = __canvas_composeGroup(gdx, gdy, mat, g.position, g.transform)
      IF childMat < 0 THEN
        RETURN out
      END IF
      LET childX AS Float = __CANVAS_COMPOSED_X
      LET childY AS Float = __CANVAS_COMPOSED_Y
      LET slot AS Integer = canvas::groupResolve(g.name)
      LET childPath AS Integer = __canvas_autoPath(path, index, slot)
      MUT k AS Integer = 0
      FOR EACH child IN canvas::groupItems(slot)
        out = __canvas_appendDraw(out, child, __canvas_hashItem(child), childX, childY, childMat, depth + 1, childPath, k)
        k = k + 1
      NEXT
      RETURN out
    ' plan-172-K: an `AutoPaint` draws its shape, and is noted as an instance at this
    ' place for the frame's resolve (`helper_autopaint.rs`). The entry appended here is
    ' the shape with its OWN paint -- what the hit test reads; the resolve swaps in a
    ' state's afterwards.
    CASE AutoPaint(a)
      RETURN __canvas_autoAppend(out, a, hash, gdx, gdy, mat, depth, path, index)
    ' plan-172-L: a `Picture` is the entries its shape and image make -- one, two or
    ' none (`helper_picture.rs`).
    CASE Picture(pic)
      RETURN __canvas_pictureAppend(out, pic, hash, gdx, gdy, mat, depth, path, index)
    ' plan-150-A: a `ParticleSystem` is the entries its live particles make -- any
    ' number, each an ordinary shape that comes back through this function
    ' (`helper_particles.rs`).
    CASE ParticleSystem(ps)
      RETURN __canvas_particleAppend(out, ps, gdx, gdy, mat, depth, path, index)
    CASE ELSE
      LET offset AS Integer = __canvas_geometryFor(item, hash)
      out = collections::append(out, offset)
      __canvas_noteEntry(offset, hash, gdx, gdy, mat)
      RETURN out
  END MATCH
END FUNC

' What goes beside one draw entry -- the record at `offset`, appended by the caller --
' in the lists parallel to the draw list: the groups' composed offset and matrix, and
' the entry's draw hash, folded from `hash`.
FUNC __canvas_noteEntry(offset AS Integer, hash AS Integer, gdx AS Float, gdy AS Float, mat AS Integer) AS Nothing
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
  drawHash = __canvas_hashFloat(__canvas_hashFloat(drawHash, gdx), gdy)
  ' plan-172-G: and the matrix, for the same reason as the offset -- a group turned
  ' in place changes no geometry and no offset. Only an entry that has one folds it
  ' in, so a translate-only entry's hash is what it was, and what
  ' `canvas::sceneLayout` computes natively for an item outside any group.
  __CANVAS_DRAW_MAT = collections::append(__CANVAS_DRAW_MAT, mat)
  IF mat > 0 THEN
    LET row AS Integer = (mat - 1) * 8
    drawHash = __canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(__canvas_hashFloat(drawHash, collections::getOr(__CANVAS_GROUP_ROWS, row, 1.0)), collections::getOr(__CANVAS_GROUP_ROWS, row + 1, 0.0)), collections::getOr(__CANVAS_GROUP_ROWS, row + 2, 0.0)), collections::getOr(__CANVAS_GROUP_ROWS, row + 3, 1.0))
  END IF
  __CANVAS_DRAW_HASHES = collections::append(__CANVAS_DRAW_HASHES, drawHash)
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
      ' plan-150-A: a particle system adds nothing to the signature -- it names no
      ' group -- but this is the walk with a user call to fail back to, so it is where
      ' the one system that cannot be drawn is refused: a particle's place in its life
      ' is `age / lifetime`, and there is no such number for a lifetime of zero or less.
      ' One inside a group is reached by the recursion above.
      '
      ' 77050028 is errorCode.ErrCanvasParticleLifetime. The literal rather than the
      ' name because package source does not import `errorCode`. Written here and not
      ' behind a hook of the particle chunk because this is the unconditional source's
      ' own raise: `present` declares it for every program.
      CASE ParticleSystem(ps)
        IF NOT (ps.emission.lifetime > 0.0) THEN
          FAIL error(77050028, "canvas particle system lifetime must be above zero")
        END IF
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
'   2. It bounds what reuse costs. The quad cap was 4096 when this was written; a UI
'      drawing one 200-item panel at thirty positions costs 200 blocks under sharing and
'      6000 under duplication -- over that cap, so the frame would have declined to
'      software and the feature been slowest exactly where it is most used.
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
' geometry offset, or -1 for a `Group` node -- or (plan-172-L) -2 for any other item
' that is not exactly one entry, a `Picture` of two or of none. `__canvas_sceneDraws`
' lays the draw list out from this instead of resolving every item a second time.
MUT __CANVAS_TOP_OFFSETS AS List OF Integer = []
' plan-172-L: for a frame that holds a -2, the frame's draw entries and the spans
' `canvas::sceneLayout` was given -- (scene index, first entry in the walk's list, entry
' count) for every index the walk expanded -- from which `__canvas_sceneDrawsWalk` finds
' that item's entries. It lays THOSE out, and does not ask the item again: an image's
' pixels or its closing may have changed since the entries were made, and a frame
' draws from one reading of each (`.ai/canvas-threading.md` section 7).
MUT __CANVAS_FRAME_ENTRIES AS List OF Integer = []
MUT __CANVAS_FRAME_SPANS AS List OF Integer = []
' The scene's items, in the same order -- fetched only on a frame where some item could
' not be resolved from its hash alone (a miss, a picture, a group), and empty otherwise.
' A frame whose items are all cached never copies the scene out of the ring.
MUT __CANVAS_FRAME_ITEMS AS List OF DrawItem = []
MUT __CANVAS_FRAME_ITEMS_READY AS Boolean = FALSE
' bug-686: the frame's ONE view of the installed scene, taken by `canvas::sceneSnapshot`
' at the top of `__canvas_sceneOffsets` -- raw pointers into the worker's arena, which is
' why they are Integers: nothing here owns them. Every reader in the frame goes through
' them (natively) or through copies of them (`canvas::snapshotItems` and friends).
MUT __CANVAS_SNAP_ITEMS AS Integer = 0
MUT __CANVAS_SNAP_LAYERS AS Integer = 0
MUT __CANVAS_SNAP_HASHES AS Integer = 0
' The frame's item hashes, in scene order: the ones published for the snapshot's scene,
' or -- when the snapshot caught a scene whose hashes are not published yet -- computed
' here from its items.
MUT __CANVAS_FRAME_HASHES AS List OF Integer = []
' Frames that hashed their own scene because its hashes were not published yet --
' `snapshotRehashed=` on the `--debug` stats line.
MUT __CANVAS_SNAP_REHASHED AS Integer = 0
' Slot index -> the base of that group's own run in `__CANVAS_DRAW_BLOCKS`, and its
' length. Parallel lists rather than a Map because a Map of Integer to Integer would
' allocate per frame and this is walked once per group node.
MUT __CANVAS_DRAW_MEMO_SLOT AS List OF Integer = []
MUT __CANVAS_DRAW_MEMO_BASE AS List OF Integer = []
MUT __CANVAS_DRAW_MEMO_COUNT AS List OF Integer = []
' plan-172-K: which VARIANT of the group's run a row holds. 0 is the run every use
' shares. A use in which some `AutoPaint` of the group is showing a state paint cannot
' share it -- its blocks differ -- so it gets a row of its own, named by
' `__canvas_autoVariant` of that use.
MUT __CANVAS_DRAW_MEMO_VARIANT AS List OF Integer = []

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
  ' the stroke switched off, then stroke with the fill marked off -- because one
  ' blended draw cannot composite the two against each other correctly. The three
  ' conditions are `emit_split_or_publish`'s, in its order: a non-Normal blend mode,
  ' a positive strokeHalf, and a fill -- a non-zero fill alpha, or (plan-163-D) a
  ' gradient or a picture, whose colour does not come from the fill alpha. A Line or an
  ' Arc arrives here fill-only with a negative strokeHalf (`__canvas_strokeAsFill`) and
  ' takes the single-record path.
  IF toInt(__canvas_geoAt(offset, 26)) <> 0 THEN
    IF __canvas_geoAt(offset, 7) > 0.0 THEN
      IF toInt(__canvas_geoAt(offset, 11)) > 0 OR toInt(__canvas_geoAt(offset, __CANVAS_GEO_GRADIENT_COUNT)) >= 2 OR toInt(__canvas_geoAt(offset, 0)) = __CANVAS_GEO_PICTURE THEN
        RETURN 2
      END IF
    END IF
  END IF
  RETURN 1
END FUNC

FUNC __canvas_memoLookup(slot AS Integer, variant AS Integer) AS Integer
  MUT i AS Integer = 0
  WHILE i < len(__CANVAS_DRAW_MEMO_SLOT)
    IF collections::getOr(__CANVAS_DRAW_MEMO_SLOT, i, -1) = slot AND collections::getOr(__CANVAS_DRAW_MEMO_VARIANT, i, 0) = variant THEN
      RETURN i
    END IF
    i = i + 1
  END WHILE
  RETURN -1
END FUNC

' Append one draw entry: offsets in 16.16 so the whole list stays `List OF Integer`
' and reaches an emitter without a second parallel Float list -- each with its residual
' word (bug-735), so the shaders recover the offset the oracle subtracts to 2^-47 px.
FUNC __canvas_pushOneDraw(base AS Integer, count AS Integer, dx AS Float, dy AS Float, mat AS Integer) AS Nothing
  IF count <= 0 THEN
    RETURN
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
    RETURN
  END IF
  ' SIXTEEN words per entry (plan-172-G; eight before it, four before that). The emitter
  ' addresses an entry with a shift, so the stride has to be a power of two; the trailing
  ' words are their own rather than packed into the others, because a mode smuggled into
  ' the high bits of `count` is the kind of encoding that survives exactly until someone
  ' draws 65536 instances.
  '
  ' This width is agreed in THREE places and there is no gate that checks they agree:
  ' here, `__canvas_drawsText` in helper_surface.rs, and `emit_draw_list_pass` in
  ' src/codegen/runtime/canvas/vulkan.rs (`CANVAS_DRAW_ENTRY_SHIFT`), with
  ' `canvas::sceneDrawsFlat` writing the same width natively.
  ' When they disagreed -- this function writing four words while the emitter strode by eight --
  ' the emitter read every other entry and took the following entry's `base` as a blend
  ' mode, which indexes the pipeline table out of range and hands Vulkan a junk
  ' VkPipeline. That does not fail cleanly: it SIGSEGVs inside the driver's JIT-compiled
  ' code, with a backtrace containing no MFBASIC frame at all.
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instBase)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instCount)
  LET wordX AS Integer = toInt(dx * 65536.0)
  LET wordY AS Integer = toInt(dy * 65536.0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, wordX)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, wordY)
  ' The BlendMode every block in this run shares. It travels with the entry because the
  ' pipeline is bound per draw and the draws are issued in a second pass, so a binding
  ' left in the publish walk would apply the LAST item's mode to the whole frame.
  LET mode AS Integer = toInt(__canvas_geoAt(collections::getOr(__CANVAS_DRAW_BLOCKS, base, 0), 26))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, mode)
  ' bug-735: words 5 and 6 are the offsets' residual words, `(d * 65536 - word) * 2^31`
  ' in units of 2^-47 px (`EDGE_RESIDUAL_SCALE`) -- the 16.16 word alone is up to 2^-16
  ' px from the offset the software rasteriser subtracts, and that moved a coverage step
  ' of every grouped item near black. `d * 65536 - word` is exact (the two are within one
  ' of each other) and under 1 in magnitude, so the residual fits the 32-bit word the
  ' emitters store. A frame with no group lays its entries out natively
  ' (`canvas::sceneDrawsFlat`) at offset 0, residuals 0.
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, toInt((dx * 65536.0 - toFloat(wordX)) * 2147483648.0))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, toInt((dy * 65536.0 - toFloat(wordY)) * 2147483648.0))
  ' plan-172-G: word 7 is `hasMatrix`, words 8-11 the inverse linear part of the groups'
  ' composed affine as float32 bits, words 12-15 reserved. The translation stays in
  ' words 2-3 and 5-6 either way.
  MUT hasMatrix AS Integer = 0
  MUT inverseA AS Integer = 0
  MUT inverseB AS Integer = 0
  MUT inverseC AS Integer = 0
  MUT inverseD AS Integer = 0
  IF mat > 0 THEN
    LET row AS Integer = (mat - 1) * 8
    hasMatrix = 1
    inverseA = __canvas_float32Bits(collections::getOr(__CANVAS_GROUP_ROWS, row + 4, 1.0))
    inverseB = __canvas_float32Bits(collections::getOr(__CANVAS_GROUP_ROWS, row + 5, 0.0))
    inverseC = __canvas_float32Bits(collections::getOr(__CANVAS_GROUP_ROWS, row + 6, 0.0))
    inverseD = __canvas_float32Bits(collections::getOr(__CANVAS_GROUP_ROWS, row + 7, 1.0))
  END IF
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, hasMatrix)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, inverseA)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, inverseB)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, inverseC)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, inverseD)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
END FUNC

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
FUNC __canvas_pushDraw(base AS Integer, count AS Integer, dx AS Float, dy AS Float, mat AS Integer) AS Nothing
  IF count <= 0 THEN
    RETURN
  END IF
  MUT runStart AS Integer = base
  MUT i AS Integer = base + 1
  WHILE i < base + count
    LET prev AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i - 1, 0)
    LET here AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i, 0)
    IF NOT __canvas_drawsJoin(prev, here) THEN
      __canvas_pushOneDraw(runStart, i - runStart, dx, dy, mat)
      runStart = i
    END IF
    i = i + 1
  END WHILE
  __canvas_pushOneDraw(runStart, base + count - runStart, dx, dy, mat)
END FUNC

' One more block of the frame's block list, and the instance it starts at.
FUNC __canvas_pushBlock(offset AS Integer) AS Nothing
  __CANVAS_DRAW_BLOCKS = collections::append(__CANVAS_DRAW_BLOCKS, offset)
  __CANVAS_DRAW_INST = collections::append(__CANVAS_DRAW_INST, __CANVAS_DRAW_NEXT_INST)
  __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(offset)
END FUNC

' Lay out one group's own (non-group) items once, returning its memo index. `path` is
' this USE of the group (`__canvas_autoPath`), which only an `AutoPaint` child reads.
FUNC __canvas_memoGroup(slot AS Integer, hashes AS List OF Integer, depth AS Integer, path AS Integer) AS Integer
  LET variant AS Integer = __canvas_autoVariant(path)
  LET found AS Integer = __canvas_memoLookup(slot, variant)
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
  __CANVAS_DRAW_MEMO_VARIANT = collections::append(__CANVAS_DRAW_MEMO_VARIANT, variant)
  LET row AS Integer = len(__CANVAS_DRAW_MEMO_SLOT) - 1

  MUT count AS Integer = 0
  MUT k AS Integer = 0
  FOR EACH child IN canvas::groupItems(slot)
    MATCH child
      CASE Group(g)
        ' A nested group is NOT part of this group's own run -- it gets its own memoised
        ' run and its own draw entry, at the composed offset.
        LET nested AS Integer = 0
      ' plan-172-K: an `AutoPaint` child is one block -- its shape under the paint this
      ' use of the group is showing, the record the frame's resolve put in the draw
      ' entries, so the two walks name one record.
      CASE AutoPaint(a)
        LET autoOffset AS Integer = __canvas_autoBlock(a, __canvas_hashItem(child), path, k)
        __CANVAS_DRAW_BLOCKS = collections::append(__CANVAS_DRAW_BLOCKS, autoOffset)
        __CANVAS_DRAW_INST = collections::append(__CANVAS_DRAW_INST, __CANVAS_DRAW_NEXT_INST)
        __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(autoOffset)
        count = count + 1
      ' plan-172-L: a `Picture` child is a block for each record it draws from -- the
      ' ones `__canvas_pictureAppend` made its entries of.
      CASE Picture(pic)
        LET picBlocks AS List OF Integer = __canvas_pictureBlocks(pic, __canvas_hashItem(child), path, k)
        MUT b AS Integer = 0
        WHILE b + 1 < len(picBlocks)
          __canvas_pushBlock(collections::getOr(picBlocks, b, 0))
          count = count + 1
          b = b + 2
        END WHILE
      ' plan-150-A: a `ParticleSystem` child is a block per live particle.
      CASE ParticleSystem(ps)
        count = count + __canvas_particleBlocks(ps)
      CASE ELSE
        LET childOffset AS Integer = __canvas_geometryFor(child, __canvas_hashItem(child))
        __CANVAS_DRAW_BLOCKS = collections::append(__CANVAS_DRAW_BLOCKS, childOffset)
        __CANVAS_DRAW_INST = collections::append(__CANVAS_DRAW_INST, __CANVAS_DRAW_NEXT_INST)
        __CANVAS_DRAW_NEXT_INST = __CANVAS_DRAW_NEXT_INST + __canvas_blockInstances(childOffset)
        count = count + 1
    END MATCH
    k = k + 1
  NEXT
  __CANVAS_DRAW_MEMO_COUNT = collections::set(__CANVAS_DRAW_MEMO_COUNT, row, count)
  RETURN row
END FUNC

' Emit the draw entries for one group reference at an accumulated offset: its own run,
' then its nested groups recursively.
FUNC __canvas_drawGroup(slot AS Integer, hashes AS List OF Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer) AS Nothing
  IF depth >= __CANVAS_GROUP_MAX_DEPTH THEN
    RETURN
  END IF
  LET row AS Integer = __canvas_memoGroup(slot, hashes, depth, path)
  __canvas_pushDraw(collections::getOr(__CANVAS_DRAW_MEMO_BASE, row, 0), collections::getOr(__CANVAS_DRAW_MEMO_COUNT, row, 0), gdx, gdy, mat)
  MUT k AS Integer = 0
  FOR EACH child IN canvas::groupItems(slot)
    MATCH child
      CASE Group(g)
        ' plan-172-G: the same composition the software walk makes.
        LET childMat AS Integer = __canvas_composeGroup(gdx, gdy, mat, g.position, g.transform)
        IF childMat >= 0 THEN
          LET childX AS Float = __CANVAS_COMPOSED_X
          LET childY AS Float = __CANVAS_COMPOSED_Y
          LET nested AS Integer = canvas::groupResolve(g.name)
          ' plan-172-K: the path `__canvas_appendDraw` gave the same node.
          __canvas_drawGroup(nested, hashes, childX, childY, childMat, depth + 1, __canvas_autoPath(path, k, nested))
        END IF
      CASE ELSE
        ' A non-group child contributes no draw entry of its own: its block is already
        ' inside the run this group's memo laid out.
        LET skipped AS Integer = 0
    END MATCH
    k = k + 1
  NEXT
END FUNC

FUNC __canvas_sceneDraws() AS List OF Integer
  ' bug-686: a frame with no group node is laid out natively, in one pass over the
  ' offsets `__canvas_sceneOffsets` resolved -- the walk below, specialised to one run at
  ' no offset. A frame with a group comes back FALSE, untouched, and takes the walk.
  IF canvas::sceneDrawsFlat() THEN
    __canvas_drawsVerify()
    RETURN __CANVAS_DRAWS
  END IF
  RETURN __canvas_sceneDrawsWalk()
END FUNC

FUNC __canvas_sceneDrawsWalk() AS List OF Integer
  __CANVAS_DRAWS = []
  __CANVAS_DRAW_BLOCKS = []
  __CANVAS_DRAW_INST = []
  __CANVAS_DRAW_NEXT_INST = 0
  __CANVAS_DRAW_MEMO_SLOT = []
  __CANVAS_DRAW_MEMO_BASE = []
  __CANVAS_DRAW_MEMO_COUNT = []
  __CANVAS_DRAW_MEMO_VARIANT = []
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
  MUT spanAt AS Integer = 0
  MUT extra AS Integer = 0
  MUT index AS Integer = 0
  WHILE index < count
    LET itemOffset AS Integer = collections::getOr(__CANVAS_TOP_OFFSETS, index, 0 - 1)
    IF itemOffset = 0 - 2 THEN
      ' plan-172-L: an item of several entries, or of none. Its blocks are its entries
      ' in the frame -- the scene index plus one less than the count of every span
      ' before it is where they start -- and they join the current run like any item's.
      WHILE spanAt * 3 < len(__CANVAS_FRAME_SPANS) AND collections::getOr(__CANVAS_FRAME_SPANS, spanAt * 3, 0) < index
        extra = extra + collections::getOr(__CANVAS_FRAME_SPANS, spanAt * 3 + 2, 0) - 1
        spanAt = spanAt + 1
      END WHILE
      IF spanAt * 3 < len(__CANVAS_FRAME_SPANS) AND collections::getOr(__CANVAS_FRAME_SPANS, spanAt * 3, 0) = index THEN
        LET first AS Integer = index + extra
        MUT b AS Integer = 0
        WHILE b < collections::getOr(__CANVAS_FRAME_SPANS, spanAt * 3 + 2, 0)
          __canvas_pushBlock(collections::getOr(__CANVAS_FRAME_ENTRIES, first + b, 0))
          IF runCount = 0 THEN
            runBase = len(__CANVAS_DRAW_BLOCKS) - 1
          END IF
          runCount = runCount + 1
          b = b + 1
        END WHILE
      END IF
    ELSEIF itemOffset < 0 THEN
      ' A group node: it ends the current run, then lays out (or reuses) its own.
      MATCH collections::getOr(__CANVAS_FRAME_ITEMS, index, none)
        CASE Group(g)
          __canvas_pushDraw(runBase, runCount, 0.0, 0.0, 0)
          runCount = 0
          LET topMat AS Integer = __canvas_composeGroup(0.0, 0.0, 0, g.position, g.transform)
          IF topMat >= 0 THEN
            LET topX AS Float = __CANVAS_COMPOSED_X
            LET topY AS Float = __CANVAS_COMPOSED_Y
            LET topSlot AS Integer = canvas::groupResolve(g.name)
            __canvas_drawGroup(topSlot, hashes, topX, topY, topMat, 0, __canvas_autoPath(0, index, topSlot))
          END IF
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
  __canvas_pushDraw(runBase, runCount, 0.0, 0.0, 0)
  RETURN __CANVAS_DRAWS
END FUNC

' A stand-in `DrawItem` for an index `__CANVAS_FRAME_ITEMS` does not hold. A group named
' by nothing resolves to no slot and draws nothing, so reaching it is harmless.
FUNC __canvas_noItem() AS DrawItem
  RETURN Group[id := "", position := vector::Float2[x := 0.0, y := 0.0], transform := __canvas_noTransform(), name := ""]
END FUNC

' Every item of the frame's scene -- the snapshot `canvas::sceneSnapshot` took (bug-686)
' -- in `__CANVAS_FRAME_HASHES` order: the flat items, then each layer's.
FUNC __canvas_flatScene() AS List OF DrawItem
  MUT out AS List OF DrawItem = canvas::snapshotItems()
  FOR EACH layer IN canvas::snapshotLayers()
    FOR EACH item IN layer.items
      out = collections::append(out, item)
    NEXT
  NEXT
  RETURN out
END FUNC

' The frame's items, copied out of the snapshot at most once a frame.
FUNC __canvas_ensureFrameItems() AS Nothing
  IF NOT __CANVAS_FRAME_ITEMS_READY THEN
    __CANVAS_FRAME_ITEMS = __canvas_flatScene()
    __CANVAS_FRAME_ITEMS_READY = TRUE
  END IF
END FUNC

FUNC __canvas_frameItems() AS List OF DrawItem
  __canvas_ensureFrameItems()
  RETURN __CANVAS_FRAME_ITEMS
END FUNC

FUNC __canvas_sceneOffsets() AS List OF Integer
  ' bug-682/bug-686: the one point in a frame where NO geometry offset is live -- the
  ' previous frame's are dead and this frame's do not exist yet -- so it is the only place
  ' the cache may drop a slot or move a float. Nothing after this point in the frame does.
  canvas::geoBeginFrame()

  ' bug-686: ONE consistent view of the installed scene for the whole frame -- its items,
  ' its layers and its hashes, all one publish's. The worker publishes the hashes in a
  ' second call after the scene, so a frame can catch a scene whose hashes are the
  ' previous scene's; a hit is trusted on its hash alone, so those are refused
  ' (`sceneSnapshot` answers FALSE) and the frame hashes its own items instead.
  __CANVAS_FRAME_ITEMS = []
  __CANVAS_FRAME_ITEMS_READY = FALSE
  IF canvas::sceneSnapshot() THEN
    __CANVAS_FRAME_HASHES = canvas::snapshotHashes()
  ELSE
    __CANVAS_FRAME_HASHES = __canvas_hashScene(__canvas_frameItems(), [])
    __CANVAS_SNAP_REHASHED = __CANVAS_SNAP_REHASHED + 1
  END IF

  ' bug-686, pass 1, native: every item resolved from its frame hash, and every miss
  ' of a kind `canvas::geoBuild` builds built on the spot -- into `__CANVAS_TOP_OFFSETS`,
  ' one per scene index. What it leaves at -1 (a picture, a `Text`, a `Group`, a declined
  ' paint or kind) is this function's. On a scene of the common kinds that is nothing,
  ' and the scene is never copied out of the ring.
  LET pending AS Integer = canvas::sceneResolve()
  __CANVAS_DRAW_DX = []
  __CANVAS_DRAW_DY = []
  __CANVAS_DRAW_HASHES = []
  __CANVAS_DRAW_MAT = []
  __CANVAS_GROUP_ROWS = []
  MUT side AS List OF Integer = []
  MUT spans AS List OF Integer = []
  MUT several AS Boolean = FALSE
  IF pending > 0 THEN
    __canvas_ensureFrameItems()
    ' Bound once -- see `__canvas_sceneDraws`: a call in `getOr`'s default would snapshot
    ' the whole global scene list per item.
    LET none AS DrawItem = __canvas_noItem()
    ' Pass 2: each index pass 1 left goes through `__canvas_appendDraw`, which builds a
    ' missing item's geometry, re-reads a picture's image, and expands a group into its
    ' children. Its entries land in `side` and the `__CANVAS_DRAW_*` globals, and the
    ' span -- (index, first entry, entry count) -- tells `canvas::sceneLayout` where.
    LET count AS Integer = len(__CANVAS_TOP_OFFSETS)
    MUT i AS Integer = 0
    WHILE i < count
      IF collections::getOr(__CANVAS_TOP_OFFSETS, i, 0) < 0 THEN
        LET item AS DrawItem = collections::getOr(__CANVAS_FRAME_ITEMS, i, none)
        LET before AS Integer = len(side)
        side = __canvas_appendDraw(side, item, collections::getOr(__CANVAS_FRAME_HASHES, i, 0), 0.0, 0.0, 0, 0, 0, i)
        spans = collections::append(spans, i)
        spans = collections::append(spans, before)
        spans = collections::append(spans, len(side) - before)
        MATCH item
          CASE Group(g)
            __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, i, 0 - 1)
          ' plan-150-A: a particle system is never "that entry", even on a frame where
          ' exactly one of its particles is alive: the entry is a particle's record,
          ' not the item's, and an index that names a record is held to be the item's
          ' (`__canvas_geoVerifyFrame`). Always the several-entries path.
          CASE ParticleSystem(ps)
            __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, i, 0 - 2)
            several = TRUE
          CASE ELSE
            ' plan-172-L: an item that became one entry is that entry. A `Picture` may
            ' be two, or none: -2, which sends the frame to the draw walk
            ' (`__canvas_sceneDrawsWalk`), and that takes its blocks from the frame's
            ' entries.
            IF len(side) - before = 1 THEN
              __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, i, collections::getOr(side, before, 0 - 1))
            ELSE
              __CANVAS_TOP_OFFSETS = collections::set(__CANVAS_TOP_OFFSETS, i, 0 - 2)
              several = TRUE
            END IF
        END MATCH
      END IF
      i = i + 1
    END WHILE
  END IF
  ' Pass 3, native: the frame's draw entries in scene order -- one per resolved index,
  ' the span for each other -- replacing the `__CANVAS_DRAW_*` globals.
  LET offsets AS List OF Integer = canvas::sceneLayout(spans, side)
  __canvas_geoVerifyFrame(pending)
  ' plan-172-K: every `AutoPaint` the walk noted is given its state for this frame, and
  ' an entry showing a state paint is swapped for that paint's record -- in the list
  ' returned here, in `__CANVAS_DRAW_HASHES` and in `__CANVAS_TOP_OFFSETS`, before
  ' anything reads them. Asked first, so a frame with no `AutoPaint` in it -- every
  ' frame of a program that constructs none -- does not hand the list anywhere.
  IF __canvas_autoPending() THEN
    LET resolved AS List OF Integer = __canvas_autoResolve(offsets, spans)
    IF several THEN
      __CANVAS_FRAME_ENTRIES = resolved
      __CANVAS_FRAME_SPANS = spans
    END IF
    RETURN resolved
  END IF
  ' Kept only for a frame that needs them: a copy of the frame's entries is a cost no
  ' ordinary scene should pay.
  IF several THEN
    __CANVAS_FRAME_ENTRIES = offsets
    __CANVAS_FRAME_SPANS = spans
  END IF
  RETURN offsets
END FUNC

' The frame buffer's item region holds one block per drawn QUAD, and both backends index
' it by instance (plan-116-A). A shape is one quad; a glyph run is one per glyph, because
' each glyph is its own quad with its own block.
'
' Counting the run's whole glyph count over-estimates by the glyphs whose cache entry the
' eviction pass dropped -- those draw nothing and take no block. Over-estimating ends a
' pass a hair early, which is the safe direction: under-estimating would let the emitter
' write past the mapping.
'
' bug-765: every `MAX_FRAME` value below is the size of one region of the frame BUFFER,
' and so of one PASS. It is not a limit on a frame: a frame whose blocks do not fit the
' buffer is drawn as several passes into one target (`__canvas_passNext`).
'
' Each backend has its own (bug-686 gave Metal its own; bug-688 gave Vulkan its own
' at the same size) -- `__CANVAS_VULKAN_MAX_FRAME_ITEMS` and
' `__CANVAS_METAL_MAX_FRAME_ITEMS`. Every value on these lines is generated from the Rust
' constant it names.
LET __CANVAS_VULKAN_MAX_FRAME_ITEMS AS Integer = @VULKAN_MAX_FRAME_ITEMS@
LET __CANVAS_METAL_MAX_FRAME_ITEMS AS Integer = @METAL_MAX_FRAME_ITEMS@

' The glyph samples one pass may carry on Metal, summed over its runs -- a buffer-wide
' region like Vulkan's (bug-670), and eight times its size since bug-686, because a
' picture's texels ride it too. `__canvas_runSamples` below counts a run's share.
LET __CANVAS_METAL_MAX_FRAME_GLYPH_SAMPLES AS Integer = @METAL_MAX_FRAME_GLYPH_SAMPLES@

LET __CANVAS_METAL_MAX_FRAME_EDGES AS Integer = @METAL_MAX_FRAME_EDGES@

' The gradient stops one pass may carry, summed over its items -- the size of the
' gradient region of each backend's shared buffer (plan-116-F). Both were 4,096 until
' bug-686 found 2,100 two-stop gradients declined by it; each is now two per quad.
LET __CANVAS_VULKAN_MAX_FRAME_GRADIENT_STOPS AS Integer = @VULKAN_MAX_FRAME_GRADIENT_STOPS@
LET __CANVAS_METAL_MAX_FRAME_GRADIENT_STOPS AS Integer = @METAL_MAX_FRAME_GRADIENT_STOPS@

' plan-163-B: the dash words one pass may carry, summed over its dashed items -- the size
' of each backend's dash region. An item takes its dash tail plus the phase word
' (`__canvas_dashWords`).
LET __CANVAS_VULKAN_MAX_FRAME_DASH_WORDS AS Integer = @VULKAN_MAX_FRAME_DASH_WORDS@
LET __CANVAS_METAL_MAX_FRAME_DASH_WORDS AS Integer = @METAL_MAX_FRAME_DASH_WORDS@

LET __CANVAS_VULKAN_MAX_FRAME_EDGES AS Integer = @VULKAN_MAX_FRAME_EDGES@
LET __CANVAS_VULKAN_MAX_GLYPH_SAMPLES AS Integer = @VULKAN_MAX_FRAME_GLYPH_SAMPLES@

' The words one item puts in the frame buffer's dash region: its dash tail and the
' phase, or nothing for a solid stroke.
FUNC __canvas_dashWords(offset AS Integer) AS Integer
  LET count AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_DASH_COUNT, 0.0))
  IF count > 0 THEN
    RETURN count + 1
  END IF
  RETURN 0
END FUNC

' The coverage samples one glyph run puts in the frame buffer's glyph region -- the sum
' of its cached bitmaps' areas. A run carries cache indices rather than bitmaps, so this
' is the only place the two caches meet, and it is why the cost of a block cannot be
' taken from the geometry alone.
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

' The words one picture puts in the frame buffer's glyph region: its image's texel
' count, from the aux pair its header carries (bug-484).
FUNC __canvas_pictureSamples(offset AS Integer) AS Integer
  RETURN toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0)) * toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 21, 0.0))
END FUNC

' bug-765: what ONE block takes of each region of a frame buffer. The answer is the
' block's instances -- its share of the item region -- and the four globals below are
' its share of the others. This is the one definition of a block's cost: the frame's
' plan (`__canvas_framePasses`) sums it to ask whether the frame fits one buffer, and
' the pass walk (`__canvas_passNext`) sums it to find where a pass ends. They used to
' be two predicates' own sums, and a count that differed between a predicate and an
' emitter let a frame write past a region.
MUT __CANVAS_COST_EDGES AS Integer = 0
MUT __CANVAS_COST_STOPS AS Integer = 0
MUT __CANVAS_COST_DASH AS Integer = 0
MUT __CANVAS_COST_SAMPLES AS Integer = 0
' The pixel block a picture's samples are the texels of, or -1 for any other block. An
' emitter copies each distinct block ONCE into a buffer and points every later picture
' of it at those texels (`emit_picture_lookup` -- Metal's since bug-686, Vulkan's since
' bug-688), so a second picture of a block already in the pass costs no samples. The key
' is the block address the emitter looks up, rebuilt from the same two header slots.
MUT __CANVAS_COST_PICTURE AS Integer = 0 - 1

FUNC __canvas_blockCost(offset AS Integer) AS Integer
  LET kind AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset, 0.0))
  MUT samples AS Integer = 0
  MUT picture AS Integer = 0 - 1
  IF kind = __CANVAS_GEO_TEXT THEN
    samples = __canvas_runSamples(offset)
  END IF
  ' bug-484: a picture's texels ride the same region as glyph coverage, one word each --
  ' the region overflowing would make one picture read another's texels, a plausible
  ' wrong image.
  IF kind = __CANVAS_GEO_PICTURE THEN
    picture = toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_HI)) * __CANVAS_GEO_PICTURE_SPLIT + toInt(__canvas_geoAt(offset, __CANVAS_GEO_PICTURE_SHADOW_LO))
    samples = __canvas_pictureSamples(offset)
  END IF
  __CANVAS_COST_SAMPLES = samples
  __CANVAS_COST_PICTURE = picture
  ' plan-116-F Phase 4: a gradient's stops take a slice of one buffer-wide region. A
  ' count below two is not a gradient and contributes nothing.
  LET stops AS Integer = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_GRADIENT_COUNT, 0.0))
  IF stops >= 2 THEN
    __CANVAS_COST_STOPS = stops
  ELSE
    __CANVAS_COST_STOPS = 0
  END IF
  MUT edges AS Integer = 0
  ' bug-686: no per-polygon cap. A polygon of any size that fits the edge region draws;
  ' the emitter indexes a large one by band so the shader does not walk every edge per
  ' pixel.
  IF kind = __CANVAS_GEO_POLYGON THEN
    edges = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  END IF
  ' plan-154-B: a PolyLine's segments take the same region, one edge each.
  IF kind = __CANVAS_GEO_POLYLINE THEN
    edges = toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  END IF
  ' plan-164-C: a curve shape's pieces take the same region, TWO slots a piece -- its
  ' chord, which the band index reads as an edge, and its control point and weight.
  IF kind = __CANVAS_GEO_POLYCURVE OR kind = __CANVAS_GEO_CURVEPOLYGON THEN
    edges = 2 * toInt(collections::getOr(__CANVAS_GEO_DATA, offset + 20, 0.0))
  END IF
  ' plan-163-B: a stroke's join records ride the same region, two edge slots each.
  edges = edges + 2 * toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_JOIN_COUNT, 0.0))
  ' plan-163-C: a path clip's edges ride the edge region too, one slot each.
  __CANVAS_COST_EDGES = edges + toInt(collections::getOr(__CANVAS_GEO_DATA, offset + __CANVAS_GEO_CLIP_EDGE_COUNT, 0.0))
  ' plan-163-B: its dash words a region of their own.
  __CANVAS_COST_DASH = __canvas_dashWords(offset)
  ' The item region counts PUBLISHED RECORDS, so it asks the same function the draw list
  ' asks. A blended item that both strokes and fills publishes two
  ' (`emit_split_or_publish`); counting it as one let a scene near the region's end write
  ' past the mapping.
  RETURN __canvas_blockInstances(offset)
END FUNC

' The sizes of the regions of the frame buffer this frame is being laid out for -- the
' backend's, set by `__canvas_framePasses` and read by `__canvas_passNext`.
MUT __CANVAS_PASS_MAX_ITEMS AS Integer = 0
MUT __CANVAS_PASS_MAX_SAMPLES AS Integer = 0
MUT __CANVAS_PASS_MAX_EDGES AS Integer = 0
MUT __CANVAS_PASS_MAX_STOPS AS Integer = 0
MUT __CANVAS_PASS_MAX_DASH AS Integer = 0

' bug-765: how a backend whose frame buffer has these region sizes draws the frame's
' blocks (`__CANVAS_DRAW_BLOCKS`, what an emitter writes into the buffer):
'
'   1  they fit one buffer: one pass, the frame's own lists handed over as they are;
'   2  they do not: several passes (`__canvas_passNext`);
'   0  some block does not fit a region BY ITSELF, so no pass could hold it. That is the
'      one scene a backend truly cannot draw, and the whole frame is the software
'      renderer's -- software is the oracle, so a declined frame is at least as correct,
'      and drawing the block cut short would draw a DIFFERENT shape.
'
' A frame is never declined for its size. Until bug-765 it was: this sum was a predicate
' that refused any frame past ONE buffer, and a scene of 100,000 dots was drawn in
' software for good.
'
' Only the blocks past the point where the running sum first leaves a region are asked
' whether they fit alone: a block before it is no larger than a sum that fitted.
'
' No region needs a per-reference multiplier. A group's blocks are recorded ONCE and
' referenced by base: a diamond referencing one leaf twice reports `entries=1 blocks=1`
' and two draw entries that share base 0 with different offsets, and the buffer holds
' the one block.
'
' A group scene cannot be declined by looking for a `Group` item: by the time this
' sees the block list the walk has expanded every group away, so a search finds
' nothing (`.ai/canvas-threading.md` section 10). Both backends draw a group from the
' draw entry's offset, sent to BOTH shader stages.
FUNC __canvas_framePasses(blocks AS List OF Integer, maxItems AS Integer, maxSamples AS Integer, maxEdges AS Integer, maxStops AS Integer, maxDash AS Integer) AS Integer
  __CANVAS_PASS_MAX_ITEMS = maxItems
  __CANVAS_PASS_MAX_SAMPLES = maxSamples
  __CANVAS_PASS_MAX_EDGES = maxEdges
  __CANVAS_PASS_MAX_STOPS = maxStops
  __CANVAS_PASS_MAX_DASH = maxDash
  MUT quads AS Integer = 0
  MUT samples AS Integer = 0
  MUT edges AS Integer = 0
  MUT stops AS Integer = 0
  MUT dash AS Integer = 0
  MUT pictures AS Set OF Integer = Set OF Integer { }
  MUT several AS Boolean = FALSE
  FOR EACH offset IN blocks
    LET own AS Integer = __canvas_blockCost(offset)
    IF NOT several THEN
      quads = quads + own
      edges = edges + __CANVAS_COST_EDGES
      stops = stops + __CANVAS_COST_STOPS
      dash = dash + __CANVAS_COST_DASH
      IF __CANVAS_COST_PICTURE < 0 THEN
        samples = samples + __CANVAS_COST_SAMPLES
      ELSEIF NOT collections::contains(pictures, __CANVAS_COST_PICTURE) THEN
        pictures = collections::add(pictures, __CANVAS_COST_PICTURE)
        samples = samples + __CANVAS_COST_SAMPLES
      END IF
      several = quads > maxItems OR samples > maxSamples OR edges > maxEdges OR stops > maxStops OR dash > maxDash
    END IF
    IF several THEN
      IF own > maxItems OR __CANVAS_COST_SAMPLES > maxSamples OR __CANVAS_COST_EDGES > maxEdges OR __CANVAS_COST_STOPS > maxStops OR __CANVAS_COST_DASH > maxDash THEN
        RETURN 0
      END IF
    END IF
  NEXT
  IF several THEN
    RETURN 2
  END IF
  RETURN 1
END FUNC

' The gate for each backend: `__canvas_framePasses` with the sizes of that backend's
' frame buffer regions. Two names because the region sizes are each backend's own
' constants, and a backend must be handed its own.
FUNC __canvas_metalRenderable(blocks AS List OF Integer) AS Integer
  RETURN __canvas_framePasses(blocks, __CANVAS_METAL_MAX_FRAME_ITEMS, __CANVAS_METAL_MAX_FRAME_GLYPH_SAMPLES, __CANVAS_METAL_MAX_FRAME_EDGES, __CANVAS_METAL_MAX_FRAME_GRADIENT_STOPS, __CANVAS_METAL_MAX_FRAME_DASH_WORDS)
END FUNC

FUNC __canvas_vulkanRenderable(blocks AS List OF Integer) AS Integer
  RETURN __canvas_framePasses(blocks, __CANVAS_VULKAN_MAX_FRAME_ITEMS, __CANVAS_VULKAN_MAX_GLYPH_SAMPLES, __CANVAS_VULKAN_MAX_FRAME_EDGES, __CANVAS_VULKAN_MAX_FRAME_GRADIENT_STOPS, __CANVAS_VULKAN_MAX_FRAME_DASH_WORDS)
END FUNC

' bug-765: a PASS is a run of the frame's draw list that fits one frame buffer -- the
' blocks an emitter writes into the buffer (`__CANVAS_PASS_BLOCKS`) and the draw entries
' over them (`__CANVAS_PASS_DRAWS`), in the layout of `__CANVAS_DRAW_BLOCKS` and
' `__CANVAS_DRAWS` and handed to the same members. `__CANVAS_PASS_ENTRY` is the frame's
' draw entry the next pass starts in and `__CANVAS_PASS_BLOCK` the block of that entry
' it starts at, or -1 for the entry's first.
MUT __CANVAS_PASS_BLOCKS AS List OF Integer = []
MUT __CANVAS_PASS_DRAWS AS List OF Integer = []
MUT __CANVAS_PASS_ENTRY AS Integer = 0
MUT __CANVAS_PASS_BLOCK AS Integer = 0 - 1

' The width of a draw entry, and the word of a pass's FIRST entry that tells an emitter
' where the pass stands in its frame (`CANVAS_DRAW_PASS_WORD`): `CONTINUES` when a pass
' of this frame was drawn before it, so the target is loaded and not cleared; `MORE`
' when one follows, so the frame is neither presented nor read back yet. A frame of one
' pass has neither, and its lists are the frame's own, whose word is 0.
LET __CANVAS_DRAW_ENTRY_WORDS AS Integer = @DRAW_ENTRY_WORDS@
LET __CANVAS_DRAW_PASS_WORD AS Integer = @DRAW_PASS_WORD@
LET __CANVAS_PASS_CONTINUES AS Integer = @DRAW_PASS_CONTINUES@
LET __CANVAS_PASS_MORE AS Integer = @DRAW_PASS_MORE@

' Start the frame's passes over, from its first draw entry.
FUNC __canvas_passBegin() AS Nothing
  __CANVAS_PASS_ENTRY = 0
  __CANVAS_PASS_BLOCK = 0 - 1
END FUNC

' The frame's block whose instances start at `instance`: the last block starting at or
' before it. A draw entry starts at a block that publishes something, and a block that
' publishes nothing (a `Text` of no glyphs) starts where the next one does.
FUNC __canvas_passBlockAt(instance AS Integer) AS Integer
  MUT low AS Integer = 0
  MUT high AS Integer = len(__CANVAS_DRAW_INST)
  WHILE high - low > 1
    LET middle AS Integer = (low + high) / 2
    IF collections::getOr(__CANVAS_DRAW_INST, middle, 0) <= instance THEN
      low = middle
    ELSE
      high = middle
    END IF
  END WHILE
  RETURN low
END FUNC

' One draw entry of the pass: the frame's entry at word `at`, over `count` instances
' starting at the PASS's instance `base`. Everything else an entry carries -- its offset,
' its blend mode, its matrix -- is the frame entry's, so a run cut in two by a pass
' boundary is drawn as it would have been whole.
FUNC __canvas_passDraw(at AS Integer, base AS Integer, count AS Integer) AS Nothing
  IF count <= 0 THEN
    RETURN
  END IF
  __CANVAS_PASS_DRAWS = collections::append(__CANVAS_PASS_DRAWS, base)
  __CANVAS_PASS_DRAWS = collections::append(__CANVAS_PASS_DRAWS, count)
  MUT word AS Integer = 2
  WHILE word < __CANVAS_DRAW_ENTRY_WORDS
    __CANVAS_PASS_DRAWS = collections::append(__CANVAS_PASS_DRAWS, collections::getOr(__CANVAS_DRAWS, at + word, 0))
    word = word + 1
  END WHILE
END FUNC

' Where in this pass the frame's instances `base`..`base + count` already are, or -1.
' `whole` holds a triple -- frame base, count, pass base -- for every entry this pass
' holds from its first block to its last, in the order they were laid, which is rising
' frame base: a group's blocks are recorded when it is first drawn.
FUNC __canvas_passFind(whole AS List OF Integer, base AS Integer, count AS Integer) AS Integer
  MUT low AS Integer = 0
  MUT high AS Integer = len(whole) / 3
  WHILE low < high
    LET middle AS Integer = (low + high) / 2
    LET there AS Integer = collections::getOr(whole, middle * 3, 0)
    IF there = base THEN
      IF collections::getOr(whole, middle * 3 + 1, 0) = count THEN
        RETURN collections::getOr(whole, middle * 3 + 2, 0 - 1)
      END IF
      RETURN 0 - 1
    END IF
    IF there < base THEN
      low = middle + 1
    ELSE
      high = middle
    END IF
  END WHILE
  RETURN 0 - 1
END FUNC

' bug-765: lay out the frame's next pass -- the longest run of its draw list, from where
' the last pass stopped, whose blocks fit every region of one frame buffer -- and answer
' whether the frame has more to draw after it.
'
' The run is found by walking the DRAW ENTRIES, not the block list, because the entries
' are the frame in paint order and a pass must draw a prefix of what is left: a group
' drawn twice names one run of blocks from two entries, and the second may fall in a
' later pass than the blocks' place in the block list. Each entry's blocks are added
' while they fit (`__canvas_blockCost`, the sum `__canvas_framePasses` made); the first
' that does not ends the pass, and the entry's remaining blocks open the next one as an
' entry of their own. The unit is the BLOCK -- one item's published records -- so the two
' records of a split blended item, and the glyphs of a `Text`, are never in different
' passes, and nothing here asks what kind a block is.
'
' Nothing else has to stay together. An entry is one instanced draw of consecutive
' blocks under one pipeline at one offset, and an instanced draw of N blocks composites
' exactly as two draws of its two halves do, in order, into the same target. A group, a
' layer and a blend-mode change are entry boundaries already.
'
' Every table an emitter keeps while it fills a buffer is the pass's, since each pass
' is one call that starts them empty: the edge, gradient, dash and glyph cursors, the
' band index, and the picture table. So a picture block is charged once a PASS, and an
' entry whose blocks this pass already holds (the same group again) is drawn from them
' and charged nothing; one whose blocks an EARLIER pass held is laid out again.
'
' `__canvas_framePasses` answered 2 before this is called, so every block fits an empty
' pass and each call places at least one.
FUNC __canvas_passNext() AS Boolean
  LET first AS Boolean = (__CANVAS_PASS_ENTRY = 0) AND (__CANVAS_PASS_BLOCK < 0)
  __CANVAS_PASS_BLOCKS = []
  __CANVAS_PASS_DRAWS = []
  MUT whole AS List OF Integer = []
  MUT pictures AS Set OF Integer = Set OF Integer { }
  MUT quads AS Integer = 0
  MUT samples AS Integer = 0
  MUT edges AS Integer = 0
  MUT stops AS Integer = 0
  MUT dash AS Integer = 0
  MUT more AS Boolean = FALSE
  LET entries AS Integer = len(__CANVAS_DRAWS) / __CANVAS_DRAW_ENTRY_WORDS
  WHILE NOT more AND __CANVAS_PASS_ENTRY < entries
    LET at AS Integer = __CANVAS_PASS_ENTRY * __CANVAS_DRAW_ENTRY_WORDS
    LET base AS Integer = collections::getOr(__CANVAS_DRAWS, at, 0)
    LET count AS Integer = collections::getOr(__CANVAS_DRAWS, at + 1, 0)
    MUT held AS Integer = 0 - 1
    IF __CANVAS_PASS_BLOCK < 0 THEN
      held = __canvas_passFind(whole, base, count)
    END IF
    IF held >= 0 THEN
      __canvas_passDraw(at, held, count)
    ELSE
      MUT here AS Integer = __CANVAS_PASS_BLOCK
      IF here < 0 THEN
        here = __canvas_passBlockAt(base)
      END IF
      LET started AS Integer = quads
      WHILE NOT more AND collections::getOr(__CANVAS_DRAW_INST, here, base + count) < base + count
        LET offset AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, here, 0)
        LET own AS Integer = __canvas_blockCost(offset)
        MUT texels AS Integer = __CANVAS_COST_SAMPLES
        LET picture AS Integer = __CANVAS_COST_PICTURE
        IF picture >= 0 THEN
          IF collections::contains(pictures, picture) THEN
            texels = 0
          END IF
        END IF
        IF quads + own > __CANVAS_PASS_MAX_ITEMS OR samples + texels > __CANVAS_PASS_MAX_SAMPLES OR edges + __CANVAS_COST_EDGES > __CANVAS_PASS_MAX_EDGES OR stops + __CANVAS_COST_STOPS > __CANVAS_PASS_MAX_STOPS OR dash + __CANVAS_COST_DASH > __CANVAS_PASS_MAX_DASH THEN
          more = TRUE
        ELSE
          __CANVAS_PASS_BLOCKS = collections::append(__CANVAS_PASS_BLOCKS, offset)
          quads = quads + own
          samples = samples + texels
          edges = edges + __CANVAS_COST_EDGES
          stops = stops + __CANVAS_COST_STOPS
          dash = dash + __CANVAS_COST_DASH
          IF picture >= 0 THEN
            pictures = collections::add(pictures, picture)
          END IF
          here = here + 1
        END IF
      END WHILE
      __canvas_passDraw(at, started, quads - started)
      IF more THEN
        __CANVAS_PASS_BLOCK = here
      ELSEIF __CANVAS_PASS_BLOCK < 0 THEN
        whole = collections::append(whole, base)
        whole = collections::append(whole, count)
        whole = collections::append(whole, started)
      END IF
    END IF
    IF NOT more THEN
      __CANVAS_PASS_ENTRY = __CANVAS_PASS_ENTRY + 1
      __CANVAS_PASS_BLOCK = 0 - 1
    END IF
  END WHILE
  MUT word AS Integer = 0
  IF NOT first THEN
    word = word + __CANVAS_PASS_CONTINUES
  END IF
  IF more THEN
    word = word + __CANVAS_PASS_MORE
  END IF
  __CANVAS_PASS_DRAWS = collections::set(__CANVAS_PASS_DRAWS, __CANVAS_DRAW_PASS_WORD, word)
  RETURN more
END FUNC

' A frame of several passes, presented straight to the window: every pass through
' `canvas::metalPresentScene`, which renders each into the one offscreen target and
' copies that into the window's drawable only for the last (`CANVAS_DRAW_PASS_MORE`).
' FALSE at any pass is the whole frame not presented, and the caller draws it again
' down the readback path from its first pass -- headless that is the first pass's
' answer, given before Metal is touched.
FUNC __canvas_metalPassesDirect(width AS Integer, height AS Integer) AS Boolean
  __canvas_passBegin()
  MUT more AS Boolean = TRUE
  WHILE more
    more = __canvas_passNext()
    IF NOT canvas::metalPresentScene(width, height, __CANVAS_GEO_DATA, __CANVAS_PASS_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_PASS_DRAWS) THEN
      RETURN FALSE
    END IF
  END WHILE
  RETURN TRUE
END FUNC

FUNC __canvas_renderMetal(width AS Integer, height AS Integer) AS Boolean
  LET passes AS Integer = __canvas_metalRenderable(__CANVAS_DRAW_BLOCKS)
  IF passes = 0 THEN
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
    MUT presented AS Boolean = FALSE
    IF passes = 1 THEN
      presented = canvas::metalPresentScene(width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS)
    ELSE
      presented = __canvas_metalPassesDirect(width, height)
    END IF
    IF presented THEN
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
  ' bug-1024: FALSE is a frame the GPU did not draw (its command buffer did not
  ' complete). The buffer then holds nothing to show, so decline, as for a scene no pass
  ' can hold: the caller draws this frame in software. bug-765: that is so of ANY pass
  ' of the frame -- the target holds the passes before it and not this one, which is not
  ' a frame, so it is never read back.
  IF passes = 1 THEN
    IF NOT canvas::metalDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS) THEN
      RETURN FALSE
    END IF
  ELSE
    ' The passes are walked here, not in a function handed the surface: the readback
    ' writes the pixels through the list this function owns.
    __canvas_passBegin()
    MUT more AS Boolean = TRUE
    WHILE more
      more = __canvas_passNext()
      IF NOT canvas::metalDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_PASS_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_PASS_DRAWS) THEN
        RETURN FALSE
      END IF
    END WHILE
  END IF
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

' `__canvas_metalPassesDirect`'s twin: every pass through `canvas::vulkanPresentScene`,
' which records each into the one swapchain image the first pass acquired and presents
' it after the last.
FUNC __canvas_vulkanPassesDirect(width AS Integer, height AS Integer) AS Boolean
  __canvas_passBegin()
  MUT more AS Boolean = TRUE
  WHILE more
    more = __canvas_passNext()
    IF NOT canvas::vulkanPresentScene(width, height, __CANVAS_GEO_DATA, __CANVAS_PASS_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_PASS_DRAWS) THEN
      RETURN FALSE
    END IF
  END WHILE
  RETURN TRUE
END FUNC

FUNC __canvas_renderVulkan(width AS Integer, height AS Integer) AS Boolean
  LET passes AS Integer = __canvas_vulkanRenderable(__CANVAS_DRAW_BLOCKS)
  IF passes = 0 THEN
    RETURN FALSE
  END IF
  ' bug-688 V6: the twin of `__canvas_renderMetal`'s direct present -- the frame goes
  ' to the window's swapchain with no CPU surface, copy, swizzle or blit, which at a
  ' full-screen surface were most of the frame. Same conditions, same fallback:
  ' `vulkanPresentScene` answers FALSE headless (no window), on a target or driver with
  ' no swapchain, when the window is not the canvas size, or when no image was free,
  ' and the frame then takes the readback path below.
  IF NOT __canvas_damageEnabled() AND NOT __canvas_dumpRequested() THEN
    MUT presented AS Boolean = FALSE
    IF passes = 1 THEN
      presented = canvas::vulkanPresentScene(width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS)
    ELSE
      presented = __canvas_vulkanPassesDirect(width, height)
    END IF
    IF presented THEN
      __CANVAS_GPU_FRAMES = __CANVAS_GPU_FRAMES + 1
      __canvas_presentedDirect()
      RETURN TRUE
    END IF
  END IF
  LET buffer AS List OF Byte = canvas::newSurface(width, height)
  ' plan-116-H: the BLOCK list, not the software walk's offsets. A shared group appears
  ' once in it; `__CANVAS_DRAWS` says who draws which slice and at what offset.
  IF passes = 1 THEN
    canvas::vulkanDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_DRAW_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_DRAWS)
  ELSE
    ' Walked here for the reason `__canvas_renderMetal` gives.
    __canvas_passBegin()
    MUT more AS Boolean = TRUE
    WHILE more
      more = __canvas_passNext()
      canvas::vulkanDrawScene(buffer, width, height, __CANVAS_GEO_DATA, __CANVAS_PASS_BLOCKS, __CANVAS_GLYPH_META, __CANVAS_GLYPH_COV, __CANVAS_PASS_DRAWS)
    END WHILE
  END IF
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
    ("VULKAN_MAX_FRAME_ITEMS", VULKAN_MAX_FRAME_ITEMS),
    ("METAL_MAX_FRAME_ITEMS", METAL_MAX_FRAME_ITEMS),
    (
        "METAL_MAX_FRAME_GLYPH_SAMPLES",
        METAL_MAX_FRAME_GLYPH_SAMPLES,
    ),
    ("METAL_MAX_FRAME_EDGES", METAL_MAX_FRAME_EDGES),
    (
        "VULKAN_MAX_FRAME_GRADIENT_STOPS",
        VULKAN_MAX_FRAME_GRADIENT_STOPS,
    ),
    (
        "METAL_MAX_FRAME_GRADIENT_STOPS",
        METAL_MAX_FRAME_GRADIENT_STOPS,
    ),
    ("VULKAN_MAX_FRAME_DASH_WORDS", VULKAN_MAX_FRAME_DASH_WORDS),
    ("METAL_MAX_FRAME_DASH_WORDS", METAL_MAX_FRAME_DASH_WORDS),
    ("VULKAN_MAX_FRAME_EDGES", VULKAN_MAX_FRAME_EDGES),
    (
        "VULKAN_MAX_FRAME_GLYPH_SAMPLES",
        VULKAN_MAX_FRAME_GLYPH_SAMPLES,
    ),
    ("DRAW_ENTRY_WORDS", CANVAS_DRAW_ENTRY_WORDS),
    ("DRAW_PASS_WORD", CANVAS_DRAW_PASS_WORD),
    ("DRAW_PASS_CONTINUES", CANVAS_DRAW_PASS_CONTINUES),
    ("DRAW_PASS_MORE", CANVAS_DRAW_PASS_MORE),
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
      LET none AS DrawItem = Group[id := "", position := vector::Float2[x := 0.0, y := 0.0], transform := __canvas_noTransform(), name := ""]
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
/// Vulkan on Linux and Windows, where a pipeline exists); a headless run
/// (`MFB_HEADLESS`) draws in software.
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
    __canvas_readSyncMode()
    canvas::setGpuMode(__canvas_wantGpu())
    __canvas_readDisplayOverride()
    canvas::startGraphics()
    __CANVAS_GFX_READY = TRUE
  END IF
END FUNC

FUNC __canvas_wantGpu() AS Boolean
  LET choice AS String = os::getEnvOr("MFB_CANVAS_GPU", "")
  IF len(choice) > 0 THEN RETURN choice <> "0"
  IF len(os::getEnvOr("MFB_HEADLESS", "")) > 0 THEN RETURN FALSE
  RETURN TRUE
END FUNC

' Make `present` wait for the frame it asked for. Any non-empty value turns it on.
'
' A test switch, yet read by EVERY build rather than only `--debug` like the others
' (`./mfb spec tooling debug-report`): while canvas is in development,
' `tools/canvas-bench`'s `sweep --release` needs one rendered frame per present in a
' release build, to measure fps without a `--debug` build's instrumentation. It moves
' behind `--debug` once canvas settles.
FUNC __canvas_readSyncMode() AS Nothing
  canvas::setSyncMode(len(os::getEnvOr("MFB_CANVAS_SYNC", "")) > 0)
END FUNC"#;

/// The two switches a test sets to make a headless run observable beyond
/// `MFB_CANVAS_SYNC`: report a pinned display rate, and hold the graphics thread
/// inside a frame. A normal build has neither — each body is empty, which leaves the
/// graphics state's display-override word at the zero it starts with.
const TEST_SWITCHES: &str = r#"FUNC __canvas_readDisplayOverride() AS Nothing
END FUNC

FUNC __canvas_frameHold() AS Nothing
END FUNC"#;

/// [`TEST_SWITCHES`] for a `--debug` build, which is the only build that reads
/// `MFB_CANVAS_DISPLAY_HZ` and `MFB_CANVAS_FRAME_HOLD_MS`.
#[rustfmt::skip]
const TEST_SWITCHES_DEBUG: &str =
r#"' plan-155-A: pin the display rate the runtime reports, since a headless run has no
' monitor to ask. Read once, by whichever asks first: the first present, or
' (plan-155-B) a `canvas::getFrameStats` made before it.
MUT __CANVAS_DISPLAY_OVERRIDE_READ AS Boolean = FALSE

FUNC __canvas_readDisplayOverride() AS Nothing
  IF NOT __CANVAS_DISPLAY_OVERRIDE_READ THEN
    ' bug-807: a malformed or out-of-range override is ignored, as if unset. It is a
    ' harness affordance, so it must never make a present or a frame-stats read fail.
    MUT hz AS Integer = toInt(os::getEnvOr("MFB_CANVAS_DISPLAY_HZ", "0")) TRAP(e)
      RECOVER 0
    END TRAP
    IF hz < 0 OR hz > 1000000 THEN
      hz = 0
    END IF
    canvas::setDisplayOverride(hz * 1000)
    __CANVAS_DISPLAY_OVERRIDE_READ = TRUE
  END IF
END FUNC

' plan-116-G, G9: hold the graphics thread INSIDE a frame, for tests that need a
' worker action to land mid-render. Off unless the variable is set.
FUNC __canvas_frameHold() AS Nothing
  LET holdMs AS Integer = toInt(os::getEnvOr("MFB_CANVAS_FRAME_HOLD_MS", "0"))
  IF holdMs > 0 THEN
    os::sleep(holdMs)
  END IF
END FUNC"#;

// `__canvas_renderLoop`/`__canvas_renderFrame` are one body in both builds except for
// one line: a skipped frame writes the stats line only in a `--debug` build
// (plan-130-E). The shared text is split at that line so it exists once.
macro_rules! render_loop_head {
    () => {
        r#"' plan-172-K: why the render loop woke -- 1, a redraw is owed; 2, nothing is owed but
' the pointer changed or an `AutoPaint` window ended (`canvas::waitForRedraw`).
MUT __CANVAS_WAKE AS Integer = 1

' bug-996: the Vulkan pipeline is never on a frame's critical path. Building it can take
' tens of seconds on a driver that has not seen this executable, so the first frame that
' wants the GPU only STARTS the build -- on a thread of its own (`canvas::vulkanBuild(0)`)
' -- and every frame asks `canvas::vulkanReady()`, which is one load. Frames are drawn in
' software until it answers TRUE, and by Vulkan from the first frame after.
'
' A program that asked for the GPU by name (`MFB_CANVAS_GPU`) builds here and now instead
' (`canvas::vulkanBuild(1)`): its first frame waits for the pipeline, once, so a test's
' every frame is a GPU frame. This is only reached with the GPU selected, so a set
' variable here is one that selected it.
MUT __CANVAS_VULKAN_ASKED AS Boolean = FALSE

FUNC __canvas_vulkanReady() AS Boolean
  IF NOT __CANVAS_VULKAN_ASKED THEN
    __CANVAS_VULKAN_ASKED = TRUE
    IF len(os::getEnvOr("MFB_CANVAS_GPU", "")) > 0 THEN
      canvas::vulkanBuild(1)
    ELSE
      canvas::vulkanBuild(0)
    END IF
  END IF
  RETURN canvas::vulkanReady()
END FUNC

FUNC __canvas_renderLoop() AS Nothing
  ' plan-172-K: the wait is timed while an `AutoPaint` is inside an enter or exit window
  ' -- the one time-based redraw there is -- and untimed (0) otherwise. It answers 0
  ' when the loop is to stop.
  __CANVAS_WAKE = canvas::waitForRedraw(__canvas_autoTimeout())
  WHILE __CANVAS_WAKE <> 0
    ' plan-155-B: time the frame and report whether -- and by which renderer -- it
    ' was drawn, for `canvas::getFrameStats`. `__canvas_renderFrame` sets the two
    ' globals on every path out of it.
    LET started AS Integer = canvas::frameNanos()
    __canvas_renderFrame()
    LET finished AS Integer = canvas::frameNanos()
    canvas::frameDone(__CANVAS_LAST_DREW, __CANVAS_LAST_RENDERER, finished - started, finished)
    __CANVAS_WAKE = canvas::waitForRedraw(__canvas_autoTimeout())
  END WHILE
END FUNC

FUNC __canvas_renderFrame() AS Nothing
  ' Not drawn until a backend says so: the skip below returns with these unchanged.
  __CANVAS_LAST_DREW = FALSE
  __CANVAS_LAST_RENDERER = 0
  ' plan-172-J: a `--debug` build's scripted pointer, applied before anything in the
  ' frame could read the pointer. Empty in a normal build.
  __canvas_pointerScript()
  ' plan-172-K: a frame nobody is owed -- the pointer changed, or an `AutoPaint` window
  ' ended -- is drawn only if some `AutoPaint` now shows a different paint, and that is
  ' asked before anything is walked (`__canvas_autoLight`). A pointer crossing empty
  ' canvas, or moving within a shape, draws nothing and walks nothing, whatever
  ' `MFB_CANVAS_DAMAGE` says. When the answer is yes the frame is an ordinary one from
  ' here on.
  IF __CANVAS_WAKE = 2 THEN
    IF NOT __canvas_autoLight() THEN
      __CANVAS_DAMAGE = []
      __canvas_frameSkipped()
      RETURN
    END IF
  END IF
  ' plan-172-E: `canvas::Size` holds Floats, for a shape's extent. The surface is whole
  ' pixels and every consumer below wants them as Integers, so they are read directly.
  LET surfaceW AS Integer = canvas::surfaceWidth()
  LET surfaceH AS Integer = canvas::surfaceHeight()
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
  ' A `--debug` build's `MFB_CANVAS_FRAME_HOLD_MS`; nothing in a normal build. Before
  ' this, every "mid-render" row was reached through `MFB_CANVAS_RESIZE_W`/`_H` firing
  ' while the worker slept, which is resize-specific -- so the rows that needed a
  ' different worker action were either untestable (R1) or tested by luck.
  __canvas_frameHold()
  ' plan-116-G: the EXPANDED hashes, written by the walk above, not
  ' `canvas::installedHashes()`. The damage diff pairs hashes with offsets by index, and
  ' expanding one `Group` node into N children makes those two lists different lengths
  ' -- which `__canvas_damageFor` detects and answers with a full redraw, so a group
  ' scene would silently never take the partial path. For a group-free scene this list
  ' is `installedHashes()` passed through unchanged.
  LET hashes AS List OF Integer = __CANVAS_DRAW_HASHES
  LET damage AS List OF Integer = __canvas_damageFor(hashes, offsets, surfaceW, surfaceH)
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
  IF NOT __canvas_damageIsFull(damage, surfaceW, surfaceH) THEN
    __CANVAS_PARTIAL = __CANVAS_PARTIAL + 1
  END IF
  __canvas_rememberScene(hashes, offsets)

  ' A GPU backend renders the whole frame whatever the damage is: it draws into its own
  ' texture and reads the result back, so there is no kept surface for it to preserve.
  ' That is the plan's "capability-absent path falls back to full-frame", and it is the
  ' honest reading here -- the capability the plan named (`VK_KHR_incremental_present`,
  ' Metal dirty rects) belongs to presenting a swapchain drawable, and the direct
  ' present paths (bug-686 Phase 4, bug-688 V6) redraw every drawable whole rather than
  ' track which regions of it are stale (Correction 24).
  ' plan-155-B: the renderer ordinals are `canvas::Renderer`'s -- 0 Software, 1 Metal,
  ' 2 Vulkan. A GPU arm that declines leaves them for the next arm to set.
  IF canvas::useGpu() AND canvas::metalReady() THEN
    IF __canvas_renderMetal(surfaceW, surfaceH) THEN
      __CANVAS_LAST_DREW = TRUE
      __CANVAS_LAST_RENDERER = 1
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  ' bug-996: FALSE while the pipeline is still being built, so this frame is software.
  IF canvas::useGpu() AND __canvas_vulkanReady() THEN
    IF __canvas_renderVulkan(surfaceW, surfaceH) THEN
      __CANVAS_LAST_DREW = TRUE
      __CANVAS_LAST_RENDERER = 2
      __canvas_phaseMark(4)
      RETURN
    END IF
  END IF
  __canvas_renderScene(offsets, damage, surfaceW, surfaceH)
  __CANVAS_LAST_DREW = TRUE
  __CANVAS_LAST_RENDERER = 0
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
/// There are two GPU arms — Metal on macOS, Vulkan on Linux and Windows — and each is
/// taken only when all three of its conditions hold: the program asked for a GPU
/// (`canvas::useGpu`, which despite its name is the one renderer-selection flag and
/// is set by `__canvas_wantGpu`), a pipeline exists (`canvas::metalReady` /
/// `canvas::vulkanReady`), and the *scene* is one that renderer draws correctly.
///
/// The second condition is asked every frame and is not the same kind of question on
/// the two backends. `canvas::metalReady` builds the Metal pipeline the first time it
/// is asked and remembers the answer. `canvas::vulkanReady` only reads whether the
/// pipeline exists yet (bug-996): `__canvas_vulkanReady` starts the build on its own
/// thread the first time and the frames before it finishes are software frames. So on
/// Vulkan the renderer of a run can change once, from software to the GPU, at the
/// first frame after the pipeline is ready.
///
/// The two are mutually exclusive in practice — `metalReady` is FALSE off macOS and
/// `vulkanReady` FALSE on it — so the order between them never decides anything;
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
/// nothing — no timer, no poll, no spin. Time is a redraw trigger in exactly one case
/// (`.ai/canvas-threading.md` §4): a `canvas::AutoPaint` inside its enter or exit
/// window, whose end `__canvas_autoTimeout` hands the wait.
///
/// It renders the *installed* scene rather than being handed one, which is what lets
/// a repaint no `present` caused — a resize, an expose — draw the right picture.
const RENDER_LOOP: &str = concat!(render_loop_head!(), render_loop_tail!());

/// bug-686 Phase 0: where a frame's time goes, reported on the `MFB_CANVAS_STATS` line.
///
/// `__canvas_renderFrame` marks five points: the frame's start, then the end of the scene
/// walk, of the draw list, of the damage diff and of the render (the predicate, the
/// backend's own work, readback and present). A normal build's mark is an empty `FUNC … AS Nothing`;
/// a `--debug` build's accumulates the nanoseconds between consecutive marks per phase,
/// and `__canvas_phaseText` reports each total in milliseconds. Cumulative, like
/// `generations=`, because the interesting quantity is the delta between frames.
const PHASE_TIMERS: &str = r#"FUNC __canvas_phaseMark(phase AS Integer) AS Nothing
END FUNC"#;

#[rustfmt::skip]
const PHASE_TIMERS_DEBUG: &str = r#"MUT __CANVAS_PHASE_AT AS Integer = 0
MUT __CANVAS_PHASE_NS AS List OF Integer = [0, 0, 0, 0]

FUNC __canvas_phaseMark(phase AS Integer) AS Nothing
  LET now AS Integer = canvas::frameNanos()
  IF phase > 0 AND __CANVAS_PHASE_AT > 0 THEN
    LET spent AS Integer = collections::getOr(__CANVAS_PHASE_NS, phase - 1, 0) + (now - __CANVAS_PHASE_AT)
    __CANVAS_PHASE_NS = collections::set(__CANVAS_PHASE_NS, phase - 1, spent)
  END IF
  __CANVAS_PHASE_AT = now
END FUNC

FUNC __canvas_phaseText() AS String
  RETURN " phaseOffsetsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 0, 0) / 1000000) & " phaseDrawsMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 1, 0) / 1000000) & " phaseDamageMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 2, 0) / 1000000) & " phaseRenderMs=" & toString(collections::getOr(__CANVAS_PHASE_NS, 3, 0) / 1000000)
END FUNC"#;

/// plan-172-J: the scripted pointer. A normal build has no script and no reader.
#[rustfmt::skip]
const POINTER_SCRIPT: &str = r#"FUNC __canvas_pointerScript() AS Nothing
END FUNC

' plan-172-K: a frame that was woken and not drawn.
FUNC __canvas_frameSkipped() AS Nothing
  __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
END FUNC

' plan-172-K: when the script's next entry falls due, on `canvas::frameNanos`' clock, or
' 0 for none. There is no script in a normal build.
FUNC __canvas_pointerNextDue() AS Integer
  RETURN 0
END FUNC

' plan-172-K: how long an `AutoPaint`'s enter and exit paints are held, in nanoseconds:
' the project's `options.canvasAutoPaintMS`, 100 unless it says otherwise.
FUNC __canvas_autoWindow() AS Integer
  RETURN canvas::autoWindowMs() * 1000000
END FUNC"#;

/// The `--debug` build's scripted pointer: `MFB_CANVAS_POINTER` holds
/// `ms,x,y,buttons,inside;...`, and at the start of each frame every entry whose time
/// has come is published through `canvas::publishPointer` — the writer the platforms'
/// own pointer handlers call. It is what lets a headless test put the pointer
/// somewhere, on every platform including the one with no scriptable window.
///
/// Time is counted from the first frame, in the monotonic clock the phase timers read.
/// Entries are applied in order and each once; `inside` 0 is the pointer leaving the
/// surface. Anything that is not a digit separates numbers, so the `,` and `;` are for
/// the reader — and a count that is not a multiple of five ignores the tail.
///
/// On the GRAPHICS thread, which is the one deliberate departure from "the main thread
/// publishes": a script has no main-thread event to ride, and the writer is
/// single-writer either way, which a headless run with no real pointer satisfies.
#[rustfmt::skip]
const POINTER_SCRIPT_DEBUG: &str = r#"MUT __CANVAS_POINTER_READ AS Boolean = FALSE
MUT __CANVAS_POINTER_SCRIPT AS List OF Integer = []
MUT __CANVAS_POINTER_NEXT AS Integer = 0
MUT __CANVAS_POINTER_T0 AS Integer = 0

FUNC __canvas_pointerNumbers(text AS String) AS List OF Integer
  LET bytes AS List OF Byte = encoding::utf8Encode(text)
  MUT out AS List OF Integer = []
  MUT value AS Integer = 0
  MUT open AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i < len(bytes)
    LET b AS Integer = toInt(collections::getOr(bytes, i, toByte(0)))
    IF b >= 48 AND b <= 57 THEN
      value = value * 10 + (b - 48)
      open = TRUE
    ELSEIF open THEN
      out = collections::append(out, value)
      value = 0
      open = FALSE
    END IF
    i = i + 1
  END WHILE
  IF open THEN
    out = collections::append(out, value)
  END IF
  RETURN out
END FUNC

FUNC __canvas_pointerScript() AS Nothing
  IF NOT __CANVAS_POINTER_READ THEN
    __CANVAS_POINTER_READ = TRUE
    __CANVAS_POINTER_SCRIPT = __canvas_pointerNumbers(os::getEnvOr("MFB_CANVAS_POINTER", ""))
    __CANVAS_POINTER_T0 = canvas::frameNanos()
  END IF
  LET count AS Integer = len(__CANVAS_POINTER_SCRIPT) / 5
  IF __CANVAS_POINTER_NEXT >= count THEN
    RETURN
  END IF
  LET elapsedMs AS Integer = (canvas::frameNanos() - __CANVAS_POINTER_T0) / 1000000
  WHILE __CANVAS_POINTER_NEXT < count
    LET at AS Integer = __CANVAS_POINTER_NEXT * 5
    IF collections::getOr(__CANVAS_POINTER_SCRIPT, at, 0) > elapsedMs THEN
      RETURN
    END IF
    LET x AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 1, 0)
    LET y AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 2, 0)
    LET buttons AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 3, 0)
    LET inside AS Integer = collections::getOr(__CANVAS_POINTER_SCRIPT, at + 4, 0)
    canvas::publishPointer(x, y, buttons, inside)
    __CANVAS_POINTER_NEXT = __CANVAS_POINTER_NEXT + 1
  END WHILE
END FUNC

' plan-172-K: a frame that was woken and not drawn -- counted, and in this build it
' writes its stats line like any other frame the graphics thread considered.
FUNC __canvas_frameSkipped() AS Nothing
  __CANVAS_SKIPPED = __CANVAS_SKIPPED + 1
  __canvas_writeStats()
END FUNC

' plan-172-K: when the script's next entry falls due, on `canvas::frameNanos`' clock, or
' 0 when it has none left. A script entry is not a pointer EVENT -- nothing signals for
' it -- so a scene with an `AutoPaint` waits no longer than this (`__canvas_autoTimeout`)
' and the entry is applied at the start of the frame that wakes.
FUNC __canvas_pointerNextDue() AS Integer
  IF NOT __CANVAS_POINTER_READ THEN
    RETURN 0
  END IF
  IF __CANVAS_POINTER_NEXT * 5 + 4 >= len(__CANVAS_POINTER_SCRIPT) THEN
    RETURN 0
  END IF
  RETURN __CANVAS_POINTER_T0 + collections::getOr(__CANVAS_POINTER_SCRIPT, __CANVAS_POINTER_NEXT * 5, 0) * 1000000
END FUNC

' plan-172-K: how long an `AutoPaint`'s enter and exit paints are held, in nanoseconds
' -- the project's `options.canvasAutoPaintMS`, unless `MFB_CANVAS_AUTO_WINDOW_MS` says
' otherwise. The override is a test affordance, off by default: a flash that lasts a
' tenth of a second cannot be photographed by a test whose software frame takes longer
' than that to draw.
MUT __CANVAS_AUTO_WINDOW_NS AS Integer = 0

FUNC __canvas_autoWindow() AS Integer
  IF __CANVAS_AUTO_WINDOW_NS = 0 THEN
    LET built AS Integer = canvas::autoWindowMs()
    MUT ms AS Integer = toInt(os::getEnvOr("MFB_CANVAS_AUTO_WINDOW_MS", "0")) TRAP(e)
      RECOVER 0
    END TRAP
    IF ms < 1 OR ms > 3600000 THEN
      ms = built
    END IF
    __CANVAS_AUTO_WINDOW_NS = ms * 1000000
  END IF
  RETURN __CANVAS_AUTO_WINDOW_NS
END FUNC

' The pointer as the graphics thread sees it, for the stats line: `x,y,buttons,inside`.
FUNC __canvas_pointerText() AS String
  LET p AS List OF Integer = canvas::pointerSnapshot()
  RETURN " pointer=" & toString(collections::getOr(p, 0, 0)) & "," & toString(collections::getOr(p, 1, 0)) & "," & toString(collections::getOr(p, 2, 0)) & "," & toString(collections::getOr(p, 3, 0))
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

FUNC __canvas_presentedDirect() AS Nothing
END FUNC"#;

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

FUNC __canvas_presentedDirect() AS Nothing
  __CANVAS_DIRECT_FRAMES = __CANVAS_DIRECT_FRAMES + 1
  __canvas_writeStats()
END FUNC"#;

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
const CLOSE_RETIRED: &str = r#"FUNC __canvas_closeRetired(gone AS List OF DrawItem, scene AS List OF DrawItem) AS Nothing
  IF len(gone) = 0 THEN
    RETURN
  END IF
  FOR EACH item IN gone
    MATCH item
      CASE Picture(p)
        LET ih AS Integer = canvas::imageHandle(p.image)
        IF ih <> 0 AND NOT __canvas_anythingNamesImage(scene, ih) THEN
          canvas::destroyImage(p.image)
        END IF
        ' plan-172-L: and the font of a `Text` it has for a shape.
        __canvas_pictureCloseFont(p, scene)
      CASE Text(t)
        LET fh AS Integer = canvas::fontHandle(t.font)
        IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
          canvas::destroyFont(t.font)
        END IF
      ' plan-172-K: an `AutoPaint` around a `Text` names that text's font.
      CASE AutoPaint(a)
        MATCH a.shape
          CASE Text(t)
            LET fh AS Integer = canvas::fontHandle(t.font)
            IF fh <> 0 AND NOT __canvas_anythingNamesFont(scene, fh) THEN
              canvas::destroyFont(t.font)
            END IF
          CASE ELSE
        END MATCH
      ' plan-150-A: a particle system names the font of a `Text` it has for a template
      ' or for an emitter.
      CASE ParticleSystem(ps)
        __canvas_particleCloseFonts(ps, scene)
      CASE ELSE
    END MATCH
  NEXT
END FUNC

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
      ' plan-172-K: and so does an `AutoPaint` around one.
      CASE AutoPaint(a)
        MATCH a.shape
          CASE Text(t)
            IF canvas::fontHandle(t.font) = handle THEN
              RETURN TRUE
            END IF
          CASE ELSE
        END MATCH
      ' plan-172-L: and a `Picture` whose shape is one.
      CASE Picture(p)
        IF __canvas_pictureNamesFont(p, handle) THEN
          RETURN TRUE
        END IF
      ' plan-150-A: and a particle system whose template or emitter is one.
      CASE ParticleSystem(ps)
        IF __canvas_particleNamesFont(ps, handle) THEN
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
        RegistryHelper::debug_split("canvas_testSwitches", TEST_SWITCHES, TEST_SWITCHES_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in
        RegistryHelper::debug_split("canvas_phaseTimers", PHASE_TIMERS, PHASE_TIMERS_DEBUG)
    {
        pkg.add_helper(helper);
    }
    for helper in
        RegistryHelper::debug_split("canvas_pointerScript", POINTER_SCRIPT, POINTER_SCRIPT_DEBUG)
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
