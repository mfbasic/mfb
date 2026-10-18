Every one of the open bug documents in `bugs/` is in exactly one table below, grouped
by what the bug affects (check below; it must print the count and an empty list).

Priority is SECURITY, HIGH, MED or LOW: SECURITY when the bug document's `Class:` is
Security, otherwise its `Severity:` line. Rows are ordered by priority, then number;
ordering constraints between bugs, owner decisions and open owner questions are in the
description.

Rules: a bug found while working is filed and gets a row in the table of the area it
affects, in the same commit. A fixed bug's row is removed when its document moves to
`bugs/completed/` (git history keeps the record). A bug in no table does not get done.

Check that every open bug has exactly one row:

```
python3 -c "import re,glob;a=[re.search(r'bug-(\d+)',f).group(1) for f in glob.glob('bugs/bug-*.md')];b=re.findall(r'^\|\s*\[.\]\s*\|\s*(\d+)\s',open('bugs/status.md').read(),re.M);print(len(a),sorted(set(a)^set(b))+[n for n in set(b) if b.count(n)>1])"
```

## Phases (bug-staging-24)

The list of a running phase is fixed when its first agent starts; nothing joins it.
Bugs found during a phase are filed into a later phase. Order is High to Low, top to bottom.

* HIGH 688 and 765: ON HOLD (owner, 2026-10-17). Not scheduled.
* SECURITY rows (980, 1017, 1042, 952, 954+947, 955, 918): not scheduled; they wait for the owner.
* MED 1082: blocked on hardware and a design decision (its doc). Not scheduled.
* Phase 1 (MED): 966, 965, 1015, 985, 964 - on main (0dc51751f).
* Phase 2 (MED): 1085, 1088, 1089 - on main (7bb985ed8). 1087 and 1091
  merged to staging (batch 3).
* **Phase 3 (MED, table order):** 1098, 1099, 832, 846, 847, 1007 + 1008 (one fix), 1009, 1010,
  1011. 847's run-time collision and the 1009/1010/1011 capacity rule wait on the owner.
* Later phases: the rest of MED, then LOW, in table order.

## Resources and STATE

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1029   | LOW      | a call through a function value that returns a resource union leaks the 16-byte union box |
| [ ]  | 1031   | LOW      | `RES` on a non-resource type is not refused at the top level, on a return type, or on a record field that also carries `STATE` |
| [ ]  | 1073   | LOW      | a function-level `List OF RES` keeps its 48-byte block at exit when the function has any inline `TRAP` (constant) |

## Canvas and graphics

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [*]  | 688    | HIGH     | Vulkan canvas parity with Metal: V1 to V4 fixed and Windows presents through a swapchain. The 10,000-item render on the Intel GPU (box 206) is down from 106 to 58 ms a frame for squares and circles and from 108 to 92 for the sweep's polygon quads (bug-staging-23: the coverage shader's centre sample is straight-line code), still as large as the program's own 94 ms a present, so not closed; what is left is the size of the one live shader and the polygon edge walk. The cold pipeline compile went from 35 s to 67 s (bug-996 takes it off the first frame). Linux present is bug-1082 |
| [*]  | 765    | HIGH     | Canvas renders 100,000 color-changing dots under 30 PPS / 30 FPS. The frame cap is gone (2026-10-12: a frame past one frame buffer is drawn in passes, every frame a GPU frame on Metal and Vulkan): 15 to 19 FPS on Metal, 4 to 5 on box 206. Left: the worker's copy and compare, geometry rebuilt on a fill-only change (planning/plan-175-A), the per-frame block upload, and the Intel GPU's own draw time |
| [ ]  | 1082   | MED      | on Linux a windowed Vulkan canvas frame is read back and blitted, never presented through a swapchain (split from 688; blocked on a real-GPU Linux box and the GTK4 surface decision) |
| [ ]  | 1086   | LOW      | the Vulkan canvas path reads a frame back without testing `vkQueueSubmit`'s or `vkQueueWaitIdle`'s result (Metal's twin fixed under 1024) |
| [ ]  | 1090   | LOW      | under a huge magnification the zero-gradient coverage fallback lights a stray column of pixels far from the shape (191 pixels at x = 450 for two off-surface squares at a 14,000 times transform); software and Metal |
| [ ]  | 1103   | LOW      | a banded polygon drawn under a group matrix renders about 1.4x slower on Metal since bug-1085 (owner accepted the cost; speed-up wanted) |

* Partial fix landed: 10,000 squares 110 → 58 ms on box 206. Render is still about 93 ms at 10,000 moving quads.
* Frames draw in passes, so 100,000 dots run on the GPU: 20 FPS on Metal, 4 to 5 on box 206. `planning/plan-175-A-canvas-geometry-identity-split.md` has all four decisions recorded.

## App windows and GTK

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1098   | MED      | a Windows `--app` build always exits with status 0: `main`'s `Integer` result is dropped at the bootstrap's `ExitProcess(0)` (console and macOS `--app` keep it) |
| [ ]  | 960    | LOW      | `mfb build --app` bundles fail `codesign --verify` ("code has no resources but signature indicates they must be present") |
| [ ]  | 972    | LOW      | closing an `--app` window (or Cmd-Q) ends a running program with exit status 0 and no teardown — buffered file output is lost, no `--debug` report (owner decided: new `app::shouldClose`) |
| [ ]  | 997    | LOW      | Windows `--app` windows open at 400×300 (a 384×261 client) while macOS and Linux open at 900×640 |

## Terminal and input

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1099   | MED      | in a terminal, `term::pollMouse` sees no events unless the program contains an `io::` reader, even one that never runs (the main thread is not subscribed to stdin) |
| [ ]  | 969    | LOW      | `term::getForeground` / `getBackground` (all app backends) and `term::isOn` (macOS) do not raise `ErrWrongMode` outside `Mode.Console`, though they declare it (after 966) |
| [ ]  | 971    | LOW      | the SGR mouse decoder and the key decoder accept malformed/overflowing numeric parameters |
| [ ]  | 973    | LOW      | in an `--app` window, `io::readKey` right after `io::readLine`/`io::input` (or a canvas `readLine`) first returns the Return key's `Release`, a release whose press was never reported |

## Collections

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 832    | MED      | a `Set OF T { … }` literal skips the element-comparability check (non-comparable T accepted; ICE when non-empty with a List element) |
| [ ]  | 846    | MED      | field access on a MapEntry / List / Map escapes the verifier and fails in native lowering with an uncoded, unlocated internal error |
| [ ]  | 847    | MED      | a `Map OF K TO V { … }` literal with a repeated key builds a map holding that key twice |
| [ ]  | 1007   | MED      | fixed-width `collections::chunks` fast path allocates each chunk at the requested size, even when it holds only a few elements |
| [ ]  | 1008   | MED      | valid large constant `chunks`/`window` sizes and strides fail during native code generation (codegen panic) |
| [ ]  | 1009   | MED      | fixed-width `collections::toSet` reserves by input length even when nearly every element is a duplicate |
| [ ]  | 1010   | MED      | native `collections::merge` reserves for every input entry even when all keys overlap |
| [ ]  | 1011   | MED      | reserved collection results retain source-sized capacity when outputs are smaller |
| [ ]  | 1036   | MED      | a list reassigned inside a `FOR EACH` over it leaks every replaced block (owner decided: no intentional leaks; free them when the loop ends) |
| [ ]  | 1002   | LOW      | `TYPE_COLLECTION_OWNERSHIP_VIOLATION` is reported twice for one collection declaration that holds a thread worker |
| [ ]  | 1006   | LOW      | `collections::window` overflows while advancing past its last window for a large valid stride |

## Strings and Unicode

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 905    | MED      | `strings::caseFold` (and regex `(?i)`) fold with Unicode 9.0 data — 184 scalars never fold |
| [ ]  | 887    | LOW      | `AttributedString & String` fails with an unlocated diagnostic naming the internal helper `#astrings_concat` |
| [ ]  | 888    | LOW      | `astrings::toMarkdown` renders an empty font name (documented as allowed) exactly like an omitted slot, so `font("")` + size reads as "font unchanged" |
| [ ]  | 906    | LOW      | runtime grapheme segmentation treats 5,222 unassigned code points as Other — diverges from UAX #29 and from the folded path |
| [ ]  | 1005   | LOW      | `strings::repeat("", n)` spends O(n) time copying zero bytes |

## Numbers (Float, Fixed, Byte, Money)

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 831    | MED      | `toFixed(String)` parses through binary64 — wrong value near the top of the range, and rejects in-range text with ErrOverflow |
| [ ]  | 845    | MED      | MATCH on a Float compares bit patterns, so `CASE 0.0` misses `-0.0` although `-0.0 = 0.0` is TRUE |
| [ ]  | 931    | MED      | `math::pow(Fixed, Fixed)` raises ErrOverflow for a negative whole exponent whose result is tiny (`pow(2.0F, -31.0F)`, `pow(10.0F, -10.0F)`) |
| [ ]  | 932    | MED      | `math::atan2(y, -0.0)` returns the wrong sign: `atan2(1.0, -0.0)` is -pi/2, `atan2(-1.0, -0.0)` is +pi/2 |
| [ ]  | 837    | LOW      | `toString` argument-mismatch diagnostic lists accepted types without `Money` |
| [ ]  | 839    | LOW      | `LET b AS Byte = -0` is `TYPE_BINDING_MISMATCH`, while the spec and the range checker treat `-0` as an in-range Byte literal (with 978) |
| [ ]  | 878    | LOW      | `toString(Fixed, n)` does not round the exact Q32.32 value: it truncates at n >= 10 (not "trailing zeros") and over-rounds just below a midpoint at n < 10 (after 977) |
| [ ]  | 879    | LOW      | `mfb test` equality-failure detail renders Float/Fixed/Money with 2 decimals, so a failing `expectFloat` reads "expected 0.30, got 0.30" |
| [ ]  | 934    | LOW      | `math::min`/`math::max` on `Float` give a different signed zero on x86-64 than on AArch64/riscv64 (`min(-0.0, 0.0)` prints `0.00` on x86, `-0.00` elsewhere) |
| [ ]  | 977    | LOW      | `toString`'s `precision` is a `Byte` — a number typed as data; make it an `Integer` with a checked 0..32 range (owner-confirmed language change; first of 977 → 878, 978 + 839, 945) |
| [ ]  | 978    | LOW      | there is no `Byte` literal suffix, and an Integer literal silently becomes a `Byte` argument — add `3b` and stop the coercion (owner-confirmed language change) |

## Data formats (datetime, JSON, CSV, base64)

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 827    | MED      | datetime::parse `f` token scales by run length, not digits read — ".12" under "fff" parses as 12 ms, ".1" under 9×f as 1 ns |
| [ ]  | 828    | LOW      | json::parse depth cap admits 257 nested containers when the innermost is empty |
| [ ]  | 829    | LOW      | csv::stringify → csv::parse drops a trailing single-empty-cell row and turns a zero-cell row into a one-cell row |
| [ ]  | 869    | LOW      | `encoding::base64Decode` accepts excess `=` padding (`"AAAA===="`, `"AA======"`) |

## Crypto and TLS

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 863    | MED      | `crypto::open(cipher, key, nonce, sealed, aad)` — the documented Sealed+aad overload fails to compile with an unlocated TYPE_CALL_ARGUMENT_MISMATCH |
| [ ]  | 864    | MED      | `tls::listen` / `http::serverSSL` fail with ErrTlsFailed for an EC (P-256) PKCS#8 key on macOS |
| [ ]  | 871    | LOW      | `crypto::hmac(crypto::Hash.SHA1, key, "text")` reports CRYPTO_SHA1_INSECURE — the String-data overload misses the sound-use exemption |

## Networking and HTTP

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 980    | SECURITY | the HTTP chunked decoder accepts framing llhttp (and RFC 9112) rejects (builds on 859's rule) |
| [ ]  | 1017   | SECURITY | on Windows, `http::respondPath` still answers 403 vs 404 by existence for a `..\` traversal (bug-862's oracle, through a backslash) |
| [ ]  | 1042   | SECURITY | `net::toUrl` does no IDNA (UTS #46) host processing: a non-ASCII or `xn--` host is stored as written (what bug-979 left; owner decided: a documented difference for now) |
| [ ]  | 876    | MED      | `tcp::connect`/`tcp::listen`/`udp::bind` accept any Integer port and silently use its low 16 bits |
| [ ]  | 865    | LOW      | http server emits the handler's stale reason phrase (`418 OK` from the spec's own teapot example) and any status integer verbatim (`HTTP/1.1 -5 OK`, `1000`, `0`) |
| [ ]  | 866    | LOW      | `http` route `:name` binds an empty segment — `/t/:id` matches `/t//` with `id = ""` |
| [ ]  | 867    | LOW      | the http server's 8 KiB-line and 100-field caps are enforced only after the head is complete |
| [ ]  | 872    | LOW      | an IPv6-literal URL parses but cannot be used (the connect half) |
| [ ]  | 986    | LOW      | an http client HEAD request waits for the server to close when the response carries a non-zero Content-Length (owner decided: a `method` field on `http::PendingState`) |
| [ ]  | 1003   | LOW      | the `http::pump` man example can hang forever: it fetches a live remote (example.com) with no deadline (flake) |

## File system, I/O and OS

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 886    | MED      | macOS `os::executablePath` / `os::appResourcePath` return the unresolved invocation path |
| [ ]  | 891    | MED      | io::readLine/io::input deliver the tail of an ill-formed UTF-8 line as a separate, error-free line |
| [ ]  | 892    | LOW      | fs::readAll / fs::readAllBytes raise ErrReadFailed after fs::seek past end of file |
| [ ]  | 893    | LOW      | fs::canonicalPath / fs::isWithin accept "file/" and "file/.." on macOS but raise ErrInvalidPath on Linux |

## Types, records and unions

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 874    | MED      | `typeName(Enum.Member)` with a literal enum member fails the build with an internal codegen error (after 835) |
| [ ]  | 945    | MED      | `Result` and `Ok` are reserved in user code — a user cannot declare their own `TYPE Result` or `TYPE Ok` (owner-confirmed language change; after 977 and 978) |
| [ ]  | 1021   | MED      | a by-field record constructor must name every field (owner decided: omitted fields take their defaults) |
| [ ]  | 855    | LOW      | walking a recursive union with `MUT cur = c.rest` deep-copies the remaining tail every step — O(n²) time and allocations |

## Templates and overloads

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 833    | MED      | a parameter of user-template type (`Stack OF T`, `Pair OF A, B`) cannot be followed by another parameter |
| [ ]  | 913    | MED      | two template FUNCs sharing a name (or a template and a plain FUNC) — the earlier declaration is silently discarded, calls bind to the wrong body |
| [ ]  | 914    | MED      | the same import alias bound to different packages in two files breaks imported-overload resolution (`TYPE_UNKNOWN_VALUE`), depending on file sort order |
| [ ]  | 1022   | MED      | a template call (`collections::merge`, any user `FUNC f OF T`) rejects a union variant where its `T` is the union, though the builtin collection members now accept it |
| [ ]  | 1038   | MED      | an imported return-type overload set cannot be selected by the expected type (`TYPE_OVERLOAD_AMBIGUOUS`) |
| [ ]  | 1039   | MED      | a template call whose argument is a direct imported-package call infers `T` as `Unknown` |
| [ ]  | 1060   | MED      | a direct call of an overloaded FUNC whose matching overload takes a `STATE` parameter fails with `SYMBOL_UNKNOWN_IDENTIFIER: Callable `p` is not a top-level function` |
| [ ]  | 1061   | MED      | a generic `FUNC bump OF T(x AS fs::File STATE Cur, v AS T)` cannot be called: `cannot infer template arguments from fs.File STATE Cur` (near 1032) |
| [ ]  | 836    | LOW      | a call that matches no user overload reports `SYMBOL_UNKNOWN_IDENTIFIER: Callable `f` is not a top-level function` |
| [ ]  | 870    | LOW      | a return-type overload (`encoding::utf8Encode`) loses its expected type under an inline TRAP |
| [ ]  | 1032   | LOW      | a generic `FUNC gen OF T(x AS T STATE Cur)` called with a `fs::File STATE Cur` argument is rejected; the instantiation carries the clause twice (NEEDS AN OWNER DECISION: is `T STATE S` on a generic parameter legal at all?) |

## Modules, visibility and entry point

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 848    | MED      | a `PUBLIC` field of an `EXPORT TYPE` is readable, constructible and `WITH`-updatable by importers |
| [ ]  | 849    | MED      | `WITH` can overwrite a `PRIVATE` record field from another file or an importing project |
| [ ]  | 920    | MED      | Source selection with a directory-symlink alias depends on `read_dir` order (files silently dropped or renamed) |
| [ ]  | 881    | LOW      | a PUBLIC and a PRIVATE declaration of one name in the same file report `SYMBOL_DUPLICATE_TOP_LEVEL` with the internal mangled name (after 912) |
| [ ]  | 924    | LOW      | Overloaded (or `PRIVATE`) `main` is reported as "must declare an entry point named `main`"; the "exactly one" diagnostic is unreachable |
| [ ]  | 1040   | LOW      | a `PRIVATE FUNC` named like a built-in (`len`) hides the built-in in its file, and the error prints the mangled name |

## Globals and initializers

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 875    | MED      | calling a non-function top-level binding is not `SYMBOL_NOT_CALLABLE` — statement form dies in the linker |
| [ ]  | 900    | MED      | a runtime helper called only in a module-level initializer fails the build with "NIR runtime call requires undeclared helper" |
| [ ]  | 901    | MED      | a module-level initializer that reads stdin fails with `ErrInvalidContext` — the main thread is subscribed only after global initialization |
| [ ]  | 1068   | MED      | the heap values of module-level bindings (lists, strings, resource records, `STATE` payloads) are not freed at exit; owner ruled they are held until exit and must then be freed by the teardown (only what the module scope owns, never everything live), because the free overwrites the memory |
| [ ]  | 1081   | MED      | a module-level `List OF RES` that is reassigned keeps every block it replaces, 240 bytes per assignment (OWNER QUESTION inside: should a replaced global's resources be released at the assignment, or held until teardown as spec 15.6 says?) |
| [ ]  | 903    | LOW      | a user `FUNC __mfb_init_globals_<project>` collides with the compiler's global initializer — unlocated internal error |

## Inline TRAP and runtime errors

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1035   | MED      | a function's emitted code is quadratic in its fallible statements (every exit path repeats the cleanup) and the compiler holds about 700 bytes per instruction, so 150 inline TRAPs peak at 1.14 GB (what bug-860 left; owner decided: share cleanup tails, which moves goldens, and cut the allocator's memory) |
| [ ]  | 835    | MED      | `typeName(x)` under an inline `TRAP` fails the build with an internal error (`native code plan does not emit runtime helper '_mfb_rt_general_typeName'`) |
| [ ]  | 838    | LOW      | `ErrStackOverflow` carries `source.line = 0`, `source.char = 0` — no usable origin location |

## Lambdas, native wrappers and function values

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 915    | MED      | a non-predicate built-in (`toString`, `len`, `io::print`) passed as a callback argument fails with the unlocated internal error `NIR local reference '…' does not resolve` |
| [ ]  | 877    | LOW      | a lambda assignment body targeting a captured `LET` reaches the NIR validator as an internal error |
| [ ]  | 880    | LOW      | a native `LINK` wrapper with more than eight MFBASIC parameters fails the build with an internal aarch64 error, regardless of its ABI slot count |

## Diagnostics quality

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1100   | MED      | `WITH r { z := … }` naming a field the record does not have passes every check and fails in native lowering with no code and no location |
| [ ]  | 882    | LOW      | `LET x = []` binds `List OF Unknown` silently, and the first `collections::append(x, v)` then fails the build with an internal error |
| [ ]  | 936    | LOW      | wrong-arity `collections::merge`/`partition` diagnostics print the internal monomorph name `#collections_merge$String$Integer` |
| [ ]  | 937    | LOW      | one unknown record field produces cascading `TYPE_CALL_ARGUMENT_MISMATCH` (overloaded builtin) and a duplicated `TYPE_UNKNOWN_VALUE` |
| [ ]  | 1041   | LOW      | a no-match call on a file's `PRIVATE`+`PUBLIC` overload set prints `#<hash>$r`, and `PRIVATE_SHADOWS_PUBLIC` fires on that legal overload (item 2 NEEDS AN OWNER DECISION; with 836 and 881) |
| [ ]  | 1058   | LOW      | `wantsCur(1)` (an Integer to an `fs::File STATE Cur` parameter) reports a second, misleading `TYPE_STATE_MISMATCH` after the argument mismatch |
| [ ]  | 1062   | LOW      | `TYPE_STATE_MISMATCH` for a bare file given to an imported FUNC value (`LET j = pkg::bump`) has no file and line |

## Codegen internals

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 946    | MED      | `Operand::Raw` keeps operands as untyped strings — remove the arm and give every operand kind a typed variant (together with 930; do after the behaviour fixes so its diffs are measured on a stable tree) |
| [ ]  | 930    | LOW      | the `.mir` dump documented as "target-neutral" contains the runtime helpers, which are hand-written per architecture with physical register names |
| [ ]  | 1080   | LOW      | a defaulted record updated with `WITH` costs about 27 KB of machine code a statement (the program is right; the executable is large and slower to build) |
| [ ]  | 1084   | LOW      | `toString` of a package enum read straight off a call result fails to build ("native code string literal has no data object"); binding the record first works |

## Packages and registry

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 952    | SECURITY | `/signing` signs and permanently logs attestations for idents/versions that the registry's REPO-17 charset forbids |
| [ ]  | 954    | SECURITY | package idents differing only in owner-prefix case are distinct packages, so a former owner can re-publish a transferred package's name (also covers 947) |
| [ ]  | 955    | SECURITY | ident-authorized mutations (release-state, org role, token revoke) sign no nonce, so a captured signature can be replayed with any live session |
| [ ]  | 947    | MED      | `/log/publish` returns another release's entry for idents/versions that differ only in ASCII case |
| [ ]  | 948    | MED      | owner names of 241–255 bytes pass validation but cannot register (246+) or ever rotate their ident (241–245) |
| [ ]  | 949    | MED      | an empty `MFB_HOME` (or empty `HOME`) puts the private key store in the current directory |
| [ ]  | 1094   | MED      | a builtin package's code and globals are linked once per importing package even when identical: `color` used by a package and its program is in the binary twice (45 + 45 functions, 132,096 bytes release); `browser.out` carries 668,700 duplicate bytes. Each copy also has its own builtin globals - possible split `canvas` state (unverified) |
| [ ]  | 856    | LOW      | §15 requires compiled cleanup metadata for drop-close failures, but the `.mfp` cleanup table is always empty, so `pkg info`'s secondary-close audit line can never fire (owner decided: remove the `.mfp` cleanup table and its `FUNCTION_TABLE` fields (format bump); derive the audit) |
| [ ]  | 907    | LOW      | a package whose IR enum members disagree with its TYPE_TABLE is accepted, and `toString(<enum>)` then returns another member's name |
| [ ]  | 908    | LOW      | FUNCTION_TABLE parameter whose `paramFlags` and `defaultConst` disagree is accepted — ABI hash says "has default", importer says "no default" |
| [ ]  | 909    | LOW      | a malformed `.mfp` that only fails the lossy metadata readers is reported as an unrelated type/symbol/manifest error, never as an invalid package |
| [ ]  | 916    | LOW      | `mfb pkg verify` reports a working `file:<dir>` source dependency as `Invalid Package`, and exits 0 when a dependency is `Invalid Package` or `Needs Update` |
| [ ]  | 925    | LOW      | `mfb pkg install` with no registry dependencies but a leftover `mfb.lock` fails instead of "nothing to install" |
| [ ]  | 940    | LOW      | `mfb repo link --start` with no owner runs the new-machine pairing flow with owner `--start` |
| [ ]  | 943    | LOW      | `mfb pkg remove <name>` on a declared local (`file://`) package says it "is not declared in project.json" |
| [ ]  | 950    | LOW      | the registry publishes a package whose header identity disagrees with its embedded manifest, and every client then refuses to install it |
| [ ]  | 951    | LOW      | `/search` matches description text from the newest release of any state but shows the newest *active* release's description |
| [ ]  | 953    | LOW      | `mfb repo transfer` tells the recipient to run a `transfer-accept` command that always fails |
| [ ]  | 956    | LOW      | package-manager spec (01–04) drift and small protocol status/message defects (after 950) |

## Audit

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 918    | SECURITY | `mfb audit` misses `AUDIT-TLS-RELAXED-TRUST` when `allowSelfSigned` is passed positionally |
| [ ]  | 850    | MED      | `mfb audit` exits 3 with spurious errors on any project that uses an imported package's exported types or globals, so `EXPORT MUT` is never surfaced |
| [ ]  | 919    | MED      | `mfb audit` exits 0 with `errors: 0` for a program that does not type-check |
| [ ]  | 941    | MED      | `mfb audit --locked` always fails a project with no registry dependencies, the state where `pkg update` deletes `mfb.lock` (after 925) |

## Formatter

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 921    | LOW      | `mfb fmt` deletes a comma-only line inside a bracket, shifting every later source line (changes `e.source.line` and test output) |
| [ ]  | 922    | LOW      | `mfb fmt` deletes a trailing comment on an `END LINK` line |
| [ ]  | 926    | LOW      | `mfb fmt` turns each leading tab in a DOC `EXAMPLE` into one space, changing the rendered example |

## Debugger and debug report

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 939    | LOW      | `mfb dbg` accepts a breakpoint at an address that is not a multiple of 4 and corrupts two instructions |
| [ ]  | 942    | LOW      | debug report `perf` durations come from macOS `CLOCK_MONOTONIC`, which has microsecond resolution, so `mfb_alloc`/`mfb_free` read as 0 ns |
| [ ]  | 1004   | LOW      | `cli_dbg::macos::an_attached_threaded_program_lists_its_threads_and_arenas` flakes: a fixed 1 s sleep does not guarantee the worker has created its arena (flake) |

## Build, linker and platform

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 958    | MED      | Mach-O `__LINKEDIT` layout is non-canonical — `strip`, `install_name_tool`, `dyld_info` refuse every mfb macOS executable |
| [ ]  | 853    | LOW      | importing a source dependency whose manifest is `kind: "executable"` first compiles and links it into its own `build/`, then refuses it |
| [ ]  | 923    | LOW      | A `libraries` locator with `libc` on `os: "windows"` passes validation but can never match |
| [ ]  | 959    | LOW      | `LINK … AS win` with a function named `pathbuf`/`vendorsep` fails the Windows vendoring build with an internal duplicate-symbol error |
| [ ]  | 1096   | LOW      | a program that calls `math::seed` but never `math::rand` still draws OS entropy at startup and links the unused `_mfb_rng_next` (all five targets) |
| [ ]  | 1097   | LOW      | linux-x86_64 declares a libc `getentropy` import for `math::rand`/`math::seed` that no emitted code calls (the seed uses the raw `getrandom` syscall); left by bug-71 |

## Test tooling

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 1059   | MED      | two GPU text tests in `rt_canvas_font` fail with main's compiler on the development Mac (6 of 6 runs on 2026-10-11; they passed in the batch 20 gate hours earlier and 5 of 5 times on 2026-10-12; cause not traced; the same two tests as bug-1024, closed in `bugs/completed/`) |
| [ ]  | 987    | LOW      | the Windows test scripts still default to the retired box 2230 (owner decided: repoint to box 206) |
| [ ]  | 1014   | LOW      | `field_kinds.tsv` is not what `field_kinds_gen.py` generates |
| [ ]  | 1033   | LOW      | the man example `variable::guide#2` does not build |
| [ ]  | 1095   | LOW      | `scripts/test-appimage.sh` fails three checks on box 2226 with a correct compiler: two need `readelf`, which the box lacks, and the liveness check assumes ssh cannot reach a display |

## Spec and man drift

| Done | Number | Priority | Description |
| ---- | ------ | -------- | ----------- |
| [ ]  | 830    | LOW      | stdlib spec drift — regex/datetime/csv/json topics contradict the code |
| [ ]  | 840    | LOW      | language spec §4/§8 cite drifted `table.rs` line numbers and a deleted `resolve_type_name` |
| [ ]  | 857    | LOW      | language spec §9–§15 drift — stale claims, misplaced/dangling citations, nonexistent §15.4 (was waiting on 975, which is fixed) |
| [ ]  | 873    | LOW      | stdlib spec topics http/url/math-rng/encoding/vector drift from the code — two stale line citations and a dozen inaccurate claims (close after the networking bugs) |
| [ ]  | 883    | LOW      | diagnostics/rule-codes spec drift (counts, a dangling "Code Collisions" section, stale shared-code prose) and seven registry rules that nothing emits but the spec presents as live |
| [ ]  | 884    | LOW      | language spec §16–§22 drift — dangling citations, incomplete surface lists, and grammar/test-framework claims the compiler contradicts (needs 840, 857 and 873) |
| [ ]  | 889    | LOW      | `mfb man tls connect` says macOS refuses certificates valid ≥ ~398 days even with `allowSelfSigned`; the measured limit is 825 days |
| [ ]  | 890    | LOW      | stdlib spec drift in 12_bits / 13_money / 14_os / 15_astrings / 17_transports — dangling citations, a stale SWAR claim, missing rows and values |
| [ ]  | 894    | LOW      | stdlib spec 20–23 imprecisions (io ESC-in-data claim, openData write-mode reads, openData man vs spec backing, stale compress sizes) |
| [ ]  | 896    | LOW      | memory spec (heap-values, collections) drift — stale sort description, set literal "tight", union tag, map stride, grow-family list, dangling §15.1 |
| [ ]  | 904    | LOW      | memory 06–09 and threading spec drift — stale `.result` helper rule, wrong `MOUSE_STATE_SLOTS` count, closure x28 save and thread-entry wording |
| [ ]  | 910    | LOW      | unicode + package spec drift — stale fold/runtime "differences", `author` called informational though trust-bound, path-component claim overstated, lenient flags undocumented |
| [ ]  | 917    | LOW      | architecture spec topics 02/03/04/05/06/08/12 drift from the implementation (glob base, pin-enforcement site, `.ir` field claims, breadth-limit example, removed seam accessors) (needs 930, 945 and 946, so it is the very last bug) |
| [ ]  | 927    | LOW      | tooling spec 01–06 drift: stale `Sub` block, missing `network` rank, unstated behaviors, small validator gaps |
| [ ]  | 938    | LOW      | architecture spec 13–23 (NIR, ISA catalogs, MIR, math kernels, internal naming, IR JSON, type inference, escape analysis) drift from the code (needs 977 and 978) |
| [ ]  | 944    | LOW      | tooling spec 07–10 drift: app-mode targets/texts stale, help screens disagree with the CLI, small CLI parser gaps |
| [ ]  | 961    | LOW      | `mfb spec linker` drift — wrong macOS vendor rpath, wrong ELF stub registers, stale object-plan relocation kinds, dangling citations |
| [ ]  | 974    | LOW      | `mfb spec app` (macOS runtime, Linux runtime, console-io, term-backend, presentation-mode, canvas) |
| [ ]  | 1102   | LOW      | `mfb man os name` and `mfb spec stdlib os` say `os::name` is `"macos"` or `"linux"`; a Windows build returns `"windows"` |
