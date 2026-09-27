### 1. Compiler-detail terminology
UNIT:      man-page:term/hideCursor  
CLAIM:     “Like everything else on this retained surface, the call emits no escape sequence.”  
VERDICT:   out-of-scope  
EVIDENCE:  `src/codegen/builtins/term/func_hide_cursor.rs:DESC` is rendered verbatim; “retained surface” and escape-sequence emission describe implementation mechanics, not behavior a MFBASIC developer needs to write a program.  
SUGGESTED: “In a terminal program, the change is shown when `term::sync` presents the next frame.”

### 2. `sync` is not universally when the change reaches the display
UNIT:      man-page:term/hideCursor  
CLAIM:     “It clears a single visibility flag in the module's state; the terminal is only told about it when `term::sync` presents a frame, whose trailing sequence shows or hides the cursor according to this flag.”  
VERDICT:   wrong  
EVIDENCE:  `src/target/linux_gtk/app_io.rs:emit_app_term_set_cursor` stores the visibility setting and calls `g_idle_add` to schedule a redraw directly from `term::hideCursor`, without `term::sync`. The console probe did confirm deferred terminal output: `probe-no-on` printed only `after\n`; `probe-frame` contained `ESC[?25l` only after `term::sync`.  
SUGGESTED: “In a terminal program, `term::sync` presents the cursor visibility selected by this call. App-window backends may redraw when the setting changes.”

### 3. The claimed previous-frame behavior is false for the GTK app backend
UNIT:      man-page:term/hideCursor  
CLAIM:     “Until the next present the cursor stays as the previous frame left it.”  
VERDICT:   wrong  
EVIDENCE:  `src/target/linux_gtk/app_io.rs:emit_app_term_set_cursor` schedules `TERM_REDRAW_IDLE_SYMBOL` immediately after setting cursor visibility; no `term::sync` is required for that redraw.  
SUGGESTED: “In a terminal program, the cursor’s visibility changes on the next `term::sync`.”