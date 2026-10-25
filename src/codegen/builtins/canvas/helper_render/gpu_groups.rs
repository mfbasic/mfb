//! plan-176-S: `RENDER_METAL_TEMPLATE`'s piece for group composition — the group rows, signatures and appended draws. `mod.rs` joins the three pieces with
//! `concat!` into the one template.

macro_rules! render_metal_groups {
    () => {
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
'
' plan-175-A: `hash` is the item's damage identity `D` -- what its draw hash folds -- and
' `geo` its geometry identity `G`, which keys its record (`__canvas_geometryOf`).
FUNC __canvas_appendDraw(offsets AS List OF Integer, item AS DrawItem, hash AS Integer, geo AS Integer, gdx AS Float, gdy AS Float, mat AS Integer, depth AS Integer, path AS Integer, index AS Integer) AS List OF Integer
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
        LET childGeo AS Integer = __canvas_geoHash(child)
        out = __canvas_appendDraw(out, child, __canvas_drawHashOf(child, childGeo), childGeo, childX, childY, childMat, depth + 1, childPath, k)
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
      RETURN __canvas_pictureAppend(out, pic, geo, hash, gdx, gdy, mat, depth, path, index)
    ' plan-150-A: a `ParticleSystem` is the entries its live particles make -- any
    ' number, each an ordinary shape that comes back through this function
    ' (`helper_particles.rs`).
    CASE ParticleSystem(ps)
      RETURN __canvas_particleAppend(out, ps, gdx, gdy, mat, depth, path, index)
    CASE ELSE
      LET offset AS Integer = __canvas_geometryOf(item, geo, hash)
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
  LET imageKind AS Integer = toInt(__canvas_geoAt(offset, 0))
  IF imageKind = __CANVAS_GEO_PICTURE OR imageKind = __CANVAS_GEO_TILE THEN
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

"#
    };
}
pub(super) use render_metal_groups;
