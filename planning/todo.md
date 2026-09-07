
# Cleanup investigation (2026-08-23, read-only survey)

## Q4 — What other areas are a mess? (ranked next cleanup targets)

1. **syntaxcheck vs ir::verify — two overlapping semantic-check passes.** Documented,
   half-finished migration (`src/rules/mod.rs:5-11`, plan-20-Z): "not-yet-relocated" vs
   "relocated" rules. Mirrored filenames (resources/types/link ↔). Rule codes: 58 in
   syntaxcheck vs 118 in ir/verify — actively moving, neither empty. ~19k lines, duplicated
   traversal, goldens pinned to transitional ordering. Finish relocation + delete syntaxcheck
   half = single biggest structural simplification.
2. **Three hand-written app/terminal runtimes, no shared layer** (overlaps Q2/bug-387). Codegen
   targets already unified via `target/linux_common/` (bug-321), but app runtimes were not:
   `macos_aarch64/app/` (8,002 LOC), `win_x86_64/app/` (3,318), `linux_gtk/` (4,813). Terminal
   render + app_io + bootstrap triplicated; plan-13/94/98 keep adding each feature 3×. ~16k LOC.
3. **CLI monoliths + stringly-typed errors.** `cli/build/mod.rs` (3,581), `cli/pkg.rs` (3,296).
   `Result<_, String>` at 484 sites (cli/manifest/resolver/os). Three error mechanisms coexist:
   `rules`+`PendingDiagnostic`, `ast::DocError`, raw `Result<_, String>`. Consolidate tooling side.
4. **os/ per-OS object writers/linkers + dead prototype.** Three stacks (linux 4,233 / windows
   3,500 / macos 3,013) over a thin shared seam; partly inherent (ELF/Mach-O/PE differ). Quick
   win: delete `src/os/windows/link/spike.rs` (426 LOC proof-of-concept PE that writes
   `mfb_spike_proof.txt`, sitting in the linker path).
5. **Hand-rolled JSON serializers.** serde avoided (2 files); ~27 files hand-emit JSON
   (`ir/json.rs` 908, `nir/json.rs` 1,096, `audit/json.rs` 639, …). `src/json.rs` is only a
   shared escaper/parser — no shared value→JSON writer. Mechanical consolidation.
6. **(Diagnose first — likely intentional) src/ir vs target/shared/nir.** Two IR layers; NIR used
   by all backends + ~40 codegen files. Probably deliberate layering (IR → NIR → arch encoders);
   confirm it earns its keep before growing either.

Cross-cutting symptoms (evidence, not targets): `#[allow(clippy::too_many_arguments)]` ×117
(missing context structs, in target/os/syntaxcheck); `#[allow(dead_code)]` ×34 (mostly still in
codegen). TODO/FIXME grep understates debt — this team encodes it in `planning/`/`bug-NN` docs.

Suggested order: #1 and #2 lead (largest, actively worsening). #3 and #5 contained mechanical
wins. #4 spike.rs is a quick delete. #6 diagnose-first.

---

# websockets

Still not quite — but the gap narrowed. You can build a standalone RFC 6455 WebSocket implementation over `tcp`/`tls`, and SHA-1 now ships, so the handshake no longer has to be hand-rolled. What remains missing is the same thing as before: the `http` package cannot perform or hand off an upgraded connection.

Updated 2026-08-30 for the `net` split (plan-110): the old monolithic `net` transport surface is gone. `net` now owns only DNS (`net::lookup`), ICMP echo (`net::ping`), URL parsing (`net::toUrl`, `net::percentDecode`, `net::parseQuery`), and the shared `net::Address` record. Byte streams moved to `tcp`, datagrams to `udp`, encrypted streams stayed in `tls`. A WebSocket package would `IMPORT tcp`, `IMPORT tls`, and `IMPORT net` (imports are not transitive, and naming an `Address` requires importing `net` itself).

What you already have:

- Raw TCP (`tcp::read`/`tcp::write`) and TLS (`tls::read`/`tls::write`) byte streams, both with full-buffer writes and a `String` overload on write.
- Partial-read semantics suitable for framed protocols: `tcp::read` is a short read, `tls::read` returns as soon as any plaintext is decrypted.
- Per-socket deadlines on both transports — `tcp::setReadTimeout`/`setWriteTimeout` and `tls::setReadTimeout`/`setWriteTimeout` (the TLS deadlines landed in plan-110-D).
- A readiness **multiplex** on both transports: `tcp::poll(List OF RES tcp::Socket, timeoutMs)` and `tls::poll(List OF RES tls::Socket, timeoutMs)` each return the first ready socket. That is enough to write a single-threaded, many-connection WebSocket server without a thread per client. `tls::poll` also accounts for bytes already buffered inside the TLS layer, which a raw transport poll would miss.
- **SHA-1**: `crypto::hash(Hash.SHA1, ...)` exists and is the standard FIPS 180-4 digest, computed by the portable software core. See the caveat below.
- Client and server TLS with certificate verification (`tls::connect`, `tls::listen`, `tls::accept`).
- Secure randomness for client masking keys and `Sec-WebSocket-Key` (`crypto::randomBytes`).
- Base64 including the URL alphabet (`encoding::base64Encode`/`base64UrlEncode`), UTF-8 validation, bitwise operations, and 64-bit integers.
- Enough collection support to maintain a receive buffer and parse fragmented frames — including `List OF RES tcp::Socket` / `List OF RES tls::Socket` for a connection table. (Resources may be collection *elements* when spelled `RES`; they may never be record *fields*, so a per-connection state record cannot embed its own socket — keep the socket and its state in parallel structures, or in a `RES ... STATE` binding.)

What is missing or awkward:

- **HTTP connection upgrade/hijacking.** Unchanged and still the blocker. `http::handleRequest` accepts the connection, owns the accepted socket, always emits `Connection: close`, drops any handler-set `Connection`/`Content-Length`, and closes the socket by lexical drop on return. A handler only ever sees a parsed `Request` and returns a `Response`; there is no way to get the live socket or the unread buffered bytes back out.
- **HTTP client upgrade support.** Also unchanged. `http::startRead` returns a `RES http::Stream STATE PendingState` — a resource union over `tcp::Socket` and `tls::Socket` — but it always sends `Connection: close` and drives toward a complete HTTP response. There is no "101 received; take this stream" operation, and no way to unwrap the union back into the underlying socket.
- **`ws://` and `wss://` URL parsing.** `net::toUrl` still lowercases the scheme and accepts only `http`/`https`; anything else raises `ErrUnsupported`. A package can rewrite the scheme before parsing (and must then re-apply the 80/443 port default itself, since `toUrl` derives the default from the scheme it saw).
- **The SHA-1 warning.** `crypto::hash(Hash.SHA1, ...)` works, but every source occurrence of `Hash.SHA1` emits the non-fatal `CRYPTO_SHA1_INSECURE` warning (2-203-0136). A WebSocket package would carry one unavoidable warning at its single handshake call site. Nothing suppresses it today; a narrow protocol-compatibility exemption (or a documented `crypto` entry point for handshake transforms) would keep a WebSocket package's build clean.
- **Every socket and listener is thread-sendable (bug-464, resolved 2026-08-31).** Resources are not sendable by default — it is a per-resource opt-in (`THREAD_SENDABLE` on a user declaration, spec §17; the registry `sendable` bit for a builtin), enforced by `require_thread_sendable` on the thread's resource plane (`src/ir/verify/resources.rs:544`). `tls::Socket`, `tls::Listener` and `tcp::Listener` used to be `sendable: false`, so `Thread OF RES tls::Socket TO …` was rejected outright with `TYPE_THREAD_NOT_SENDABLE` (2-203-0063) at the *type declaration*. All five now transfer. The blocker was never the flag: `copy_resource_to_current_arena` carried only the canonical header and **zeroed** the type-specific record tail, so a TLS handle arrived at its receiver with a null session. The registry now declares each resource's live tail slots (`RegistryResource::live_slots`) and the transfer copy carries them, per backend — `SSL_CTX*`/`SSL*` on OpenSSL, an arena SSPI block (deep-copied) on Schannel, the connection ctx / dispatch queue on Network.framework.

End of stream: **both transports raise `ErrConnectionClosed`** — they agree, and a framing loop needs a `TRAP`, not an empty-list check. `mfb man tcp read` used to claim `tcp::read` returned an empty list at EOF and shipped a drain example looping on `len(chunk) = 0`; that was the documentation being wrong about its own emitter, corrected in bug-465, and both contracts are now pinned side by side (`tests/rt-behavior/{tcp/tcp-read-eof-raises-rt,tls/tls-read-eof-raises-rt}`). Do not write a normalizing transport shim for this; there is nothing to normalize.

Writing, though, is **not** symmetric with reading and is the thing to design around: a write to a peer that has gone away is not reported — the second one kills the process with `SIGPIPE` (bug-467). A WebSocket server cannot rely on a `TRAP` around its send path to survive a client that disconnects mid-frame; detect the disconnect on the read side, where the raise is prompt and correct.

Important implementation requirements (unchanged):

- Preserve bytes following `\r\n\r\n`; the first WebSocket frame may arrive in the same read as the handshake.
- Accumulate partial frame headers and payloads across reads.
- Handle 7-bit, 16-bit, and 64-bit payload lengths with overflow and allocation limits.
- Require client-to-server masking and reject masked server frames.
- Generate a fresh unpredictable 32-bit mask per client frame.
- Validate control-frame constraints: FIN set and payload ≤125 bytes.
- Implement continuation frames and fragmented messages.
- Validate text as UTF-8 across the complete fragmented message, not independently per frame.
- Echo ping payloads in pong frames.
- Implement the close handshake and validate close codes/reason text.
- Reject unsupported RSV bits and extensions.
- Apply explicit message/frame size limits.
- Do not negotiate `permessage-deflate` unless compression and its security limits are deliberately implemented.

So the practical verdict is:

- A self-contained, blocking WebSocket client: **yes**, and cheaper than before — the HTTP handshake is still hand-written over `tcp`/`tls`, but SHA-1 is no longer part of the work.
- A standalone WebSocket server: **yes**, same work.
- A *many-connection* server without a thread per client: **yes**, using the `tcp::poll` / `tls::poll` list multiplex over a connection table. This is new; the previous entry predates the multiplex.
- A thread-per-connection plaintext (`ws://`) server: **yes** — `tcp::Socket` is thread-sendable, so an accepted socket can be `thread::transfer`red to a worker. Since bug-464 the accept loop need not stay on one thread either: `tcp::Listener` transfers too, so a program can bind on one thread and accept on another.
- A thread-per-connection secure (`wss://`) server: **yes** since bug-464 — `tls::Socket` and `tls::Listener` are both sendable, so an accepted TLS socket transfers to a worker and the accept loop can itself live on a transferred listener. The single-threaded `tls::poll` multiplex remains the cheaper choice for many idle connections; the two are now a real design choice rather than one option.
- WebSockets integrated into the existing HTTP router: **no**. Unchanged.

The single highest-value platform addition is now an **HTTP upgrade API** that returns the live transport plus buffered surplus bytes — on the server side out of `handleRequest`, and on the client side out of the `http::Stream` union. (Making `tls::Socket` thread-sendable was second on this list; bug-464 did it, so a threaded `wss://` server is no longer blocked.) Native `ws`/`wss` `toUrl` support and a warning-free SHA-1 spelling for protocol handshakes are both smaller conveniences.

---

# test fix

run acceptance under each system in the matrix.

## riscv fail


test cli::build::tests::builtin_codegen_corpora_lower_in_process ... ok

failures:

---- cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched stdout ----

thread 'cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched' (7543) panicked at src/cli/build/mod.rs:1691:33:
mfb test should pass: ()

---- cli::build::tests::build_project_coverage_test_writes_a_report stdout ----
Wrote coverage report to /tmp/.tmprfwNj0/coverage.html

thread 'cli::build::tests::build_project_coverage_test_writes_a_report' (7485) panicked at src/cli/build/mod.rs:3029:33:
coverage test passes: ()


failures:
    cli::build::tests::build_project_coverage_test_writes_a_report
    cli::build::tests::mfb_test_host_run_leaves_project_build_dir_untouched

test result: FAILED. 3803 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7728.90s

error: test failed, to rerun pass `-p mfb --bin mfb`

---

## Benchmark-equivalence concerns

Some results strongly suggest that not every implementation performs equivalent observable work:

C list operations such as transform, window, zip, and replace
report approximately 0 ms.
C chunks, flatten, drop, and take are also nearly free.
MFBasic copy is about 0.005 ms while C copy is 2.813–18.512 ms.
Python crypto is dramatically faster because modules such as hashlib
call optimized native implementations.
Python regex similarly uses a mature native regex engine.
Even at -O0, a C compiler can eliminate unused results or reduce work when it can prove that it has no observable effect. Every benchmark should therefore:

Consume the complete result.
Compute and validate a checksum.
Keep workload sizes and data identical.
Exclude setup consistently.
Confirm equivalent allocation and copying semantics.
Prevent constant folding and dead-result elimination.
Verify output after timing, rather than printing inside the timed region.
The near-zero C collection timings deserve particular scrutiny before calculating an overall geometric mean.

---

## Benchmark Main performance cliffs

The biggest issue appears to be dynamic collection reduction:

Dynamic reduce: roughly 825–892 ms
Dynamic reduceRight: roughly 872–935 ms
Python equivalents: roughly 49–52 ms
C equivalents: roughly 1.1 ms
That is likely more than normal dynamic-dispatch overhead. It suggests something structural, such as:

copying or materializing the accumulator on every iteration;
repeated boxing/unboxing;
generic callback dispatch through a costly runtime path;
reference-count or state synchronization overhead;
repeated dynamic type checks;
accidental quadratic behavior.
State-backed mutations are another major hotspot:

list (State-Dynamic).set: 1236 ms median
removeAt: 245 ms
prepend: 103 ms
map (State-Dynamic).removeKey: 66 ms
set (State-Dynamic).remove: 70 ms
The progression from plain → record → state strongly suggests mutation bookkeeping, copy-on-write, observer propagation, or state snapshots are dominating.

Base64 is also anomalous:

MFBasic: 154.844 ms
C: 6.910 ms
Python: 4.874 ms
This likely indicates MFBasic is implementing Base64 using high-level per-byte operations while Python and C call optimized native routines.

---

# FS

- add `fs::pathSeparator() as String`
- add `fs::pathListSeparator() as String`
- add `fs::lineEnding() AS String`

---

# MORE FUCKING STRINGS

Straight answer first: plan-111's charter, in its own words (`planning/completed/plan-111-A…md:8`), was *"delete every **type** string after the AST."* It did that and only that. Nothing in it addressed names, operators, keywords, literals, symbols, or machine operands. The gate it left behind (`tests/no_type_strings.rs`) encodes that same narrow scope, which is why it reads green while 899 `== "` / `!= "` sites remain in `src/` (`grep -rn --include="*.rs" '[!=]= *"' src/ | wc -l`) — `ir` 204, `codegen` 212, `arch` 86, `target` 82, `monomorph` 32, `optimizer` 26, `hir` 5, `resolver` 2.

Here is the enumerated census of what is still a string after the AST.

## Still strings

- **Local/global variable identity** — `IrValue::Local(String)`, `Global(String)` (`src/ir/value.rs:25,26`), `IrOp::Bind/Assign/AssignGlobal { name: String }` (`src/ir/op.rs:8,21,26`), `NirValue::Local(String)` (`src/target/shared/nir/mod.rs:264`). Bindings are matched by name string from HIR to register allocation — no index, no `Symbol`.

- **Call targets / function identity** — `Call { callee: String }` (`src/hir/mod.rs:432`), `IrValue::Call/CallResult { target: String }` (`src/ir/value.rs:57,65`), `NirValue::Call/CallResult/RuntimeCall { target: String }`. Dispatch is string compare plus `split_once('.')` on `"pkg.member"` (27 sites; `src/ir/shape.rs:1769,1834,1854`, `src/ir/lower.rs:2462`) against literals `"thread.start"`, `"net.poll"`, `"process.spawnEnv"`, `"tls.listen"`, `"crypto.sign"`.

- **Member / field names** — `MemberAccess { member: String }` (`src/hir/mod.rs:466`), `HirRecordUpdate.field: String` (`:291`), `IrValue::MemberAccess { member: String }` (`src/ir/value.rs:118`), decided by `member.as_str()` match (`src/ir/lower.rs:2164,2172,2179`).

- **Declaration keywords, as text, inside the IR** — `IrType { kind: String, visibility: String }` (`src/ir/types.rs:6,7`), `IrFunction.kind: String` (`:209`), `IrField.visibility: Option<String>`. `"record"`/`"union"`/`"enum"`, `"public"`/`"private"`, `"function"`/`"sub"` are compared as spellings: 53 sites (`grep -rn 'visibility *== *"\|kind *== *"' src/ | grep -v _tests.rs`).

- **Literal payloads** — `HirExpression::Number(String)` (`src/hir/mod.rs:415`), `IrValue::Const { value: String }` (`src/ir/value.rs:23`), `NirValue::Const { value: String }`. Numeric literals stay source text and are re-parsed downstream: 24 `parse::<f64>/<i64>/<u64>` in `src/ir`+`src/codegen`.

- **Machine operands and registers** — `Operand::Raw(Box<str>)` plus `impl From<&str> for Operand` (`src/codegen/engine/operand/operand.rs:163,362`), so any `"x0".into()` mints a string operand. The file's own doc: *"in the pre-allocation stream every register operand is `Raw`"*. Regalloc then re-parses it: `starts_with('%')` (`src/codegen/engine/regalloc/analysis.rs:298,352`), `value.starts_with('%') || value == "sp"` (`src/codegen/engine/regalloc/mod.rs:107`). 465 non-test `.render()`/`.rendered()` calls.

- **Type spellings inside the codegen code plan** — `CodeParam.type_: String`, `CodeFunction.returns: String`, `CodeStackSlot.type_: String` (`src/codegen/engine/types/types.rs:37,23,1332`), filled by rendering a `ParameterType` back out — `param.type_.clone().name().into_owned()` (`src/codegen/engine/function/function_lowering.rs:829`, `src/codegen/link/thunk/link_thunk.rs:1755`) — and by bare literals `"Nothing".to_string()`, `"Integer".to_string()` (`src/codegen/engine/builder/mod.rs:2185,2267,2397`). These contradict plan-111's headline ("`ParameterType` is the compiler's only type currency … to the emitted byte") and pass the gate because the gate has a class for `&str` *parameters* and none for a `String` *struct field*. In fairness: I grepped for readers and found none that decide on them — they terminate in `json_string(&self.returns)` (`src/codegen/engine/builder/code_impl.rs:252`, `src/codegen/engine/mir/mir.rs:901`). Dead carriers, not live decisions — but they are type strings after the AST.

- **Generic instantiation identity** — monomorph names instantiations with a mangled string (`emit$Integer`, `show$List$OF$String`) built by `sanitize_type_name` (`src/monomorph/helpers.rs:546`), documented lossy at `src/monomorph/lower.rs:2352`: "`(`/`)` and `{`/`}` both sanitize to `$`."

- **Link symbol / library identity** — `library: String`, `symbol: String`, `alias: String` (`src/hir/mod.rs:102,104,123`; `src/codegen/engine/types/types.rs:120,121,125`), compared against `"libc.so.6"`, `"libpthread.so.0"`, `"GetStdHandle"`, `"getentropy"`.

- **Target/platform identity** — `NativeCodePlan { target: String, arch: String }` (`src/codegen/engine/types/types.rs:7,11`), compared `== "macos-aarch64"`.

- **Data-object and relocation tags** — `CodeDataObject { kind: String, layout: String, value: String }` (`:126,127,130`); operand-class tags `== "label"`, `== "external"`, `== "symbol"`, `== "data"`, `== "str_u64"` in `src/arch`; and `emit_symbol_ref(kind: &str)` matching `"adrp"`/`"add_pageoff"` (`src/arch/aarch64/encode/emitter.rs:1190-1195`).

- **Ad-hoc positional tags** — e.g. `side(instructions, models, overlay, i, "lhs")` / `"rhs"` (`src/optimizer/opt2/plans/ranges.rs:229-230`).

## Two things that are actually clean

- **Lexer tokens do not survive the AST.** `grep -rn "TokenKind\|lexer::" --include="*.rs" src/ | grep -v "^src/lexer.rs" | grep -v "^src/ast/"` → one file, `src/fmt.rs`, the source formatter working on raw text.
- **Instruction opcodes are typed.** `CodeOp` is a `Copy` enum; the `== "adrp"`/`== "fadd_d"`/`== "lr"` hits in `src/arch` are almost all `#[cfg(test)]` inspection via `op.mnemonic()`, not selection logic. Same for `RuntimeHelper`, `RegClass`, `AbiConvention`/`AbiRole`, `LoopKind`.

So the typed machinery exists and works — it was applied to exactly one vocabulary. No changes made.

---

# Ecosystem gaps: what to build next, and in which shape (2026-09-06, read-only survey)

Survey question: given the most-downloaded Rust crates as a proxy for what programs
actually need, what is missing from MFB's ecosystem — and for each gap, should it be a
builtin, a binding package, or a pure MFB package?

Finding on the premise: the crates.io top-20 by all-time downloads is the *wrong* map.
MFB already has a builtin equivalent for 15 of those 20 (`hashbrown`→`Map`/`Set`,
`getrandom`/`rand`→`crypto::randomBytes`+`math::rand`, `bitflags`→`bits`,
`base64`→`encoding`, `indexmap`→`Set OF T`, `itertools`→`collections`,
`thiserror`→`errorcode`, `regex-syntax`→`regex`, `memchr`→`strings::find`,
`serde`→`json`, …), and the other five (`syn`, `quote`, `proc-macro2`, `cfg-if`,
`libc`) are Rust-specific plumbing MFB structurally does not need. The signal is one
tier down: the crates people reach for *on top of* those primitives.

Source: `curl -s "https://crates.io/api/v1/crates?page=1&per_page=60&sort=downloads"`.

## The four shapes that already exist in this repo

There are four, not three; the fourth ("builtin + dlopen") changes two of the verdicts.

| Shape | Where | Mechanism | Examples |
|---|---|---|---|
| Builtin | `src/codegen/builtins/<pkg>/` | Rust descriptors, emitted as native code. Always present, nothing to resolve. | `json`, `csv`, `regex`, `strings` |
| Builtin + dlopen | same | Builtin that reaches a *system* lib at runtime via dlopen | `tls` → libssl (`emit_dlopen_libssl`, `src/target/shared/code/tls/mod.rs:199`) |
| Binding package | `packages/<pkg>/` | `LINK` block + `"libraries"` in `project.json`; `type: system` or `type: vendor` | `sqlite3` (system libsqlite3), `libsnd` (7 vendored builds) |
| Pure MFB package | `packages/<pkg>/` | Only `.mfb` under `src/` | `yaml` 2,795 ln, `jwt` 3,286 ln, `mustache` 1,839 ln |

Spec defines the binding shape at `src/docs/spec/language/17_native-libraries.md:3-9`:
"A source package that declares `LINK` is a binding package."

Shape-selection criteria the repo's own choices reveal:

* **Builtin** when a builtin already consumes it (`http` needs gzip, `net` needs TLS), when
  it needs syscalls/arch code MFB source cannot express, or when it is a hot byte loop at
  scale (why `json`/`csv`/`regex` are Rust and not packages).
* **Binding package** when a mature ubiquitous C library exists, the surface is large, and
  reimplementing is a correctness or security liability — and only user code needs it.
* **Pure MFB package** when it is composition over existing builtins with no syscalls and
  no perf cliff.

## The gaps, ranked, with a shape verdict

| # | Thing | Shape | Why |
|---|---|---|---|
| 1 | CLI arg parsing (`clap`, #34) | Pure MFB package | Whole surface today is `os::args`; `grep -rl "os::args" examples/ packages/` = 7 files each hand-rolling a loop, incl. four of our own oracle probes. argv is tiny — no perf argument. |
| 2 | Logging (`log`, #29) | Pure MFB package (condition below) | Composition over `io`, `os::getEnv`, `datetime`. Today the surface is `io::print`/`io::printError`. |
| 3 | TOML | Pure MFB package | Exact `yaml` precedent — parse to `json::Json`, let `json::get`/`stringify` do the rest. Config-scale text, no perf cliff. Oracle is free (Python `tomllib`). |
| 4 | gzip/deflate | Builtin + dlopen — already decided | plan-93-A specifies `compress::` mirroring the libssl dlopen against system zlib. Reasoning holds: 64 MiB bodies, dynamic Huffman + 32 KiB LZ77 window, and `http::` is a consumer. Unblocks 93-B/C. |
| 5 | zip/tar archives | Pure MFB package | Container formats are header parsing and offsets; delegate DEFLATE to `compress::`. Matches http's own contract — "all protocol work is string manipulation; only the transport branches reach native code" (`src/builtins/http.rs:5`, quoted in plan-93-A). Unblocks `.docx`/`.xlsx`/`.odt`/`.epub`/`.jar`. |
| 6 | XML | Pure MFB package | See note below. `grep -ril xml src/codegen/builtins/ packages/` hits only HTML *escaping* and MIME tables — zero parsing coverage. |
| 7 | WebSocket | Pure MFB package (client + standalone server); the `http`-integrated server needs a builtin seam | `wss://` establishes TLS at connect time and then speaks HTTP over it, so it never needs `tls::wrap` — this is NOT blocked by the macOS constraint below. A package owning the connection from `tcp::connect`/`tls::connect` onward ships today. Only sharing a port with `http::server` needs the hijack seam. See `# websockets` above, which reaches the same three verdicts. |
| 8 | PostgreSQL client | **Binding package (libpq)** — corrected 2026-09-06 | Postgres negotiates TLS in-band (SSLRequest, then handshake on the same socket), which is exactly the `tls::wrap` that cannot exist. See note below. |

Deliberately **not** recommended: a web framework (`http::route` already does `:name`,
`:name?` and `*` path params) and image codecs (canvas already decodes PNG via
`helper_png.rs`).

Suggested order: **#1 first** — smallest thing on the list, no compiler change, seven files
in-tree waiting for it, and it shortens every later package's oracle probe and example.
Then **#4**, because it is already scoped, already half-implemented, and three written
plans queue behind it.

## Hard constraint on every network package: TLS must be established at connect time

`tls::wrap(tcp::Socket)` does not exist and cannot (`.ai/net-tls.md:204` — Network.framework
fixes TLS in `nw_parameters` at creation; the two alternatives that could adopt a live fd
are LibreSSL, unsupported for new development, and Secure Transport, deprecated and capped
at TLS 1.2). This is permanent architecture under Apple's rules, not a gap awaiting work.

The consequence for package selection is a clean rule:

* **Pure-MFB-viable** — TLS is established at connect time, before any protocol bytes:
  HTTPS, `wss://`, Redis (TLS-on-connect), MongoDB, gRPC.
* **NOT pure-MFB-viable, binding-package candidates only** — TLS is negotiated in band on
  an already-open plaintext socket: PostgreSQL (SSLRequest), MySQL (TLS after the initial
  handshake packet), SMTP/IMAP/POP3 (`STARTTLS`), FTPS (`AUTH TLS`), LDAP StartTLS.

Check which side a protocol falls on **before** scoping it as a package. The failure mode
is not a compile error — it is discovering at the end that the driver works, and cannot
encrypt on one of the five targets.

## The three calls worth defending

**Postgres as a libpq binding — corrected 2026-09-06.** This entry originally argued
the opposite (pure MFB, shaped like `jwt`, because Postgres is a wire protocol and
essentially no ecosystem binds libpq). That reasoning ignored TLS. Postgres negotiates
encryption **in band**: connect in plaintext, send the 8-byte SSLRequest (code 80877103),
read `S`, then handshake TLS *on that same socket*. That final step is precisely
`tls::wrap(tcp::Socket)`, which `.ai/net-tls.md:204` establishes can never exist — the
macOS constraint is permanent, not pending. A pure-MFB driver would therefore be
plaintext-only, which is unusable against any managed provider. libpq brings its own TLS
and does its own negotiation, so the binding is the correct permanent design; the
deploy-dependency cost (libpq absent by default on macOS and Windows, so `type: system`
plus a documented prerequisite, or vendored builds like `libsnd`) is real but strictly
smaller than shipping a driver that cannot encrypt. One escape hatch exists and is not
enough on its own: PG 17's `sslnegotiation=direct` (ALPN `postgresql`) does TLS
immediately on connect and would work with `tls::connect` as-is — but it needs server >= 17
and is not the default, so a pure-MFB driver could only encrypt against the newest
Postgres.

**XML as pure MFB, not a libxml2 binding.** `yaml` at 2,795 lines is the precedent and XML
is comparable. libxml2 would mean a large wrapper surface, an inherited CVE history, and —
because it is not a system library on Windows — the full `libsnd` vendoring treatment.
Scope to **XML only**: HTML5 tag-soup parsing is a different animal (the WHATWG
error-recovery algorithm is a spec unto itself) and deserves its own decision later.

**Logging, with one condition.** Package is right *unless* the `http` builtin server should
emit through the same facade. Builtins cannot depend on packages, so that requirement alone
forces it into `io::` as a builtin. Decide that first — cheap now, expensive after adoption.

## Gaps 9-16 — the second tier (2026-09-06)

Same method, one tier deeper: crates.io ranks 61-400 as the lens, filtered to what is
genuinely absent, with a shape verdict. Absence for each row confirmed with
`grep -rilE <concept> src/codegen/builtins/ packages/`; glob's only hits are
`http::route`'s URL `*` matching (not paths) and bigint/semver's only hits are
`node_modules` noise inside oracle directories.

| # | Thing | Rank | Shape | Why |
| --- | --- | --- | --- | --- |
| 9 | Binary struct reader/writer | `byteorder` #101 | Pure MFB package | Read/write fixed-width ints at an offset in a `List OF Byte`, LE/BE. `bits::bswap16/32/64` is the primitive; nothing composes it into a cursor. Foundation for #5 (zip/tar), #16, image formats, every wire protocol. `packages/jwt/src/ecdsa.mfb` already hand-rolls byte assembly. |
| 10 | ASN.1 DER + PEM key encoding | `pem` #376, `der` #274, `pkcs8` #303 | **Builtin — `crypto::*`** (decided) | Belongs with the keys, not in a sibling package. Closes bug-516 (`0x04‖X‖Y‖d` is neither raw `d` nor PKCS#8, so keys do not move between MFB and OpenSSL). |
| 11 | Path glob + recursive walk | `glob` #173, `walkdir` #168 | Pure MFB package | Both absent; `fs::listDirectory` is single-level. Table stakes for any CLI or build tool. Pairs with #1. |
| 12 | Platform standard directories | `dirs` #359, `home` #290 | **Builtin — `os::*`** (decided) | Naming and the `fs::tempDirectory` move are settled below. |
| 13 | semver | #59 | Pure MFB package | Absent, and pointed: this repo HAS a package registry (`repository/`, `.mfp`, `project.json`'s `version`), so range resolution exists in Rust but is unavailable to MFB programs. Any tooling written in MFB needs it. Tiny. |
| 14 | Word wrap + width-aware layout | `textwrap` #287 | Pure MFB package | Absent. The missing half of bug-528 (pad counts scalars, `displayWidth` counts columns). `strings::displayWidth` + `strings::graphemes` are the primitives; nothing composes them into wrapping or table layout. What makes `term` usable for real TUIs. |
| 15 | Arbitrary-precision integers | `num-bigint` #182 | **Builtin package — `big`** (decided) | Function-based math, no operators; a value record. |
| 16 | Protocol Buffers / binary serde | `prost` #183, `bincode` #367 | Pure MFB package | Sits directly on #9. Service-to-service wire format. |

Seven of these eight are composition over primitives that already exist, which is why most
are small — calibrated against `libsnd` (863 lines) and `sqlite3` (1,078), items 11, 12, 13
and 14 are each plausibly under 500. The connect-time-TLS constraint above does not bite
anywhere in this batch; none of these are network protocols.

### #10 — the ASN.1 split still has to be decided

Two different things live under "DER" and only one is unambiguously crypto's:

* **Key import/export** (PKCS#8, SPKI, SEC1, PEM armor) — clearly `crypto::`, and it is what
  closes bug-516.
* **Generic ASN.1 parsing** — `packages/jwt/src/ecdsa.mfb` hand-writes a DER parser to
  unpack the signature `crypto::sign` already returns, with a comment about "a short-form
  length byte here, a long-form length there -- and any of them verifies". If `crypto::`
  gains only key import/export, that parser stays hand-rolled.

Decide whether the goal includes deleting jwt's parser (via a signature-to-raw conversion,
or an exposed DER reader). It changes the scope.

### #12 — naming, and what `dirs` actually provides

Settled convention: `os::` owns standard locations and spells them `*Path`, matching the
two that already exist (`os::executablePath`, `os::resourcePath`).

* `fs::tempDirectory` moves to **`os::tempPath`**.
* Add **`os::homePath`** (or `os::userPath`).

The move is a breaking rename with a measured blast radius:
`grep -rn "fs::tempDirectory"` excluding `.git`/`target`/`node_modules` → **161 call sites**,
of which **116 are generated** `.mfb` under `benchmark/mfb/src/` and **3 are the Python
generators** that emit them (`benchmark/mfb/gen_list.py`, `gen_map.py`, `gen_set.py`). Edit
the three generators and regenerate; do not hand-edit the 116.

What the `dirs` crate provides, since the name is opaque: the per-platform answer to "where
does this OS expect me to put things". The value is entirely in the divergence — the three
platforms disagree completely, and guessing means littering `~` on macOS or writing to the
wrong hive on Windows.

| purpose | Linux | macOS | Windows |
| --- | --- | --- | --- |
| home | `$HOME` | `$HOME` | `%USERPROFILE%` |
| config | `$XDG_CONFIG_HOME` or `~/.config` | `~/Library/Application Support` | `%APPDATA%` (Roaming) |
| cache | `$XDG_CACHE_HOME` or `~/.cache` | `~/Library/Caches` | `%LOCALAPPDATA%` |
| data | `$XDG_DATA_HOME` or `~/.local/share` | `~/Library/Application Support` | `%APPDATA%` |
| state / logs | `$XDG_STATE_HOME` or `~/.local/state` | `~/Library/Logs` | `%LOCALAPPDATA%` |
| runtime | `$XDG_RUNTIME_DIR` | (none) | (none) |

It also exposes the user media folders (desktop, documents, downloads, pictures, videos,
music, fonts, templates, public). Note that `homePath` and `tempPath` are the two LEAST
interesting members — they barely differ across platforms. **config / cache / data are the
ones worth having**, and are the reason the crate exists.

### Considered and left out of 9-16

* **gRPC** (`tonic` #310) — passes the connect-time-TLS rule, but needs HTTP/2, which
  `http` does not speak. Builtin-scale project, not a package.
* **`strsim`** (#41) — high rank, but ~80 lines of Levenshtein. Fold into the CLI package
  (#1) as the "did you mean" backing rather than shipping standalone.
* **`ipnet`** (#187), **`httpdate`** (#181) — small enough to belong inside `net` and `http`.
* **`petgraph`** (#248), **`lru`** (#340), **`memmap2`** (#342) — real but niche; memmap
  needs a builtin for the syscall.
* **`criterion`** (#400) — a `perf` builtin exists, but its surface is in `perf.rs` rather
  than `func_*.rs` and could not be extracted the usual way. Check what it covers before
  scoping anything here.

## Design question to fold into plan-93-A before starting it

`src/codegen/builtins/canvas/helper_inflate.rs` is 460 lines of inflate, and
`grep -rn inflate src/codegen/builtins/ --include=mod.rs` shows it registered only by
`canvas/mod.rs:1052` — canvas-private, for PNG. Landing `compress::` as written yields
**two inflate implementations**, one hand-written in Rust and one via dlopen'd zlib, with
different bug surfaces. Resolve it in the plan: either point canvas at `compress::`, or
state explicitly why PNG keeps its own (canvas may need it without a zlib dependency on
some target). Not a defect at HEAD — a design question for the plan.

---
