### 1. Overview's mode-gating rule names the wrong exemptions
UNIT:      man-pkg:canvas
PAGE:      package-wide (overview; loadFont, loadImage)
CATEGORY:  overview-mismatch
CLAIM:     "every call that touches the surface requires app::Mode.Canvas — outside it they raise the trappable ErrWrongMode. The canvas::Paint constructors canvas::fill, canvas::stroke and canvas::fillStroke are exempt"
VERDICT:   misleading
EVIDENCE:  `grep -ln ErrWrongMode src/codegen/builtins/canvas/func_*.rs` lists only createImage, didResize, enableMouse, loadImage, pollMouse, present, presentLayers, removeGroup, setGroup, waitFrame. The other 16 members (loadFont, loadSystemFont, listSystemFonts, measureText, caretStops, getBytes, setBytes, getSize, destroyImage, destroyFont, getFrameStats, flatten, nurbsToBeziers, fill, stroke, fillStroke) have no mode gate and no ErrWrongMode in their rendered Errors tables. "Touches the surface" does not predict the split: createImage/loadImage are gated while setBytes/getBytes/getSize(image) are not, and loadImage is gated while loadFont is not. The loadFont page says nothing about mode either way; the loadImage page says "Requires app::Mode.Canvas", so a developer reading both siblings cannot tell whether the asymmetry is real.
SUGGESTED: Replace the exemption sentence with the actual list: "Ten calls need app::Mode.Canvas and raise ErrWrongMode elsewhere: present, presentLayers, setGroup, removeGroup, createImage, loadImage, waitFrame, didResize, enableMouse and pollMouse. Everything else — the Paint constructors, the font calls, measuring, image reads and writes, closing, getFrameStats and the curve helpers — works in any mode." Add one line to loadFont: "Unlike canvas::loadImage, loadFont does not require app::Mode.Canvas."

### 2. "Redrawn only on resize or uncover" is contradicted by setBytes and AutoPaint
UNIT:      man-pkg:canvas
PAGE:      package-wide (overview vs setBytes, AutoPaint)
CATEGORY:  overview-mismatch
CLAIM:     "the runtime draws that scene, and draws it again only when the window is resized or uncovered, until the next present."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/canvas/func_set_bytes.rs:141-146` doc on the public setBytes: "a setBytes on an image the live scene draws signals the redraw itself (`.ai/canvas-threading.md` section 4, trigger 5)". The setBytes page states the new pixels "appear on the next rendered frame" without a present; the AutoPaint type says "present it once and the window keeps it up to date". Both redraw a presented scene with no present, resize or uncover.
SUGGESTED: "…and draws it again, until the next present, only when something it shows changes without a present: the window is resized or uncovered, canvas::setBytes refills an image it draws, or the mouse moves over a canvas::AutoPaint."

### 3. Four pages say a closed font draws nothing; the overview and Text say it falls through to the next font
UNIT:      man-pkg:canvas
PAGE:      destroyFont, loadFont, measureText, types (canvas::Font)
CATEGORY:  consistency
CLAIM:     destroyFont: "Text in a presented scene that names a released font draws as nothing rather than faulting." loadFont: "the item then draws nothing rather than failing." measureText: "Text already in a presented scene still draws (as nothing) after its font is closed". Font resource: "that text simply draws as empty."
VERDICT:   inconsistent
EVIDENCE:  Overview: "closing a font makes its text draw from the next font in its list, or nothing when none is left". Text.fonts: "a closed font in the list is passed over; with none open the text draws nothing". Implementation agrees with the latter: `src/codegen/builtins/canvas/helper_font.rs:900-904` (SHAPE_RUN doc): "a closed font — has no index, so it maps nothing and is passed over: a codepoint no font maps takes glyph 0 of the first open font, and when no font is open … draws nothing". The four function/resource pages state the one-font special case as the general rule.
SUGGESTED: On each of the four pages: "Text in a presented scene draws each character from the next open font in its list, and draws nothing when none is left, rather than failing."

### 4. The types page says passing a handle aliases it; setGroup moves it
UNIT:      man-pkg:canvas
PAGE:      types (Resources preamble, canvas::Image, canvas::Font); overview
CATEGORY:  consistency
CLAIM:     "Passing one to a call, or copying a record that holds one, gives a second name for the same open handle — an alias, not a copy — and closing through either name closes it."
VERDICT:   inconsistent
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-canvas/moveproj/src/main.mfb`: `canvas::setGroup("a", [tile])` where tile holds `img`, then `canvas::getSize(img)`; `mfb build --app moveproj` → `error[2-203-0055 TYPE_USE_AFTER_MOVE]: binding is used after move. Binding img was moved`. The setGroup page documents this ("becomes the group's to close … naming the same image again is a compile error"), but the Image and Font resource entries on the types page, and the overview's Image paragraph ("An image closes itself when its binding goes out of scope, or earlier with canvas::destroyImage"), say nothing about the one call in the package that takes ownership. A developer who reads types and overview and then puts a Picture into setGroup gets a build error neither page predicted.
SUGGESTED: Add to canvas::Image and canvas::Font: "One call takes the handle instead of aliasing it: an image or font inside the items given to canvas::setGroup becomes the group's, and the binding cannot be named again." Add the same sentence to the overview's Image paragraph.

### 5. The overview is an essay on present, not a map of the package
UNIT:      man-pkg:canvas
PAGE:      package-wide (overview)
CATEGORY:  discoverability
CLAIM:     The Description never names loadFont, loadSystemFont, listSystemFonts, measureText, caretStops, enableMouse, pollMouse, AutoPaint, ParticleSystem, presentLayers, setGroup/removeGroup (Group appears once, as a culling aside), or how a canvas program reads the keyboard.
VERDICT:   missing
EVIDENCE:  `mfb man canvas` Description, read in full: eleven paragraphs on present/caching/copy semantics, Paint zero values and Image lifetime. Text is mentioned only as "canvas::Text.fonts a list of RES canvas::Font"; nothing says where a Font comes from. Mouse input appears nowhere. Keyboard input in canvas mode is stated only inside pollMouse ("Keys typed meanwhile are kept, in order, for the program's next io:: read") and in the ErrWrongMode message. A developer arriving with "how do I draw text", "how do I get a click" or "how do I read a key while the canvas is up" has no path from the overview to the page.
SUGGESTED: Add a short "Where to start" paragraph after the first one: shapes and their fields → mfb man canvas types (Rectangle, Circle, …); colour → fill/stroke/fillStroke; text → loadFont or loadSystemFont (listSystemFonts to pick), Text, measureText, caretStops; images → loadImage/createImage, Picture/Tile, setBytes; mouse → enableMouse, pollMouse, AutoPaint; keyboard → the io:: readers work in Mode.Canvas; animation → waitFrame, getFrameStats, ParticleSystem; large or repeated content → setGroup/Group, presentLayers.

### 6. The fact that AutoPaint needs no enableMouse is stated only on Picture
UNIT:      man-pkg:canvas
PAGE:      types (canvas::AutoPaint, canvas::Picture); enableMouse
CATEGORY:  discoverability
CLAIM:     Picture: "When the shape is a canvas::AutoPaint, the paint the mouse selects is the one used, and nothing else is needed for that to work — not canvas::enableMouse." AutoPaint's own description and the enableMouse page do not say this.
VERDICT:   missing
EVIDENCE:  `mfb man canvas types` AutoPaint paragraph: "present it once and the window keeps it up to date" — never mentions enableMouse. `mfb man canvas enableMouse`: "Mouse reporting is opt-in … a program that only draws pays nothing" — a reader concludes hover effects need it on. The rule lives only on Picture, where a developer building a plain AutoPaint button will not look.
SUGGESTED: Add to AutoPaint: "It does not need canvas::enableMouse; the window tracks the mouse for it on its own." Add to enableMouse: "canvas::AutoPaint does not need this — it reacts to the mouse whether or not reporting is on."

### 7. Ellipse cites a Circle rule the Circle entry does not state
UNIT:      man-pkg:canvas
PAGE:      types (canvas::Circle, canvas::Ellipse)
CATEGORY:  consistency
CLAIM:     Ellipse: "A radius of 0.0 or less draws nothing, the same rule canvas::Circle follows." Circle.radius: "The radius in pixels."
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/canvas/helper_geometry/header_kinds.rs:175` `IF radius <= 0.0 THEN` (circle header) and `:228` for radiusX — the rule is real and shared, but only Ellipse documents it, by pointing at a page that is silent. ParticleSystem.emitter also relies on "A canvas::Circle with radius := 0.0 is a single point", so a reader has three entries and one unstated rule.
SUGGESTED: Circle.radius: "The radius in pixels. 0.0 or less draws nothing."

### 8. Group depth limit is stated on presentLayers and setGroup but not on present
UNIT:      man-pkg:canvas
PAGE:      present
CATEGORY:  consistency
CLAIM:     present's Errors table lists ErrDepthExceeded; its Description never says what raises it. presentLayers: "Groups nested more than 64 deep — which is also how a group that draws itself reports — raise ErrDepthExceeded"; setGroup: "Nesting is allowed to a depth of 64, and canvas::present raises ErrDepthExceeded past it".
VERDICT:   inconsistent
EVIDENCE:  `mfb man canvas present` Description (lines 2410-2444 of the --all render) covers ErrWrongMode, ErrCanvasParticleLifetime and ErrOverflow and omits ErrDepthExceeded; the sibling presentLayers page states the same condition for the same table row.
SUGGESTED: Add to present, mirroring presentLayers: "Groups nested more than 64 deep — which is also how a group that draws itself reports — raise ErrDepthExceeded."

### 9. Three pages write the mode as Mode.Canvas, the rest as app::Mode.Canvas
UNIT:      man-pkg:canvas
PAGE:      didResize, enableMouse, pollMouse
CATEGORY:  consistency
CLAIM:     "Requires Mode.Canvas; elsewhere it raises the trappable ErrWrongMode."
VERDICT:   inconsistent
EVIDENCE:  `grep -n "Requires .Mode.Canvas" src/codegen/builtins/canvas/func_did_resize.rs:34 func_enable_mouse.rs:43 func_poll_mouse.rs:101`; the seven other gated pages and the overview write "app::Mode.Canvas". The overview itself insists types are package-qualified ("a bare DrawItem is not a known type"), so the bare spelling reads as a different name.
SUGGESTED: "Requires app::Mode.Canvas; elsewhere it raises the trappable ErrWrongMode." on all three.

Checked and found consistent: the overview's `DO WHILE … LOOP` fragment compiles and runs (`dwproj` probe printed 3); Transform rotation sign and angle convention agree across overview, present, fillStroke and the Transform entry; Paint field count of thirteen matches the types table; overview and Image/Font resource entries agree on thread affinity; the derived Functions table covers every registered member.
