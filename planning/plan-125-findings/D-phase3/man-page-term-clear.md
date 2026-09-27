### 1. Linux GTK clear presents without `term::sync`
UNIT:      man-page:term/clear
CLAIM:     "The blanked screen appears when the program calls `term::sync`."
VERDICT:   wrong
EVIDENCE:  `src/target/linux_gtk/app_io.rs:emit_app_term_clear` calls `g_idle_add(TERM_REDRAW_IDLE_SYMBOL)` after clearing; `src/target/linux_gtk/term_draw.rs:emit_term_redraw_idle_helper` snapshots the grid and calls `gtk_widget_queue_draw`. Thus Linux GTK app builds schedule a present directly from `term::clear`, without `term::sync`.
SUGGESTED: On the console and macOS app builds, the cleared surface appears at the next `term::sync`; Linux GTK app builds schedule a redraw when `term::clear` runs.