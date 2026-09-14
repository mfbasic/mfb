
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

**Every normal ending already passes through `_mfb_shutdown`:** normal return, `EXIT PROGRAM`, untrapped errors, and SIGINT/SIGTERM on Unix console builds.
**Some endings skip it:**
- closing an app-mode window while the program is still running (macOS, GTK, Windows);
- Ctrl-C on Windows;
- SIGPIPE on a stdout write, and crashes.
- On those paths nothing would be printed.

---

## Memory

`src/docs/spec/memory/04_arenas.md` describes the arena correctly; the implementation in
`src/codegen/memory/arena/arena.rs` does not match it. Work top to bottom: sections 1–5
come before any plan is written. The Bucket List at the end is the inventory of what is
wrong.

### 1. Build the measurement harness

**In place (plan-130, `planning/completed/plan-130-*`):** `mfb build --debug` / `mfb test --debug`
make the program print a report to stderr as the last thing `_mfb_shutdown` does
(`mfb spec tooling debug-report`). On every target it gives:

- per-arena counters for the main, worker and graphics arenas: maps, bytes, alloc/free calls,
  live and peak-live bytes, and which path served each allocation;
- `process.peak_rss_bytes`;
- on macOS, the perf timings.

Checked 2026-09-12 against strace: `yamljson to-json samples/config.yaml` reports
`arena.0.maps 12`, equal to strace's 12 executable-IP anonymous maps. Programs that never reach
`_mfb_shutdown` print nothing (app-window close, Windows Ctrl-C, SIGPIPE, crashes).

**Still open — none of these three is delivered by plan-130:**

1. **RSS over time per thread.** The report had only end-of-run values (per-arena
   `peak_live_bytes`, process `peak_rss_bytes`), not a time series.
   **Done (plan-133-C, `planning/completed/plan-133-C-*`):** every arena now reports a
   series sampled on grow, at most 256 samples, with the last sample always the latest grow
   (`mfb spec tooling debug-report`). The keys are `arena.<n>.series.count` and
   `arena.<n>.series.<i>.t_ns`, `.mapped_bytes`, `.live_bytes` and `.peak_rss_bytes`.
   Sampling every grow costs +1.4 % of a browser load on 2223 (8,022 vs 7,914 ms median).
   **First data, 2026-09-13, box 2223:** a `--debug` linux-aarch64 browser loading
   `https://en.wikipedia.org/wiki/Main_Page`. The worker arena (`arena.1`) made 189,654 grows,
   and its series kept 188 samples. Selected rows:

   | sample | t (ms) | mapped MiB | live MiB | peak RSS MiB |
   |---:|---:|---:|---:|---:|
   | 0 | 955.5 | 0.0 | 0.0 | 4.7 |
   | 1 | 1,595.8 | 28.6 | 3.8 | 42.8 |
   | 20 | 3,934.0 | 144.4 | 90.7 | 160.6 |
   | 60 | 5,051.5 | 305.3 | 249.8 | 321.6 |
   | 100 | 6,198.6 | 465.3 | 407.7 | 481.6 |
   | 140 | 7,334.4 | 623.8 | 564.2 | 640.1 |
   | 180 | 8,495.1 | 783.9 | 722.9 | 800.2 |
   | 187 | 8,640.3 | 818.9 | 730.7 | 835.1 |

   From 1.6 s on, live bytes grow steadily, about 140 MiB per second. From 3.9 s they stay
   54–61 MiB below mapped, and the gap widens to 88 MiB at the last sample. The worker never gives memory back during the load (`arena.1.unmaps 0` against 189,654
   maps and 70,951,911 frees): it ends at `live_bytes`
   751,305,104 of `mapped_bytes` 858,677,248.
2. **Measure the entropy-fill cost** on grow and free (fill on vs off), to know its share of
   every number below. Measurement only; the fill stays.
   **Measured 2026-09-13 (plan-133-B Phase 2), box 2223** (native aarch64, 4 KiB pages; load
   0.16–0.45). The whole `benchmark/mfb` suite was built at `b31e6abf8` twice: normally, and
   with a throwaway one-line patch that makes `_mfb_arena_fill_random` return at entry. Each
   build ran `--run 3`, and per-`section.row` medians were compared with plan-130-C's script.
   Checksums were identical in every run.

   | pair (normal → fill-off) | geomean over 485 rows | rows ≥ 0.05 ms |
   |---|---:|---:|
   | normal first, fill-off second | x0.701 | x0.699 over 456 |
   | fill-off first, normal second | x0.710 | — |
   | noise: normal vs normal, across the two positions | x0.999 | — |
   | noise: fill-off vs fill-off, across the two positions | x1.012 | — |

   With the fill off, the suite runs about **29–30% faster by geomean**, the same in both run
   orders. Summed medians fall from 4,103.2 to 3,271.1 ms. The ten arena-heavy rows (first
   pair / swapped pair): bignum.modmul x1.034 / x0.999, bignum.modexp x1.036 / x1.006,
   crypto.churn x0.939 / x0.924, arena.transient x0.941 / x0.898, arena.mixed x0.964 / x0.811,
   arena.growshrink x0.828 / x0.710, scalarbench.listchurn x0.889 / x0.789, mapchurn.churn
   x1.008 / x0.997, datetime.civil x0.888 / x0.956, datetime.iso x1.011 / x1.000. **Single rows
   are noisy:** the same build against itself swings rows by up to x1.42 (`io.binary`), so only
   the geomean is a result. Tables: `/tmp/plan-133-b/ab/2223-*.table`.

   **macOS host** (16 KiB pages), the same two builds compiled natively and run normal then
   fill-off, `--run 3` (18:50:47 → 18:51:10; 1-minute load 4.08 → 4.21, of which three UTM VMs
   use ~2.7 cores all the time; checksums identical): geomean normal → fill-off **x0.702** over
   485 rows, the same as 2223. Named rows: bignum.modmul x0.991, bignum.modexp x0.998,
   crypto.churn x0.941, arena.transient x0.847, arena.mixed x0.899, arena.growshrink x0.701,
   scalarbench.listchurn x0.783, mapchurn.churn x0.988, datetime.civil x0.856, datetime.iso
   x0.980. An earlier pair that ran under a peer's `rustc` (load 8.96–12.93) read x0.615 and was
   discarded (plan-133-B Corrections).

   **Browser `Main_Page` load, box 2223**, normal vs fill-off browser builds, 3 runs each back
   to back (21:50:31 → 21:51:21, load 0.09 → 0.49). Timed with `tools/browser-load-timer` from
   Enter to the footer's file count, so it includes the live network fetch.

   | build | runs (ms) | median |
   |---|---|---:|
   | normal | 7,300 / 7,451 / 7,366 | 7,366 ms |
   | fill-off | 6,192 / 6,077 / 6,097 | 6,097 ms |

   With the fill off, the page loads **~17% faster** (x0.828). The spread within each build
   (≈150 ms and ≈115 ms) is small against the 1,269 ms gap.

   **The fill counters from one `--debug` `Main_Page` load, box 2223** (normal compiler,
   `load_ms=7832 exit=0`):

   | arena | `alloc_bytes` | grow fill (calls / bytes) | free scrub (calls of `free_calls` / bytes) | fill bytes / `alloc_bytes` |
   |---|---:|---|---|---:|
   | main | 413,581,616 | 5,125 / 121,716,576 | 323,463 of 373,655 / 402,924,592 | 126.9% |
   | worker | 5,887,087,504 | 189,654 / 852,608,320 | 23,813,366 of 70,951,911 / 4,000,551,824 | 82.4% |
   | both | 6,300,669,120 | 194,779 / 974,324,896 | 24,136,829 of 71,325,566 / 4,403,476,416 | **85.4%** |

   Over one page load the fill writes bytes equal to 85% of everything the program allocates.
   The main arena reads over 100% because each grow fills a whole fresh block before the program
   uses it. The counters check out exactly: `fill_grow_calls == grow`, `fill_grow_bytes ==
   mapped_bytes − 32 × grow`, and `free_bytes − fill_free_bytes == 16 × free_calls` (5,978,480
   and 1,135,230,576) in both arenas. Two of every three worker frees (47.1 M) are 16 B chunks,
   which have no payload to scrub.
3. **Add an app-sized soak test**: a large parse or long server loop whose peak RSS must stay
   flat across iteration counts. The existing leak tests only cover small code shapes, so
   none of the Bucket List was caught. It fails today and tells you when a fix works.
   **Landed (plan-133-A, 2026-09-13): `tests/runtime/rt_debug_soak.rs`.** Each case builds a
   workload at N and 2N with `--debug` and requires the main arena's `live_bytes` to grow by
   less than 1 MiB. It asserts on `live_bytes`, not RSS, because RSS is 4× `mapped_bytes` on
   Apple Silicon (Bucket List 12a). Status from one full run of that test binary with
   `-- --include-ignored` (release build, 2026-09-13): 2 passed, 5 failed as intended, 144 s.

   | case | N / 2N | status |
   |---|---|---|
   | `a_flat_split_loop_keeps_live_bytes_constant` (control) | 20 / 40 | passes |
   | `a_json_parse_loop_keeps_live_bytes_constant` (plan-134 guard, 1.1 MiB array) | 20 / 40 | passes |
   | `a_dom_parse_loop_keeps_live_bytes_constant` (saved `BASIC` page) | 1 / 2 | `#[ignore]` bug-620/621; fails: +8,318,848 B |
   | `a_resolve_styles_loop_keeps_live_bytes_constant` (generated page, 60 rules) | 4 / 8 | `#[ignore]` bug-620/621; fails: +24,608,640 B |
   | `a_thread_copy_back_loop_keeps_live_bytes_constant` | 400 / 800 | `#[ignore]` bug-622; fails: +2,912,000 B |
   | `an_http_read_loop_keeps_live_bytes_constant` (loopback plain HTTP) | 20 / 40 | `#[ignore]` bug-623; fails: +1,314,240 B |
   | `a_paint_loop_keeps_live_bytes_constant` (small styled page, layout + canvas) | 2000 / 4000 | `#[ignore]` bug-620/621 + bug-625; fails: +1,920,000 B (960 B per paint) |

   Each bug's fix removes its case's `#[ignore]` as its acceptance.

### 2. Find the browser's actual problem

1. **Never freed, or freed but not reused?** Parse the same saved Wikipedia HTML with
   `dom::parse` twice on the main thread (no threads) and compare maps/RSS after the first
   and second parse.
   **Measured 2026-09-13** (plan-133-A Phase 1; main `14c9fc1ca`, after plan-134; macOS host;
   `target/release/mfb build --debug` of a scratch project importing `examples/browser/dom`
   as a source package; `dom::parse` of the saved 603,614-byte `BASIC` page in a loop on the
   main thread; harness `/tmp/plan-133-a/run.sh`). The control is `strings::split(html, "<")`
   in the same loop.

   | stage | N | `live_bytes` | `alloc_calls` | `free_calls` |
   |---|---:|---:|---:|---:|
   | `dom::parse` | 1 | 8,332,368 | 1,706,679 | 1,651,068 |
   | `dom::parse` | 2 | 16,651,216 | 3,413,352 | 3,302,132 |
   | control | 1 | 13,520 | 7 | 5 |
   | control | 2 | 13,520 | 8 | 6 |

   Verdict: **never freed.** Each `dom::parse` leaves 8,318,848 B live — 55,609 of its
   1,706,673 allocations are never freed — while the control reads flat.

   **Every stage, measured 2026-09-13** (plan-133-A Phase 2; same host, build and harness).
   Each stage runs the worker's calls in the worker's order on the saved `BASIC` page and its
   two stylesheets (224,723 B + 6,839 B, fetched once); earlier stages run once before the
   loop. Leak per call = `live_bytes(2N) − live_bytes(N)` over N. "Owned" = the leak left
   after rewriting the named bugs' sites in a scratch copy of the package
   (`/tmp/plan-133-a/patch_dom.py`: 13 sites bound to a `LET` first; the repo is untouched).

   | stage | N / 2N | leak per call (B) | alloc / free per call | owner | with bug-620/621 sites rewritten |
   |---|---|---:|---|---|---:|
   | parse (`dom::parse`) | 1 / 2 | 8,318,848 | 1,706,673 / 1,651,064 | bug-620, bug-621 | 0 |
   | style links (`dom::styleLinks`) | 1 / 2 | 0 | 10 / 10 | flat | — |
   | attach css (`dom::attachCss`) | 1 / 2 | 0 | 43,726 / 43,726 | flat | — |
   | resolve styles (`dom::resolveStyles`) | 1 / 2 | 718,525,504 | 112,385,542 / 71,000,608 | bug-620, bug-621 | 0 |
   | index fields (`dom::indexFields`) | 1 / 2 | 0 | 45,158 / 45,158 | flat | — |
   | copy-back (worker → `thread::waitFor`, main arena) | 1 / 2 | 5,268,592 | 963 / 3 | bug-622 | 5,268,592 (not those sites) |
   | links/fields (`display::links`, `dom::fieldSpecs`) | 1 / 2 | 0 | 43,339 / 43,339 | flat | — |
   | paint (`display::paint`, incl. `dom::updateLayout`) | 1 / 2 | 89,520 | 130,509 / 129,759 | bug-620, bug-621 (87,888); bug-625 (1,632 = 34 × the 48 B one `AttributedString` leaks; the row count is inferred, not counted) | 1,632 |
   | fetch (`http::read`, HTTPS, `BASIC`) | 1 / 2 / 4 | 384 | ≈5 blocks unfreed | bug-623 | — |
   | fetch, plain HTTP over loopback (6,839-byte body) | 20 / 40 | 62,435 | 216 / 211 | bug-623 | — |
   | control (`strings::split(html, "<")`) | 1 / 2 | 0 | 1 / 1 | flat | — |

   The two shapes behind bug-620/621, each reproduced in one screen with no browser code:
   `IF strings::lower(s) <> "zz" THEN RETURN FALSE` leaks the condition's `String` on every
   early return (16 B per call; flat when bound to a `LET` first or when the condition is
   false), and `DO WHILE i < n AND strings::mid(s, i, 1) <> "="` frees its condition temp once
   per loop instead of once per pass (8 blocks per call over a 9-character run). Also filed
   while probing: bug-624 (a package's private `TYPE` collides with a same-named program type).

   **Do the stages account for the worker? Yes, within 1%** (plan-133-A Phase 3). One page load
   runs parse + style links + attach css + resolve styles + index fields once, and the worker
   also keeps the document it returns (its arena is never reclaimed, Bucket List 1).

   | page | sum of stage leaks + returned document (host) | worker arena, one real run of the pipeline (host) | worker on box 2223 (2026-09-13) | host / 2223 |
   |---|---:|---:|---:|---:|
   | `BASIC` | 8,318,848 + 0 + 0 + 718,525,504 + 0 + 5,261,472 = 732,105,824 | 732,119,344 (`copyback` N=1) | 735,804,400 | 99.5% |
   | `Main_Page` | — (stages not run separately) | 749,061,840 (`copyback_mp` N=1) | 751,305,088 | 99.7% |

   With the bug-620/621 sites rewritten, the `Main_Page` worker arena ends holding 4,239,824 B,
   the same size as the main arena's copy (4,246,928 B). So bug-620 and bug-621 own
   744,822,016 B of the worker's 751 MB, and the rest is the returned page. The main arena's
   per-load growth is bug-622, whose result copy is 4.2–5.3 MB per page. The worker figure on
   2223 is 0.3–0.5% above the host's. Not measured why; a guess is that the pages changed
   between the 2223 run and the host's fetch, or that the host run skipped something the live
   fetch does (redirects, the HTTP response).
2. **Alloc vs free call counts** during one browser page load (gdb breakpoint counts on
   box 2223, or the plan-67-F perf rows on macOS). A free count near the alloc count means
   reuse is the problem; a tiny free count means values are never freed.
   **Measured 2026-09-12** (main at `f31de1d37`, `mfb build --debug --target linux-aarch64`,
   box 2223, 120x40 pty, load the page, wait 40 s, `q`). Both pages loaded and rendered and
   the program exited 0. The index-bounds crash plan-130-C hit on `Main_Page` was fixed by
   `6a29185d3` (an in-place `collections::set` widening an empty string).

   | page | arena | maps | mapped | alloc calls | free calls | freed | live at exit | peak live |
   |---|---|---:|---:|---:|---:|---:|---:|---:|
   | Main_Page | main | 15,236 | 174 MB | 290,570 | 194,790 | 67% | 112 MB | 115 MB |
   | Main_Page | worker | 192,273 | 895 MB | 109,560,293 | 67,918,823 | 62% | 841 MB | 842 MB |
   | BASIC | main | 15,052 | 140 MB | 175,730 | 101,950 | 58% | 127 MB | 128 MB |
   | BASIC | worker | 194,378 | 941 MB | 112,504,257 | 71,072,133 | 63% | 856 MB | 858 MB |

   - `process.peak_rss_bytes`: 1,083,760,640 (Main_Page) and 1,095,602,176 (BASIC).
   - The worker requests 4.0 GB (Main_Page) / 4.3 GB (BASIC) for one page, and 61–63% of its
     allocations are quick-bin hits. `flushes 0` and `insert_free_calls 0` in every arena.
   - When the worker returns, the page has already been copied into the main arena
     (112–127 MB live there), yet the worker still holds 841–856 MB live. Nothing reads that
     memory again. It is never reclaimed (Bucket List 1).
   - The Main_Page worker's numbers equal plan-130-C's crash-time run: the same 109,560,293
     allocations, and live bytes within 16 B. The crash was in rendering on the main thread,
     after the worker finished.
   - Reading (not yet proven): freed memory is being reused (quick-bin hit rate, no
     flushes), and the growth is values the worker never frees. Item 1 above is the test
     that confirms or refutes this.

If values are never freed, allocator changes will not move the browser's numbers: judge the
A/B tests in section 3 on the other workloads and treat the browser as a separate problem.

### 3. Spike / A-B test

Every A/B below should report arena map count, peak RSS, and wall time on the same set:
the browser page loads, the audio example, and the allocator benchmarks the `arena.rs`
comments cite (bignum-modexp, datetime, large-list churn). The bins and gates being
questioned were each added for a measured speedup, so no change lands on map count alone.
Build every variant from the same base commit, in its own worktree, as throwaway code, and
measure on macOS and at least one Linux box.

A/B tests:

1. **Drain the large bins in `arena_flush_coalesce`** vs HEAD.
2. **Coalesce in `arena_free`** (route frees through `arena_insert_free`, all sizes vs large
   only) vs the bin push. The bins were the plan-25-A / allocator-01 speedups, so measure
   what coalescing costs back.
3. **Remove the "skip flush when the list is empty" gate** (plan-64 A1) vs HEAD; its comment
   names a datetime workload that regressed without it.
4. **Let a large request split a bigger parked chunk** (best-fit or first-fit over the large
   bins) vs exact-size-only reuse.
5. **Default block size** — 4 KiB vs 64 KiB vs geometric growth. Measure syscall count, fill
   cost and RSS.
6. **A full double-free check** (walk the target bin) vs the head-only check — cost on the
   free path, or whether it belongs in a debug-only build.
7. **The ×1.5 buffer growth policy** behind the audio example's series vs a larger factor or
   a reserve, once 1–4 decide how freed large chunks come back.

Spikes (is it possible and safe, not which is faster):

8. **A dedicated `mmap`/`munmap` for very large requests** (the audio example's 43–129 MB
   buffers). Find the threshold where it beats keeping the chunk in the arena.
9. **Unmap a block once it is entirely free.** How much per-block live accounting it needs,
   and what it costs on the free path.
10. **Reclaim a worker arena when its thread completes.** What still points into the worker
    arena after `thread::waitFor` (the result copy, the control block, which lives in the
    parent's arena), and whether it can be destroyed safely at that point.

### 4. Record the results

Write the measured numbers from sections 1–3 into the Bucket List, and strike any item the
tests disprove.

### 5. Decide whether any result changes the spec

The spec is the target, but an A/B may show a specified behavior costs too much (for example
coalescing on every free undoing the bin speedups). Either accept the cost or change the
spec, and settle it here. Then write the plan.

### Bucket List

Code does not match the spec:

1. **Worker arenas are never reclaimed.** The spec says a worker arena is reclaimed when its
   package instance ends. The only call to `_mfb_arena_destroy` is in `lower_shutdown`
   (`src/codegen/os/process/process_lifecycle.rs`), on the main arena, at exit.
2. **Large bins never drain.** The spec says a colliding large chunk is recovered when the
   bins drain at flush-before-grow. `lower_arena_flush_coalesce` gathers only the free list
   and the 128 quick bins, so a freed chunk > 2048 B is reused only by an exact-size request
   and never coalesces.
3. **`arena_free` does not coalesce.** The spec says a free merges with its address-adjacent
   neighbours (prev, next, both). `lower_arena_free` only pushes onto a quick or large bin
   and never calls `arena_insert_free`.
4. **The double-free guard is weaker than specified.** The spec says a repeated free of the
   same address is a no-op. `lower_arena_free` only detects `ptr == bin head`, i.e. an
   immediate re-free.
5. **Flush-before-grow differs from the spec.** The spec drains every quick bin through the
   coalescing insert and retries the walk. The code skips the flush entirely when the
   address-ordered list is empty, and the retry re-enters at the designated-victim scan.
6. **Large frees go to the wrong place.** The spec's intro puts large chunks on the
   address-ordered list; the code parks them on the hashed large bins.
7. **Entropy fill runs after the bin push**, not after the coalescing insert as the spec says
   (follows from 3).
8. **`arena_destroy` clears more than the spec says**: everything from arena-state offset 104
   to the end (large bins, stdout buffer words, v128 slots, current error, stdin words), not
   just the list heads, quick bins and designated-victim words. Decide which is right.

Look into:

9. **No block is ever unmapped before exit.** An arena that becomes empty keeps every block;
   memory only grows until the process ends.
10. **The browser example's growth.** 60 s of driven use (example.com, Wikipedia `BASIC`,
    Hacker News) mapped 238,167 arena blocks / 1,308,315,648 bytes; the fetch worker for the
    603,614-byte Wikipedia page alone mapped 194,400 blocks / 939 MB. Re-measure after 2–5,
    and check separately whether recursive values (`dom::Node`) are ever freed — the spec's
    Scope-Drop Frees section excludes recursive composites.
11. **The audio example's buffer growth.** Two short tunes grew one buffer in ×1.5 steps to
    requests of 43–129 MB, 611,160,064 bytes mapped in total.
12. **Verify the spec's "O(1) amortized regardless of the size mix" claim** once 2–5 land;
    today a mix of large sizes defeats reuse.
12a. **On Apple Silicon each 4 KiB default arena block costs a 16 KiB page** (plan-133-A,
    2026-09-12). Page size: 16,384 on the macOS host (`sysctl -n hw.pagesize`) vs 4,096 on
    box 2223 (`getconf PAGESIZE`). A `json::parse` loop over a 1,146,842-byte array (N=20,
    before plan-134) mapped 269,946 blocks; `maps × 16,384` = 4,422,795,264 B against a peak
    RSS of 4,452,155,392 B, so RSS ≈ 4× `mapped_bytes` on macOS and ≈ `mapped_bytes` on 2223.
    Any RSS comparison across the two hosts is off by that factor; weigh it with A/B 5 (default
    block size).

No testing needed:

13. **Stale `type_is_flat` references in the spec**: `03_heap-values.md:83`,
    `04_arenas.md:343`, `05_collections.md:245`. plan-114-B split it into
    `type_is_memcpy_copyable` and `type_is_arena_transferable`.
14. **`threading/08_queue-semantics.md:170`** says the runtime bulk-reclaims the worker arena
    at teardown; nothing does (same gap as 1).

