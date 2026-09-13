### 1. Secure-root opener omitted from the overview’s handle inventory
UNIT:      man-pkg:fs
PAGE:      package-wide
CATEGORY:  discoverability
CLAIM:     "`fs::open`, `fs::openFile`, `fs::openFileNoFollow`, and `fs::createTempFile` return a `File`"
VERDICT:   missing
EVIDENCE:  `mfb man fs openWithin` printed `fs::openWithin(root AS String, relPath AS String, [mode AS String]) AS fs::File`; `src/codegen/builtins/fs/mod.rs:123` contains the quoted inventory but omits its registered `func_open_within::register` at `mod.rs:198`.
SUGGESTED: Add `fs::openWithin` to this inventory and mention it as the choice for opening a caller-provided relative name beneath a trusted directory.

### 2. Atomic-write guarantee is unconditional in the overview but conditional on its pages
UNIT:      man-pkg:fs
PAGE:      package-wide
CATEGORY:  overview-mismatch
CLAIM:     "`fs::writeTextAtomic` and `fs::writeBytesAtomic` variants stage the new contents in a temporary file and swap it in with an OS rename so readers never observe a partial write."
VERDICT:   misleading
EVIDENCE:  `mfb man fs writeTextAtomic` and `mfb man fs writeBytesAtomic` both print, immediately after their all-or-nothing claim: “The final rename is atomic when the host filesystem supports atomic rename.” The unqualified overview sentence is in `src/codegen/builtins/fs/mod.rs:115-119`.
SUGGESTED: Say “on filesystems that support atomic rename, readers see either the old file or the complete new file,” and direct readers to the atomic-write pages for the durability caveat.

### 3. `openWithin` omits the Linux fallback disclosed by its sibling
UNIT:      man-pkg:fs
PAGE:      fs::openWithin
CATEGORY:  consistency
CLAIM:     "`fs::openWithin` ... [has] a host-enforced guarantee that the result cannot escape that directory" and “a component swapped to a symbolic link after canonicalization is rejected rather than followed.”
VERDICT:   inconsistent
EVIDENCE:  `mfb man fs openWithin` printed the unconditional guarantee. `mfb man fs openFileNoFollow` / `src/codegen/builtins/fs/func_open_file_no_follow.rs:36-41` disclose that Linux `ENOSYS` falls back to `O_NOFOLLOW`, which protects only the final component. `src/codegen/builtins/fs/gen_open.rs:804-848` shows `lower_fs_open_within_helper` makes the same `openat2` call and takes the same `ENOSYS -> plain open fallback`; `open_flag_set` at `gen_open.rs:41-47` identifies Linux’s fallback flag as `O_NOFOLLOW`.
SUGGESTED: Give `openWithin` the same Linux fallback qualification as `openFileNoFollow`; do not promise whole-path containment on hosts where only the final-component check is available.