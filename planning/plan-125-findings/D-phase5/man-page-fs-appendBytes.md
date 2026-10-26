### 1. `bytes` parameter leaks the list's internal layout
UNIT:      man-page:fs/appendBytes
CLAIM:     "The bytes to append, in order, taken verbatim from the list's data region after the file's existing contents."
VERDICT:   out-of-scope
EVIDENCE:  "data region" is the codegen name for the byte-List payload past the collection header (`src/codegen/builtins/fs/gen_atomic_write.rs:lower_fs_write_path_helper`, the `COLLECTION_HEADER_SIZE` / `COLLECTION_OFFSET_DATA_LENGTH` block). A developer has no "data region" in their model of a `List OF Byte`; §3 bans layout vocabulary. The sentence is also garbled: "taken verbatim from the list's data region after the file's existing contents" reads as if the region sits after the file. Probe (`/tmp/plan-125-scratch/D-phase5/man-page-fs-appendBytes/probe`, `all256` case) confirms the developer-visible fact: a list holding 0..255 produced a 512-byte file after two calls whose `xxd` starts `0001 02…` and ends `…fdfe ff`.
SUGGESTED: "The bytes to append, written after the file's existing contents in list order, each exactly as held. An empty list leaves the file's length unchanged. The list itself is not changed."

### 2. Path rule justified by the host `open` call
UNIT:      man-page:fs/appendBytes
CLAIM:     "The string must not be empty and must not contain an embedded NUL byte, because the host `open` call requires a NUL-terminated path."
VERDICT:   out-of-scope
EVIDENCE:  The rule is right (probe: empty path and `"a" & NUL & "b"` both raise 77050002 ErrInvalidArgument, from the `invalid` label in `lower_fs_write_path_helper`), but the "because the host `open` call…" clause explains it via the C API the compiler marshals to, which only matters to someone reading the lowering (§1, §3). Every other sentence in that paragraph is developer-facing; this one is not.
SUGGESTED: "The string must not be empty and must not contain an embedded NUL byte; either raises `ErrInvalidArgument` before the file is touched."

### 3. "append flag" paragraph is host-API framing and repeats itself
UNIT:      man-page:fs/appendBytes
CLAIM:     "The file is opened with the append flag set, so every write is positioned at the current end of the file. The bytes are written exactly as the list holds them. […] Bytes are written exactly as held in the list, with no encoding, decoding, or newline translation, so the function is suitable for binary data as well as text."
VERDICT:   misleading
EVIDENCE:  "the append flag" names an `open(2)` flag (`gen_open.rs:open_flag_set`, `append = O_WRONLY|O_CREAT|O_APPEND|O_CLOEXEC`) rather than the thing a developer can observe, which is that each call lands at whatever the end of the file is at write time, even if another process has grown it since. The paragraph also states "written exactly as the list holds them" twice in five sentences, and "an empty list leaves the file's length unchanged" appears three times on the page (parameter cell, this paragraph, and the first paragraph). Verified true by probe: `all256 again` doubled the file to 512 bytes; `empty list new file` created `empty.bin` at 0 bytes (`stat -f '%Sp %z'` → `-rw------- 0`).
SUGGESTED: Replace the paragraph with: "Each write lands at the current end of the file, so appends from several programs at once do not overwrite one another. Bytes are written exactly as held in the list, with no encoding, decoding, or newline translation, so the function suits binary data as well as text. If the host accepts only part of a write, the rest is retried until every byte is written or the host reports a failure; an interruption never loses or duplicates bytes. An empty list creates the file if needed and otherwise changes nothing." Drop the duplicate "empty list" sentence from the first paragraph.

### 4. Windows behaviour for a directory path cannot be verified here
UNIT:      man-page:fs/appendBytes
CLAIM:     "and `ErrWriteFailed` for any other refusal — including `path` naming a directory, which Windows reports as `ErrAccessDenied`."
VERDICT:   UNVERIFIED
EVIDENCE:  macOS half confirmed by probe: `directory as path` (an existing directory `adir`) raised 77020002 ErrWriteFailed, matching the POSIX branch of `gen_shared.rs:emit_errno_error_mapping` (EISDIR=21 falls through to ErrWriteFailed). The Windows branch maps `GetLastError` 5 to ErrAccessDenied, and `CreateFile` on a directory without backup semantics does return ERROR_ACCESS_DENIED, so the sentence is consistent with the code; but I have no Windows host and did not run it. Main thread may want a Windows probe or a citation in the ledger.
SUGGESTED: Keep the sentence; record the Windows case as unverified-by-run in the phase ledger.

### 5. Mutation-vs-value edge not stated
UNIT:      man-page:fs/appendBytes
CLAIM:     (absence) — nothing on the page says whether `bytes` is left unchanged.
VERDICT:   incomplete
EVIDENCE:  §2 requires "mutation vs. new value" stated explicitly. The lowering only reads the list (`load_u64` of length/capacity, then the write loop over `cursor`/`remaining`; no store into `value`) and returns Nothing, so the list is untouched; the page never says so.
SUGGESTED: Add to the `bytes` cell (see finding 1) or the first paragraph: "The list itself is not changed."

Everything else on the page held under probe: created files get mode `0600` under umask 022 (`all.bin`, `empty.bin`, `héllo_日本.bin`, `output/log.bin` all `-rw-------`), an existing `0644` file kept its mode, appending through `link.bin` grew `target.bin` from 1 to 3 bytes, a missing directory raised ErrNotFound, a read-only file and a `chmod 000` directory both raised ErrAccessDenied, a Unicode file name worked, and both page examples compiled with `mfb build` and ran twice each (`output/log.bin` → `0a0a`; `combined.bin` → the two lines twice).
