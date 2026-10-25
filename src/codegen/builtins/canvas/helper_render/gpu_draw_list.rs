//! plan-176-S: `RENDER_METAL_TEMPLATE`'s piece for the GPU draw list, its memo and the scene offsets. `mod.rs` joins the three pieces with
//! `concat!` into the one template.

macro_rules! render_metal_draw_list {
    () => {
        r#"' plan-116-H: the GPU draw list, and the block layout it indexes into.
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
MUT __CANVAS_VIEWPORT_W AS Integer = 0
MUT __CANVAS_VIEWPORT_H AS Integer = 0

' How many item-buffer blocks one geometry record occupies.
' How many INSTANCES one geometry block publishes. This has to agree, case for case,
' with what the emitters actually write into the item buffer -- `emit_split_or_publish`
' and `emit_glyph_publish` in src/codegen/runtime/canvas/vulkan/publish.rs -- because the draw
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

' Append one draw entry: offsets as float32 bit patterns so the whole list stays
' `List OF Integer` and reaches an emitter without a second parallel Float list -- the
' form both shaders read every position in.
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
  ' src/codegen/runtime/canvas/vulkan/publish.rs (`CANVAS_DRAW_ENTRY_SHIFT`), with
  ' `canvas::sceneDrawsFlat` writing the same width natively.
  ' When they disagreed -- this function writing four words while the emitter strode by eight --
  ' the emitter read every other entry and took the following entry's `base` as a blend
  ' mode, which indexes the pipeline table out of range and hands Vulkan a junk
  ' VkPipeline. That does not fail cleanly: it SIGSEGVs inside the driver's JIT-compiled
  ' code, with a backtrace containing no MFBASIC frame at all.
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instBase)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, instCount)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, __canvas_float32Bits(dx))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, __canvas_float32Bits(dy))
  ' The BlendMode every block in this run shares. It travels with the entry because the
  ' pipeline is bound per draw and the draws are issued in a second pass, so a binding
  ' left in the publish walk would apply the LAST item's mode to the whole frame.
  LET mode AS Integer = toInt(__canvas_geoAt(collections::getOr(__CANVAS_DRAW_BLOCKS, base, 0), 26))
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, mode)
  ' Words 5 and 6 are reserved and zero: they held the offsets' 16.16 residuals until the
  ' shaders went to fp32. A frame with no group lays its entries out natively
  ' (`canvas::sceneDrawsFlat`) at offset 0, whose float32 bits are 0 too.
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  __CANVAS_DRAWS = collections::append(__CANVAS_DRAWS, 0)
  ' plan-172-G: word 7 is `hasMatrix`, words 8-11 the inverse linear part of the groups'
  ' composed affine as float32 bits, words 12-15 reserved. The translation stays in
  ' words 2-3 either way.
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
  __canvas_enterGroupMatrix(mat)
  MUT runStart AS Integer = base
  MUT visible AS Boolean = __canvas_boundsMeetViewport(collections::getOr(__CANVAS_DRAW_BLOCKS, base, 0), __CANVAS_VIEWPORT_W, __CANVAS_VIEWPORT_H, dx, dy)
  MUT i AS Integer = base + 1
  WHILE i < base + count
    LET prev AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i - 1, 0)
    LET here AS Integer = collections::getOr(__CANVAS_DRAW_BLOCKS, i, 0)
    LET hereVisible AS Boolean = __canvas_boundsMeetViewport(here, __CANVAS_VIEWPORT_W, __CANVAS_VIEWPORT_H, dx, dy)
    IF hereVisible <> visible OR NOT __canvas_drawsJoin(prev, here) THEN
      IF visible THEN
        __canvas_pushOneDraw(runStart, i - runStart, dx, dy, mat)
      END IF
      runStart = i
      visible = hereVisible
    END IF
    i = i + 1
  END WHILE
  IF visible THEN
    __canvas_pushOneDraw(runStart, base + count - runStart, dx, dy, mat)
  END IF
  __canvas_enterGroupMatrix(0)
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
        LET picGeo AS Integer = __canvas_geoHash(child)
        LET picBlocks AS List OF Integer = __canvas_pictureBlocks(pic, picGeo, __canvas_drawHashOf(child, picGeo), path, k)
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
        LET childGeo AS Integer = __canvas_geoHash(child)
        LET childOffset AS Integer = __canvas_geometryOf(child, childGeo, __canvas_drawHashOf(child, childGeo))
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
  ' plan-173: both lay out every block and publish draw entries only for the runs in
  ' the viewport (`__canvas_boundsMeetViewport`, natively `emit_block_visible`).
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
  ' plan-150-B: and the particle systems the last frame's walk noted for the damage
  ' diff (`helper_particles.rs`; nothing in a program that constructs none).
  __canvas_particleBegin()
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
        ' plan-175-A: the frame's hashes are every index's `D`, then every index's `G`.
        side = __canvas_appendDraw(side, item, collections::getOr(__CANVAS_FRAME_HASHES, i, 0), collections::getOr(__CANVAS_FRAME_HASHES, count + i, 0), 0.0, 0.0, 0, 0, 0, i)
        ' plan-150-B: any particle system that walk expanded belongs to this index.
        __canvas_particleOwned(i, before)
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
  ' plan-150-B: where each of those systems' entries landed in the laid-out frame.
  __canvas_particleLayout(spans)
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

"#
    };
}
pub(super) use render_metal_draw_list;
