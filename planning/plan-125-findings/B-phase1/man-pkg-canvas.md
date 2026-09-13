### 1. Closed-font measurement contract contradicts itself
UNIT:      man-pkg:canvas  
PAGE:      package-wide  
CATEGORY:  consistency  
CLAIM:     "`A font with no head table, or one that has since been closed, measures as all zeroes rather than failing`" and "`text naming a released font measures and draws as empty rather than faulting.`"  
VERDICT:   inconsistent  
EVIDENCE:  Rendering `mfb man canvas measureText` lists `ErrResourceClosed`; `src/codegen/builtins/canvas/func_measure_text.rs:lower_font_bytes` calls `emit_closed_guard` then `raise_error_bare("ErrResourceClosed")`, before `__canvas_measureText` can return zero metrics. The probe `mfb build --app /tmp/plan-125-scratch/B-phase1/man-pkg-canvas/closed-font` also rejected measuring the directly closed binding with `TYPE_USE_AFTER_MOVE`.  
SUGGESTED: Replace the measurement claim on both pages with: “A presented `canvas::Text` item whose font has been closed draws nothing. `canvas::measureText` requires an open font and raises `ErrResourceClosed` otherwise.”