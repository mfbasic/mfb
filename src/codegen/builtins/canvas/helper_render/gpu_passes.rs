//! plan-176-S: `RENDER_METAL_TEMPLATE`'s piece for the frame passes — block costs, renderability and the Metal/Vulkan renders. `mod.rs` joins the three pieces with
//! `concat!` into the one template.

macro_rules! render_metal_passes {
    () => {
        r#"' The frame buffer's item region holds one block per drawn QUAD, and both backends index
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

' plan-175-B: the draw-list rows -- nodes -- one pass may carry, the size of each
' backend's node region.
LET __CANVAS_METAL_MAX_FRAME_NODES AS Integer = @METAL_MAX_FRAME_NODES@
LET __CANVAS_VULKAN_MAX_FRAME_NODES AS Integer = @VULKAN_MAX_FRAME_NODES@

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
  IF kind = __CANVAS_GEO_PICTURE OR kind = __CANVAS_GEO_TILE THEN
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
MUT __CANVAS_PASS_MAX_NODES AS Integer = 0

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
'
' plan-175-B: and the frame's draw-list ROWS, each a node of the frame buffer's node
' region (`__canvas_rowNodes`): a frame of more than `maxNodes` is several passes too.
FUNC __canvas_framePasses(blocks AS List OF Integer, maxItems AS Integer, maxSamples AS Integer, maxEdges AS Integer, maxStops AS Integer, maxDash AS Integer, maxNodes AS Integer) AS Integer
  __CANVAS_PASS_MAX_ITEMS = maxItems
  __CANVAS_PASS_MAX_SAMPLES = maxSamples
  __CANVAS_PASS_MAX_EDGES = maxEdges
  __CANVAS_PASS_MAX_STOPS = maxStops
  __CANVAS_PASS_MAX_DASH = maxDash
  __CANVAS_PASS_MAX_NODES = maxNodes
  MUT quads AS Integer = 0
  MUT samples AS Integer = 0
  MUT edges AS Integer = 0
  MUT stops AS Integer = 0
  MUT dash AS Integer = 0
  MUT pictures AS Set OF Integer = Set OF Integer { }
  MUT several AS Boolean = __canvas_frameNodes() > maxNodes
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
  RETURN __canvas_framePasses(blocks, __CANVAS_METAL_MAX_FRAME_ITEMS, __CANVAS_METAL_MAX_FRAME_GLYPH_SAMPLES, __CANVAS_METAL_MAX_FRAME_EDGES, __CANVAS_METAL_MAX_FRAME_GRADIENT_STOPS, __CANVAS_METAL_MAX_FRAME_DASH_WORDS, __CANVAS_METAL_MAX_FRAME_NODES)
END FUNC

FUNC __canvas_vulkanRenderable(blocks AS List OF Integer) AS Integer
  RETURN __canvas_framePasses(blocks, __CANVAS_VULKAN_MAX_FRAME_ITEMS, __CANVAS_VULKAN_MAX_GLYPH_SAMPLES, __CANVAS_VULKAN_MAX_FRAME_EDGES, __CANVAS_VULKAN_MAX_FRAME_GRADIENT_STOPS, __CANVAS_VULKAN_MAX_FRAME_DASH_WORDS, __CANVAS_VULKAN_MAX_FRAME_NODES)
END FUNC

' plan-175-B: the nodes one draw-list row takes of a pass's node region -- its repeat,
' and one for a node-only row (repeat 0) -- and the frame's sum of them.
FUNC __canvas_rowNodes(at AS Integer) AS Integer
  LET repeat AS Integer = collections::getOr(__CANVAS_DRAWS, at + __CANVAS_DRAW_REPEAT_WORD, 1)
  IF repeat > 1 THEN
    RETURN repeat
  END IF
  RETURN 1
END FUNC

FUNC __canvas_frameNodes() AS Integer
  MUT nodes AS Integer = 0
  MUT at AS Integer = 0
  WHILE at < len(__CANVAS_DRAWS)
    nodes = nodes + __canvas_rowNodes(at)
    at = at + __CANVAS_DRAW_ENTRY_WORDS
  END WHILE
  RETURN nodes
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
LET __CANVAS_DRAW_REPEAT_WORD AS Integer = @DRAW_REPEAT_WORD@

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
    ' plan-175-B: a row is a node of the pass's node region, so the rows a pass holds
    ' are bounded like its blocks. A pass full of nodes ends before this row, which opens
    ' the next one; an empty pass always takes it (a row is at most the region).
    LET rows AS Integer = len(__CANVAS_PASS_DRAWS) / __CANVAS_DRAW_ENTRY_WORDS
    IF rows > 0 AND rows + __canvas_rowNodes(at) > __CANVAS_PASS_MAX_NODES THEN
      more = TRUE
    ELSEIF held >= 0 THEN
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
END FUNC"#
    };
}
pub(super) use render_metal_passes;
