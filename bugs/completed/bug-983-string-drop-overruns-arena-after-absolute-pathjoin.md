# bug-983: a function that slices a `MUT String` and `fs::pathJoin`s it onto an owned root dies with SIGBUS in the string drop (`http::respondPath("//etc/hosts")` crashes the server)

Last updated: 2026-10-07
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety

Status: Fixed (2026-10-07, bs18-agent-i)
Regression Test: `tests/rt-behavior/fs/fs-pathjoin-absolute-owned-root-rt`; two `//`-capture TCASEs in `tests/rt-behavior/http/func_http_respondPath_valid`

A function that (1) reassigns a `MUT rel AS String` from a parameter, (2) re-slices it with
`strings::mid`, (3) joins it onto a root with `fs::pathJoin([root, rel])`, and (4) is called
with `root` an owned `String` (`fs::tempDirectory()`, not a literal) and `rel` starting with
`//` dies with SIGBUS (exit 138) when the frame drops its strings. The fault is at
`_mfb_arena_fill_random` under `_mfb_arena_free` from `_mfb_rt_drop_owned_string_cap`, writing
past the end of a mapped arena chunk. That is a free with the wrong size, or a free of the
wrong block. It happens at -O0 through -O3, and whichever way the function returns.

This is reachable remotely. `http::respondPath` has exactly this shape: it reassigns
`MUT rel`, slices off one leading `/`, then calls `fs::pathJoin([root, rel])`. A request whose
`*` capture is `//etc/hosts` (an absolute `rel` after the slice) kills the server process.

**Correct behavior:** each `String` is freed exactly once, with its own size. The function
returns its value. `http::respondPath` answers 403 for an absolute path that escapes `root`
(or 404 if it does not exist), and the server keeps running.

## Failing Reproduction

Measured by the coordinator on 2026-10-06 with main at c6dd68acc
(`/tmp/main-dbg-target/release/mfb`), macOS aarch64.

```
mfb init u   # src/main.mfb:
IMPORT fs
IMPORT io
IMPORT strings

FUNC check(s AS String, root AS String) AS Integer
  MUT rel AS String = ""
  rel = s
  IF strings::startsWith(rel, "/") THEN
    rel = strings::mid(rel, 1, len(rel) - 1)
  END IF
  LET candidate AS String = fs::pathJoin([root, rel])
  IF fs::fileExists(candidate) = FALSE THEN
    RETURN 404
  END IF
  MUT within AS Boolean = FALSE
  within = fs::isWithin(root, candidate) TRAP(e)
    RECOVER FALSE
  END TRAP
  IF within = FALSE THEN
    RETURN 403
  END IF
  RETURN 200
END FUNC

FUNC main AS Integer
  io::print(toString(check("//etc/hosts", fs::tempDirectory())))
  RETURN 0
END FUNC

mfb build -q -O <0|1|2|3> u && ./u/build/u.out; echo rc=$?
```

- Observed: no output, `rc=138` at each of `-O 0`, `-O 1`, `-O 2`, `-O 3`. It is the same
  with `"//etc/nonexistent_zz"`, which takes the `RETURN 404` path.
- Expected: `403`, rc 0.
- Controls (all rc 0):
  - `"/etc/hosts"` (only one slash, so `rel` is relative after the slice) prints `404`.
  - The same program with root `"/tmp"` (a literal) prints `403`.
  - With the `MUT rel` / `strings::mid` steps removed (`rel` passed straight in), it prints `403`.
- `mfb dbg` on a `--debug -O0` build:
  `--[XC] … F: _mfb_arena_fill_random+0x2e0 X: a bad memory access`, with backtrace
  `mfb_arena_fill_random ← mfb_arena_free ← mfb_rt_drop_owned_string_cap ← check ← main`.

The http form: a `respondPath` probe with `*` = `"//etc/hosts"` and root
`fs::pathJoin([fs::tempDirectory(), "x"])` dies the same way, with backtrace
`mfb_arena_fill_random ← mfb_arena_free ← mfb_rt_drop_owned_string ← http::respondPath`.
It is the same at main and with bs16-agent-b's bug-862 fix, which does not touch this path.

## Suspected root cause (guess, not verified)

`fs::pathJoin` restarts at an absolute component, so for an absolute `rel` its result may be
`rel`'s own block (or a view into it) rather than a fresh copy. The frame would then free that
block twice, or free it with the other string's capacity. Start by checking what
`fs::pathJoin([root, "/abs"])` returns and how `candidate`'s ownership is recorded.

## Related

- Found by the coordinator while reviewing bug-862 (`http::respondPath` existence oracle).

## Resolution

**Root cause (measured with `mfb dbg`).** Not a double free: a heap OVERRUN in
`fs::pathJoin`. `lower_fs_path_join_helper` (`src/codegen/builtins/fs/gen_path_builder.rs`)
sizes its result in pass 1 for the FINAL path, where an absolute component resets the
length to 0, and allocates exactly `total + 9` (a `String` block is freed at that size).
Pass 2 then copied EVERY component from index 0 and only rewound the cursor when it reached
the absolute one, so the components before it were written into a block sized for the
tail alone. With `root` = the 49-byte macOS temp directory and the tail `/etc/hosts` (10
bytes), pass 2 wrote ~40 bytes past the 24-byte block, over the next arena block: the
re-sliced `MUT rel` string. Its header became path text (`"7tj6bwwx"`), and the frame's
`_mfb_rt_drop_owned_string_cap` freed that "length", so `_mfb_arena_fill_random` ran off
the mapped chunk (SIGBUS). A literal `"/tmp"` root overran by too little to reach a live
block, which is why it looked like an ownership bug.

Debugger session (main compiler, `--debug -O0` build of the doc's repro):
`bp _mfb_rt_drop_owned_string_cap bt 2; list reg x0, x1; show mem [x0] 48` showed the third
drop's block holding `7tj6bwwx3tmxrgn80000gn/T/` where `/etc/hosts` had been; then
`bp _mfb_fn_check+1560` (after `rel = mid(...)` stores the new block), `wp [sp+0x38]`, `run`
stopped three times at `_mfb_rt_fs_path_join+0x14c` (the pass-2 byte copy) changing the
header `0x0a` → `0x37` → `0x7437` → `0x6a7437`.

**Fix.** Pass 1 also records `first`, the index of the last absolute component (0 when
none), and pass 2 starts there. Everything before it is discarded by the absolute rule
anyway, so the output is unchanged and the block is never overrun. The result is
byte-for-byte what it was for every input; only the components that used to be written
and then overwritten are skipped.

**`http::respondPath`.** Its `__http_pathEscapesRoot` now treats an absolute `rel` (what a
`//`-prefixed capture leaves after the one-slash strip) as an escape: 403 before any
filesystem probe. The spec already says "any escape (`..`, absolute, symlink-out) yields
`403`" and bug-862 settled that an escape is 403 whether or not it exists; answering 404
for a missing absolute target (this doc's "(or 404 if it does not exist)") would be the
bug-862 existence oracle again, so the spec wins over that parenthetical.

### Proof

| check | main f6b4099c9 | fix |
| --- | --- | --- |
| doc repro, `-O 0..3` (`./u/build/u.out`) | no output, rc=138 ×4 | `403`, rc=0 ×4 |
| `scripts/test-accept.sh … fs-pathjoin-absolute-owned-root-rt` | `[exit 138]` mismatch | passed, and at `MFB_OPT=1/2/3` |
| `func_http_respondPath_valid` (`mfb test`) | dies after case 6, rc 1 | 8/8 pass (via test-accept) |
| siblings `fs-pathjoin-rules-rt`, `func_fs_pathJoin_valid/_invalid`, `bug132_pathnormalize_root_pop` | pass | pass |

A live server (`http::route("/static/*" or "/*", …)`, curl `--path-as-is` with `//`, `///`,
`%2F` forms) answered 404 and survived on BOTH compilers: the router does not hand
`respondPath` a `//`-prefixed capture from the wire, so the crash needs a handler that
passes such a capture or `req.path` itself (as the TCASE's constructed `Request` does).
Measured, not assumed; the remote-reachability claim in the summary above is therefore
"reachable through `respondPath`'s argument", not "through the stock router".

## Golden impact

`scripts/artifact-gate.sh target/release/mfb all` with this fix on top of bug-982/993: 14
diffs beyond that run's 16, each explained:

- `byte-identity/fs` `.ncodesum` on all five targets: `_mfb_rt_fs_path_join` only
  (function-level `-ncode` diff against main, linux-x86_64).
- `.ir` of every fixture that embeds the `http` package (`byte-identity/http`,
  `rt-behavior/http/func_http_response_valid`, `http-tcp-transport-rt`,
  `packages/http-process-coexist-rt`, `syntax/http/http_async_stream_valid`,
  `syntax/resources/resource-state-qualified-import-valid`,
  `syntax/threads/func_thread_start_self_http_fanout`) and `byte-identity/http`'s
  macOS/Windows `.ncodesum`: the new absolute check in `__http_pathEscapesRoot` plus
  `_mfb_rt_fs_path_join`.
