### 1. `fs::openData` is omitted from the list of functions that return a `File`
UNIT:      man-page:fs/close
CLAIM:     "Every `File` returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile` is closed by itself when the `RES` binding that holds it goes out of scope" (and the parameter description's matching list: "as returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, or `fs::createTempFile`")
VERDICT:   incomplete
EVIDENCE:  `mfb man fs openData` declares both overloads `AS fs::File` (`src/codegen/builtins/fs/func_open_data.rs`, `return_type: ParameterType::named(super::FILE_TYPE_ID)` on both implementations), and the package overview says "fs::openData returns a File over a copy of a String or List OF Byte". Probe `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-close/opendata.mfb` (`RES d = fs::openData("line one\nline two\n", "read")` / `fs::readLine(d)` / `fs::close(d)`) built with `mfb build -q p-opendata` and printed `line one` then `closed an openData File`, exit 0. So a sixth opener produces a `File` that `fs::close` accepts, and both lists on this page present themselves as exhaustive ("Every `File` returned by …"). Because `fs::openData` is not named in the description, the derived See also list omits it too.
SUGGESTED: In the parameter description: "The open `File` resource to close, as returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, `fs::openData`, or `fs::createTempFile`. Must not already be closed or moved." In the description: "Every `File` returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, `fs::openData`, or `fs::createTempFile` is closed by itself when the `RES` binding that holds it goes out of scope, …"

Everything else on the page checked out against the binary. For the audit record, the probes under `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-close` and what they showed:

- Both page examples (`p-ex1`, `p-ex2`) build and run, printing `first line` and `hello`, exit 0.
- `uam.mfb`: naming `f` after `fs::close(f)` in the same function is rejected at build time with `TYPE_USE_AFTER_MOVE`.
- `closedfn.mfb`, `closedrec.mfb`, `closedrec2.mfb`: after a called function closed the handle, or after closing through one copy of a record, a later `fs::readLine` or second `fs::close` raised `77030004 Resource handle is already closed.`
- `buf2.mfb`: a fresh handle reports `isBuffered` FALSE. With buffering on, the file was empty before `fs::close` and held the written bytes after it. `scopebuf.mfb` showed the same drain at scope exit without an explicit close.
- `scope.mfb`: explicit close followed by scope exit ran cleanly.
- `p-moved`: closing a handle after `thread::transfer` handed it to a worker, via a second record copy, raised `77030009` with the ErrResourceMoved message.
- The ErrCloseFailed-wins-over-ErrWriteFailed ordering could not be provoked by a probe. It is confirmed by reading `lower_fs_close_helper` in `src/codegen/builtins/fs/gen_handle.rs`, where the close-result branch is tested before the saved drain result.
- A whole-word sweep of the rendered page against `scripts/man-census.sh --banned-list` produced no hits.
