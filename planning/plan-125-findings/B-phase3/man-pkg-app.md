### 1. `setMode` omits the usable Canvas mode
UNIT:      man-pkg:app
PAGE:      app::setMode
CATEGORY:  consistency
CLAIM:     "`mode` is one of the two `app::Mode` enum members: `app::Mode.Console` ... or `app::Mode.None`"; the package overview and types page list `Canvas` as a third member.
VERDICT:   inconsistent
EVIDENCE:  Rendered `mfb man app --all` shows the contradiction. The probe `mfb build --target linux-x86_64 /tmp/plan-125-scratch/B-phase3/man-pkg-app` successfully compiled `app::setMode(app::Mode.Canvas)` and `app::getMode() = app::Mode.Canvas`. `src/codegen/builtins/canvas/func_present.rs` also uses `app::setMode(app::Mode.Canvas)` in its examples.
SUGGESTED: State that `setMode` accepts all three members, including `app::Mode.Canvas`, and describe Canvas as the graphics surface. Update the parameter description too; “Any other type” does not tell the reader that the omitted enum member is valid.

### 2. Canvas is promised but not discoverable from `app`
UNIT:      man-pkg:app
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     The overview introduces "`Canvas` — a 2D graphics surface drawn by the `canvas` package," but its only See also entry is `mfb man app types`; neither function page directs a reader to `mfb man canvas`.
VERDICT:   missing
EVIDENCE:  Rendered `mfb man app --all` contains no `mfb man canvas` cross-reference. Rendered `mfb man canvas` says its surface is established by `app::setMode(app::Mode.Canvas)`, and `rg -n 'setMode\\(app::Mode\\.Canvas\\)' src/codegen/builtins/canvas --glob 'func_*.rs'` finds that setup throughout Canvas examples.
SUGGESTED: Add `mfb man canvas` to the package overview’s See also section and to `app::setMode`; say, “For 2D drawing, call `app::setMode(app::Mode.Canvas)` before using `canvas`; see `mfb man canvas`.”