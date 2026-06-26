# plan-03 — Built-in HTTP Package (client + server, with `net` URL support)

Last updated: 2026-06-25

This document is the **normative definition and implementation plan** for the
built-in `http` package — both a **blocking** HTTP/1.1 **client** (Parts A–E) and a
single-threaded, routing **server** (Part F). Both are built as a
**source package** in the established `json`/`csv`/`regex` idiom — a thin Rust
shim (`src/builtins/http.rs`) plus an MFBASIC implementation file
(`src/builtins/http_package.mfb`) injected at compile time — and it **layers on
the existing native `net` and `tls` packages** for transport. No new Rust
intrinsics or syscalls are introduced; every byte on the wire goes through
`net::*` (plaintext) or `tls::*` (TLS), which already exist and work.

Because a URL is a **general networking concept**, not an HTTP-specific one, this
plan puts the URL type and its helpers in **`net::`**, not `http::`:

- `net::Url` — the parsed-URL record (§A.1).
- `net::toUrl(href AS String) AS net::Url` — parse an href into its components (§A.2).
- `toString(url AS net::Url) AS String` — render a `Url` back to an href (§A.3).
- `net::toAddress(url AS net::Url) AS net::Address` — project a `Url` to a
  connectable `net::Address` (host + resolved port) (§A.4).

`http` then `IMPORT net` and consumes `net::Url`. This is a deliberate layering:
the address/URL vocabulary lives with the transport, and `http` is a thin
protocol layer on top.

> **`net` becomes a hybrid package.** Today `net` is **native-only** (a Rust shim
> at `src/builtins/net.rs`; codegen at `src/target/shared/code/net.rs`; no
> MFBASIC source file). URL parsing is pure string work, so this plan gives `net`
> its **first source companion** — `src/builtins/net_package.mfb` — holding
> `Url`, `toUrl`, the `Url` rendering, and `toAddress`, while sockets/DNS/UDP stay
> native. This is the one structural change to `net`; §B/§C detail it.

> **Naming note.** The number `03` was historically attached to the network stack
> (`plan-03-net.md`, referenced in `src/builtins/tls.rs:6`/`:111`); that file is no
> longer in `specifications/` and `net`/`tls` have landed. This plan reuses
> `plan-03` for the HTTP layer plus the `net` URL additions it depends on.

> **Read/write shape (confirmed).** `read` is the body-less verb
> (`read(url, method = "GET")`) and `write` is the body-carrying verb
> (`write(url, body, method = "POST")`) — see §B.3. The method name is a trailing
> defaulted argument on both.

It complements:

- `specifications/standard_package.md` §11 (`net`) and §10.4 (`tls`)
- `specifications/error_codes.md` (the `7-705-*` generic and `7-707-*` network ranges)
- `specifications/mfbasic.md` (`TRAP`/`RECOVER`/`FAIL` error handling; universal `toString`)
- `specifications/plan-02-csv.md` (the source-package shim and wiring template this plan mirrors)

---

# Part A — `net` URL support

## A.1 `net::Url`

```basic
EXPORT TYPE Url
  scheme   AS String    ' "http" or "https" (lowercased)
  username AS String    ' userinfo before ':'  ("" if none)
  password AS String    ' userinfo after ':'   ("" if none)
  host     AS String    ' registered name or IP literal (IPv6 without brackets)
  port     AS Integer   ' explicit port, or the scheme default (80 / 443)
  path     AS String    ' begins with "/"; "/" when the href had none
  query    AS String    ' raw query without the leading '?'  ("" if none)
  fragment AS String    ' raw fragment without the leading '#'  ("" if none)
END TYPE
```

`Url` is a flat, **copyable value record** (no resource fields). It is declared
in `net_package.mfb` as an `EXPORT TYPE` and registered through
`net::is_builtin_type` (which already lists `Socket`/`Address`/… — §C.1) so it is
recognized as a built-in package type and accessed by field
(`url.host`, `url.port`), not by `MATCH`.

`Url` is **`http`-flavored**: §A.2 accepts only the `http`/`https` schemes and
applies their default ports. A fully scheme-generic URL type (for `ws`, `ftp`, …)
is a future generalization (§E); the field layout above is already general enough
to promote without churn.

## A.2 `net::toUrl`

`FUNC toUrl(href AS String) AS Url` parses an **absolute** URI:

```
scheme "://" [ userinfo "@" ] host [ ":" port ] path [ "?" query ] [ "#" fragment ]
```

Fixed decisions:

1. **Scheme** is matched case-insensitively and stored lowercased. Only `http`
   and `https` are accepted; any other scheme fails `ErrUnsupported` (`77050007`).
   A missing `://` (relative or scheme-less href) fails `ErrInvalidFormat`
   (`77050003`).
2. **Userinfo** (`user:pass@`) is optional. `username` is the part before the
   first `:`; `password` the part after. Both stored **as written** (no
   percent-decoding in v1 — §E).
3. **Host** is a DNS name, an IPv4 literal, or a bracketed IPv6 literal (`[::1]`).
   For IPv6 the brackets are stripped; `host` holds the inner address. An empty
   host fails `ErrInvalidFormat`.
4. **Port** is optional; absent → scheme default (`http` 80, `https` 443). A
   present port must be 0–65535 digits only, else `ErrInvalidFormat`.
5. **Path** defaults to `"/"` when absent; stored as written (not
   percent-normalized). It ends at the first `?` or `#`.
6. **Query** is between `?` and `#`, without the leading `?`. **Fragment** is
   after `#`, without the leading `#`. The fragment is parsed for completeness but
   is **never** sent on the wire (§B.3).
7. Parsing is byte-oriented over the same UTF-8 grapheme handling `json`/`csv`
   use (`strings::*`).

## A.3 `toString(url AS net::Url) AS String`

Renders a `Url` back to an absolute href — the inverse of `toUrl` — as
`scheme://[user[:pass]@]host[:port]path[?query][#fragment]`, omitting empty
userinfo, a port equal to the scheme default, an empty query, and an empty
fragment.

**Round-trip guarantee:** for any `Url` `u` produced by `toUrl`,
`toUrl(toString(u))` yields an equal `Url`.

`toString` is the **universal** built-in (`mfbasic.md` §18); overriding it for a
package type is an **already-supported pattern** — multiple built-in packages
provide their own `toString` rendering. `net` does the same for `Url`: a plain
`FUNC toString(value AS Url) AS String` declared in `net_package.mfb`, selected by
the existing `toString` overload resolution. **No new compiler mechanism is
required** — the override slots into the machinery already used for the other
packages' `toString` implementations.

## A.4 `net::toAddress`

`FUNC toAddress(url AS Url) AS Address` projects a `Url` onto a connectable
endpoint: `Address[host := url.host, port := url.port]`. Because `toUrl` always
fills `port` with the scheme default when absent, the result is always concrete
and feeds the existing `net::connectTcp(address AS Address, …)` overload directly.
`toAddress` performs **no DNS resolution** (it carries the host name through); use
`net::lookup` when a resolved IP is required.

`Address` is the existing native `net` record
(`Address[host AS String, port AS Integer]`, `net.rs:74`); `toAddress` constructs
it from the `Url` fields in `net_package.mfb`.

---

# Part B — the `http` package

## B.1 The functions

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::read` | `FUNC read(url AS net::Url, headers AS Map OF String TO String = {}, method AS String = "GET") AS Result` | Performs a **body-less** request (`GET`, `HEAD`, `DELETE`, `OPTIONS`, …) and returns the response. `headers` are extra request headers. Blocking. |
| `http::write` | `FUNC write(url AS net::Url, body AS String, headers AS Map OF String TO String = {}, method AS String = "POST") AS Result` | Performs a request **with a body** (`POST`, `PUT`, `PATCH`, …), sending `body`, and returns the response. `headers` are extra request headers. Blocking. |
| `http::header` | `FUNC header(result AS Result, name AS String) AS String` | Case-insensitive lookup of one response header value. Fails `ErrNotFound` when absent. |
| `http::headerOr` | `FUNC headerOr(result AS Result, name AS String, default AS String) AS String` | Case-insensitive header lookup, or `default` when absent. |

`IMPORT http` requires no manifest dependency — `http` is a built-in package.
Imports are **file-scoped** (`src/resolver.rs:391` builds a fresh import map per
file), so `http_package.mfb`'s internal `IMPORT net`/`IMPORT tls` are visible only
inside that file and **do not leak** into the user's namespace — exactly as
`json_package.mfb`'s internal `IMPORT collections` does not. A program that writes
`IMPORT http` therefore gets only `http::*`. Because the public surface takes a
`net::Url`, callers add their own `IMPORT net` (to name `net::Url` and call
`net::toUrl`); they **never** need `IMPORT tls`, which stays sealed inside `http`.

Both defaults are **trailing**, with `headers` before `method`. `headers` defaults
to `{}` — the empty map, which is the default value of any `Map OF String TO String`
(`mfbasic.md`: "Empty map, when `K` and `V` have default values"). So:

- `http::read(url)` — automatic headers (§B.3), method `GET`.
- `http::read(url, hdrs)` — extra headers `hdrs`, method `GET`.
- `http::read(url, hdrs, "DELETE")` — extra headers and an explicit method.

Ordering `headers` first keeps the common "add headers, keep the default method"
call to two arguments. This is sound because neither `read` nor `write` is
**overloaded** — defaults and overloading do not combine (`mfbasic.md` §6), and
these are distinct, single-declaration names.

## B.2 Types

```basic
EXPORT TYPE Header
  name  AS String        ' field name as received (case preserved)
  value AS String        ' field value, leading/trailing OWS trimmed
END TYPE

EXPORT TYPE Result
  status      AS Integer          ' status code, e.g. 200, 404
  reason      AS String           ' reason phrase, e.g. "OK" ("" if omitted)
  httpVersion AS String           ' "1.0" or "1.1" from the status line
  headers     AS List OF Header   ' response headers, in received order
  body        AS String           ' decoded response body (§B.4)
  ok          AS Boolean          ' TRUE iff status is in 200..299
END TYPE
```

Both are flat, **copyable value records**. The transport socket is opened, used,
and closed entirely inside `read`/`write`; **no handle escapes**, so a `Result` is
a plain value that can be returned, copied, and sent across threads. `Header` and
`Result` are declared in `http_package.mfb` and registered through
`http::is_builtin_type`.

## B.3 Request model

`read` and `write` perform one blocking request/response exchange and close the
connection (§B.5). They differ only in the body:

| | `http::read` | `http::write` |
|--|--------------|----------------|
| Default method | `"GET"` | `"POST"` |
| Request body | none (no `Content-Length`) | `body`, UTF-8-encoded |
| Typical methods | `GET`, `HEAD`, `DELETE`, `OPTIONS` | `POST`, `PUT`, `PATCH` |

The method is sent verbatim (uppercased; empty/non-token method fails
`ErrInvalidArgument`). The package does **not** enforce method/body agreement.

**Request line:** `METHOD <path>[?<query>] HTTP/1.1` (fragment never sent; empty
path normalized to `/`).

**Automatic headers** (the implementation always provides these):

| Header | Value |
|--------|-------|
| `Host` | `url.host`, plus `:port` when the port is not the scheme default. |
| `User-Agent` | `mfb-http/1` |
| `Accept` | `*/*` |
| `Connection` | `close` (fresh connection per request; read to EOF — §B.5). |
| `Content-Length` | UTF-8 byte length of the body. **`write` only.** |

**Caller headers (`headers` map).** Each entry adds or overrides a request header,
matched **case-insensitively** against the automatic set. So a caller may override
`User-Agent`, `Accept`, or `Host`, and add arbitrary headers (`Authorization`,
`Content-Type`, …). Two headers are **reserved by the implementation** and a
caller entry for them is ignored, to preserve correct framing:

- `Content-Length` — always the actual UTF-8 body length (`write`).
- `Connection` — always `close`, which the read-to-EOF model (§B.5) depends on.

Send order: the automatic headers first (with caller overrides applied in place),
then any remaining caller headers in the map's stable iteration order
(`mfbasic.md` §… map iteration). Header names are emitted as written; values are
sent verbatim (no folding). Userinfo in the `Url` is **not** auto-converted to
`Authorization` in v1 (§E) — pass it explicitly via `headers` if needed.

## B.4 Response model & parsing

With `Connection: close`, the client reads the socket to EOF, accumulating the
**entire** response into one buffer, then parses it with pure string code.

- **Status line** `HTTP/<v> <status> [reason]` → `httpVersion`, `status`,
  `reason`. A non-status-line first line fails `ErrInvalidFormat`.
- **Headers** read until the first empty line; each `name: value` becomes a
  `Header` (value OWS-trimmed). Duplicates preserved in order;
  `http::header`/`headerOr` match case-insensitively and return the first.
- **`ok`** = `status` in `200..=299`. **Redirects are not followed** (§E): a
  `3xx` returns as-is with `ok = FALSE` and `Location` available via `header`.
- **Body**, by precedence:
  1. `Transfer-Encoding: chunked` → de-chunked (hex length line, data, `CRLF`;
     `0`-chunk terminates; trailers ignored). Malformed framing →
     `ErrInvalidFormat`.
  2. else `Content-Length: N` → exactly `N` bytes; a short read →
     `ErrConnectionClosed` (`77070004`).
  3. else → everything until EOF (HTTP/1.0 `Connection: close`).
  A `HEAD` response and `204`/`304` carry no body (`body = ""`).
- **Decoding:** v1 decodes the body as **UTF-8 text** into `Result.body`
  (consistent with `net::readText`/`tls::readText`). The read loop accumulates
  **bytes** (`net::read`/`tls::read` → `List OF Byte`) so chunk/`Content-Length`
  accounting is byte-exact, then decodes once via the **existing**
  `toString(List OF Byte) AS String` conversion (already used in
  `json_package.mfb:49`; recognized by `general::resolve_call` for `["List OF Byte"]`).
  Binary bodies are a known limitation; a future `http::readBytes → List OF Byte`
  is the remedy (§E).
- **Size cap:** the response is bounded by an internal cap (default 64 MiB);
  exceeding it fails `ErrOverflow` (`77050010`).

## B.5 Blocking transport & platform support

`read`/`write` select transport by `url.scheme`:

| Scheme | Connect | I/O | Close |
|--------|---------|-----|-------|
| `http` | `net::connectTcp(net::toAddress(url))` | `net::write` / `net::read` | `net::close` |
| `https` | `tls::connect(url.host, url.port, 0, url.host)` | `tls::write` / `tls::read` | `tls::close` |

Resource handles cannot be stored in a union or `List` (the ownership model
forbids resource elements — `net.rs:156`), so the `Socket` and `TlsSocket` paths
**cannot share a single variable**. The implementation has two thin transport
branches (connect → send → read-to-EOF → close), both delegating to the **shared
pure helpers** for request building (§B.3) and response parsing (§B.4). The bulk
of `http` — request serialization, response parsing, chunk decoding, header
lookup — is transport-agnostic string code.

**Read-to-EOF.** `net::read`/`tls::read` return a non-empty chunk or **fail with
`ErrConnectionClosed` (`77070004`) at end of stream** (`standard_package.md` §11).
The accumulation loop wraps the read in an inline `TRAP` and treats `77070004` as
end-of-response (`RECOVER` → stop); any other error propagates.

**Platform support.**

| | `http://` | `https://` |
|--|-----------|-------------|
| Linux aarch64 | ✅ via `net` | ✅ via `tls` (system OpenSSL 1.1/3 by `dlopen`) |
| macOS aarch64 | ✅ via `net` | ✅ via `tls` (Network.framework, `tls.rs:7`) |

Both transports work on both targets: `tls` drives system OpenSSL on Linux and
Network.framework on macOS (`tls.rs:4-8`), so `http` adds no platform code and has
no platform gate.

**Timeouts.** v1 uses the `net`/`tls` default connect timeout and no explicit
per-read timeout. A `timeoutMs` parameter is a likely v1.1 addition (§E).

---

# Part C — Implementation Plan

The work parallels the `json`/`csv` source-package template, with two shims
(`net` gains a source companion; `http` is new). A compiler review confirms that
**almost everything `http` needs already exists** — `http` is mostly glue.

## C.0 Reuse inventory

| `http` need | Reused as | Source |
|-------------|-----------|--------|
| Plaintext connect / read / write / close | `net::connectTcp`, `net::read`/`readText`, `net::write`/`writeText`, `net::close` | `src/builtins/net.rs` |
| TLS connect / read / write / close | `tls::connect`, `tls::read`/`readText`, `tls::write`/`writeText`, `tls::close` | `src/builtins/tls.rs` |
| End-of-stream detection | `net`/`tls` read **fail with `ErrConnectionClosed`** at EOF, caught by inline `TRAP`/`RECOVER` | `standard_package.md` §11; `mfbasic.md` §… |
| Endpoint to connect to | `net::Address` + `net::connectTcp(Address)` overload (fed by `net::toAddress`) | `net.rs:74`, `:136` |
| **Byte → text decode** of the body | `toString(List OF Byte) AS String` | `general.rs` `resolve_call`; used in `json_package.mfb:49` |
| Text → wire bytes (request) | `net::writeText`/`tls::writeText` (UTF-8 encode internally) | `net.rs:18`, `tls.rs:17` |
| Body byte length for `Content-Length` | `strings::byteLen` | `mfbasic.md` §18 |
| Case-insensitive header matching | `strings::lower`/`caseFold`, `strings::split`, `strings::trim`, `find`, `mid` | `mfbasic.md` §18 |
| Headers map / header list / response building | `collections::*` (`get`, `hasKey`, `keys`, `append`, `Map`/`List`) | `collections` |
| Source-package shim (all 11 hooks) | copy `src/builtins/json.rs` structure verbatim; embed via `crate::ast::parse_source_internal` | `json.rs:105` |
| Trailing-default arguments | `default_argument_padding` | `regex.rs`, `tls.rs:112` |
| Built-in record types with no reserved IDs | `EXPORT TYPE` in the `.mfb` + `is_builtin_type` | `json.rs:18`, `json_package.mfb:4` |
| Cursor threading in the parser | small node record (`__HttpScan { text, index }`) | pattern from `json_package.mfb:42` |
| Man-page generation | `man_pages` / `write_pages` / `parse_package` | `build.rs`, `src/man/mod.rs` |

**Genuinely new (the only code with real logic):** the HTTP/1.1 protocol itself —
URL parse/format (`net`), request serialization, status-line + header parsing,
chunked decoding, and header-map merging. The `net` docs are explicit that the
transport helpers add **no framing** ("Programs that exchange records should define
their own delimiter, length prefix, or protocol parser" — `standard_package.md`
§11), so this protocol layer is exactly the part `net` does *not* provide and the
part this plan adds. Everything below the protocol — transport, TLS, byte/text
conversion, collections, the package scaffolding — is reused.

## Phase 1 — `net` URL source companion

**`src/builtins/net_package.mfb`** (new — `net`'s first source file). Header
`IMPORT strings`. Contents:

- `EXPORT TYPE Url` (§A.1).
- `__net_toUrl(href AS String) AS Url` — the parser (§A.2); `FAIL error(...)` with
  `ErrInvalidFormat`/`ErrUnsupported` per §A.2.
- `FUNC toString(value AS Url) AS String` — the renderer (§A.3); a normal
  `toString` overload, exactly as other built-in packages already provide.
- `__net_toAddress(url AS Url) AS Address` — `RETURN Address[url.host, url.port]`
  (§A.4). (Verify a source `.mfb` can construct the native `Address` record; if
  not, implement `toAddress` as a tiny native shim instead — it is a pure field
  copy either way.)

**`src/builtins/net.rs`** (extend the existing native shim):

- Add consts `URL_TYPE = "Url"`, call names `TO_URL = "net.toUrl"`,
  `TO_ADDRESS = "net.toAddress"`, and targets `__net_toUrl` / `__net_toAddress`.
- `is_builtin_type`: add `URL_TYPE`.
- `is_net_call`: add `TO_URL`, `TO_ADDRESS`.
- `call_param_names`: `toUrl → &[&["href","value","url"]]`,
  `toAddress → &[&["url"]]`.
- `call_return_type_name`: `toUrl → "Url"`, `toAddress → ADDRESS_TYPE`.
- `resolve_call`: `toUrl ["String"] → "Url"`; `toAddress ["Url"] → "Address"`.
- `expected_arguments`/`argument_types`/`arity`: `toUrl (1,1)`, `toAddress (1,1)`.
- `implementation_name(name)`: `toUrl → __net_toUrl`, `toAddress → __net_toAddress`
  (new for `net`).
- The `Url` `toString` override needs no shim entry: it is a `FUNC toString(Url)`
  in `net_package.mfb`, resolved by the existing `toString` overloading (§A.3).
- Source-package hooks (new for `net`), copied from `json.rs:105`: `source_file()`
  → `crate::ast::parse_source_internal(Path::new("<builtin-net>"), "builtins/net.mfb", include_str!("net_package.mfb"))`,
  `uses_package(ast)` → any import `package_name() == "net"`,
  `augmented_project(ast)` → clone + push `source_file()` when used.

## Phase 2 — `http` Rust shim: `src/builtins/http.rs`

Modeled on `json.rs` + `regex.rs`. `pub(crate)` surface:

- Consts: `READ = "http.read"`, `WRITE = "http.write"`, `HEADER = "http.header"`,
  `HEADER_OR = "http.headerOr"`; types `HEADER_TYPE = "Header"`,
  `RESULT_TYPE = "Result"`; targets `__http_read`/`__http_write`/`__http_header`/
  `__http_headerOr`.
- `is_builtin_type` → `"Header" | "Result"`.
- `is_http_call` → the four names.
- `call_param_names`: `read → &[&["url"],&["headers"],&["method"]]`,
  `write → &[&["url"],&["body"],&["headers"],&["method"]]`,
  `header → &[&["result"],&["name"]]`,
  `headerOr → &[&["result"],&["name"],&["default","fallback"]]`.
- `call_return_type_name`: `read`/`write` → `"Result"`; `header`/`headerOr` →
  `"String"`.
- `resolve_call` (let `M = "Map OF String TO String"`):
  `read ["Url"]|["Url",M]|["Url",M,"String"] → "Result"`;
  `write ["Url","String"]|["Url","String",M]|["Url","String",M,"String"] → "Result"`;
  `header ["Result","String"] → "String"`;
  `headerOr ["Result","String","String"] → "String"`.
- `expected_arguments`: `"Url, Map OF String TO String, String"` /
  `"Url, String, Map OF String TO String, String"` / `"Result, String"` /
  `"Result, String, String"`.
- `arity`: `read (1,3)`, `write (2,4)`, `header (2,2)`, `headerOr (3,3)`.
- `default_argument_padding` (defaults are the empty map then the method):
  `read` → `[("Map OF String TO String","{}"),("String","GET")][provided.saturating_sub(1)..]`;
  `write` → `[("Map OF String TO String","{}"),("String","POST")][provided.saturating_sub(2)..]`.
- `implementation_name` → the `__http_*` targets.
- `source_file`/`uses_package`/`augmented_project` over
  `include_str!("http_package.mfb")` (same shape as `json.rs:105`, via
  `crate::ast::parse_source_internal`).

> No reserved type IDs, no `consumes_argument`, no `resource_close_function`:
> `http`'s types are plain records and it owns no resource handles.

## Phase 3 — `http` MFBASIC implementation: `src/builtins/http_package.mfb`

Header: `IMPORT net`, `IMPORT tls`, `IMPORT strings`, `IMPORT collections`,
`IMPORT errorCode`. Export `Header`/`Result`; implement the four `__http_*` plus
private helpers:

- **Pure (shared):** `__http_buildRequest(method, url, body, hasBody, headers)`
  (merges the automatic headers with the caller `headers` map per §B.3, forcing
  the reserved `Content-Length`/`Connection`),
  `__http_parseResponse(raw AS String) AS Result`, `__http_dechunk`,
  `__http_findHeader` (case-insensitive).
- **Transport branches:** `__http_exchangeTcp(url, requestText) AS String` (via
  `net::*`, using `net::connectTcp(net::toAddress(url))`) and
  `__http_exchangeTls(url, requestText) AS String` (via `tls::*`; opens with the
  macOS `ErrUnsupported` capability check). Each: connect, `writeText`, loop
  reads accumulating until `ErrConnectionClosed`, `close`, return the raw bytes.
- `__http_read`/`__http_write` compose build (passing the `headers` map) →
  exchange (branch on `url.scheme`) → parse → `Result`.
  `__http_header`/`__http_headerOr` wrap `__http_findHeader`.

Read loop shape (per §B.5; `net` branch identical with `net::readText`):

```basic
MUT raw AS String = ""
MUT more AS Boolean = TRUE
WHILE more
  LET chunk AS String = tls::readText(sock, 65536) TRAP
    IF e.code = errorCode::ErrConnectionClosed THEN RECOVER ""
    PROPAGATE
  END TRAP
  IF chunk = "" THEN
    SET more = FALSE
  ELSE
    SET raw = raw & chunk
    IF strings::byteLen(raw) > __HTTP_MAX THEN FAIL error(errorCode::ErrOverflow, "response too large")
  END IF
END WHILE
```

**MFBASIC source-package constraints** (carry over the regex/json gotchas):
reserved words cannot be identifiers; ≤ 8 parameters per function; no direct
field assignment (build records with constructor syntax; thread cursor state via
small node records, e.g. `__HttpScan { text, index }`); cross-file visibility
needs `EXPORT`; escape `\r\n` in literals.

## Phase 4 — Wire both shims into the compiler

`net` (now hybrid) and `http` (new) both need registration:

- `src/builtins/mod.rs`:
  - `pub(crate) mod http;` (`net` already declared).
  - `is_builtin_import`: add `"http"`.
  - `is_builtin_type`: `net` already chained (now matches `Url` too); add
    `http::is_builtin_type`.
  - `call_return_type_name`: add `http::call_return_type_name` (net already
    chained).
  - `is_builtin_call`: add `http::is_http_call` (net already chained).
  - `call_param_names`: add `http::call_param_names` (net already chained).
- `src/resolver.rs:42` — chain `net::augmented_project` **and**
  `http::augmented_project` into the `json` → `regex` sequence.
- `src/typecheck.rs:117` — same two `augmented_project` additions; add the
  `is_http_call` dispatch + `check_http_builtin_call` near `:4606`; add
  `http::arity` near `:4852`, `http::resolve_call`/`expected_arguments` near
  `:4872`; ensure `net::toUrl`/`toAddress` flow through the existing
  `check_net_builtin_call` path.
- `src/ir.rs:411` — same two `augmented_project` additions; add
  `http::default_argument_padding` near `:2710`; `net::implementation_name` and
  `http::implementation_name` near `:2725`; `net`/`http` `resolve_call`/
  `expected_arguments` near `:2229`/`:2391`. The `Url` `toString` override is a
  source `FUNC toString(Url)` resolved by the existing `toString` overloading — no
  change at the `toString` lowering site (`:2366` / `builtins::general`) is needed.

> **Augmentation ordering.** `json` then `regex` apply in sequence. Append
> `net` then `http` after them in `resolver.rs`/`typecheck.rs`/`ir.rs` (all three
> must agree). `http`'s source `IMPORT net`, so `net`'s augmentation must be in
> the chain before `http`'s typecheck resolves `net::Url`.

## Phase 5 — Man pages

- `src/man/builtins/net/`: add `toUrl.txt`, `toAddress.txt`, and a `Url`
  rendering note in `package.txt` (the `net` man dir already exists; just add
  pages and chain them in `build.rs`).
- `src/man/builtins/http/`: `package.txt`, `read.txt`, `write.txt`, `header.txt`,
  `headerOr.txt` (NAME / SYNOPSIS / DESCRIPTION / ERRORS / EXAMPLES; `read`/`write`
  note the platform matrix). Wire `build.rs` (`http_dir`, `man_pages`,
  `rerun-if-changed`, `write_pages(..., "HTTP_FUNCTION_PAGES", …)`) and
  `src/man/mod.rs` (`parse_package(... "builtins/http/package.txt" …)` +
  `"http" => Some(generated::HTTP_FUNCTION_PAGES)`).

## Phase 6 — User documentation

- `specifications/standard_package.md`: add `net::toUrl`/`toString(Url)`/
  `net::toAddress` and the `Url` type to §11 (Net); add a new "Built-in HTTP
  Package" section after §12 (JSON) with the `Header`/`Result` blocks, the §B.1
  table, and the request/response/transport/platform summaries.
- `specifications/error_codes.md`: no new codes; add `net`(URL) and `http` to the
  "used by" notes for `ErrInvalidFormat`, `ErrUnsupported`, `ErrOverflow`,
  `ErrInvalidArgument`, `ErrNotFound`, and the `7-707-*` rows `http` propagates.

## Phase 7 — Tests (golden)

Mirror `tests/func_json_*`. Split by network dependence:

**Offline (deterministic, the bulk):**

- `func_net_tourl_valid` / `func_net_tourl_invalid[_runtime]` — field-by-field
  parse incl. IPv6, default-port omission, userinfo; missing `://`
  (`ErrInvalidFormat`); non-`http(s)` scheme (`ErrUnsupported`).
- `func_net_url_tostring_valid` — `toUrl`→`toString`→`toUrl` round-trip.
- `func_net_toaddress_valid` — `Url` → `Address` host/port.
- `func_http_parse_*` — drive `__http_parseResponse` over fixed raw responses:
  `Content-Length`, chunked, EOF-framed, `HEAD`/`204`, malformed status line, bad
  chunk; assert `status`/`reason`/`headers`/`body`/`ok`.
- `func_http_header_valid` / `func_http_headerOr_valid` — case-insensitive
  lookup; missing default and `ErrNotFound`.
- `func_http_buildrequest_valid` — drive `__http_buildRequest` over an empty map,
  a custom-header map, an override of `User-Agent`/`Host`, and an attempt to set
  the reserved `Content-Length`/`Connection` (asserting they are forced); check
  the serialized request line + header block byte-for-byte.

**Networked (gated; on-device, not the default sweep):**

- `http_read_loopback` / `http_write_loopback` — a `net::listenTcp` worker serves
  a canned response; `http::read`/`write` against `127.0.0.1`; assert the
  round-trip (hermetic, no external host).
- `http_https_loopback` — a `tls`-terminated loopback (Linux and macOS) exercising
  the `https` transport branch end-to-end.

Generate goldens with `scripts/sync-goldens.sh`; verify offline ones via
`scripts/test-accept.sh`, and loopback/`https` on-device (Linux aarch64 for TLS).

---

# Part D — Worked example

```basic
IMPORT net
IMPORT http
IMPORT io

FUNC main AS Integer
  ' Parse + inspect the URL (lives in net::).
  LET url AS net::Url = net::toUrl("https://api.example.com:8443/v1/items?limit=10#frag")
  io::print(url.host)                       ' api.example.com
  io::print(toString(url.port))             ' 8443
  io::print(url.path)                       ' /v1/items
  io::print(toString(url))                  ' https://api.example.com:8443/v1/items?limit=10#frag
  LET addr AS net::Address = net::toAddress(url)   ' Address[host:=api.example.com, port:=8443]

  ' GET with no extra headers (method defaults to GET).
  LET got AS http::Result = http::read(url)
  io::print(toString(got.status) & " " & got.reason)      ' 200 OK
  IF got.ok THEN io::print(http::headerOr(got, "Content-Type", "?"))
  io::print(got.body)

  ' GET with custom request headers.
  LET auth AS Map OF String TO String = { "Authorization" := "Bearer xyz", "Accept" := "application/json" }
  LET secured AS http::Result = http::read(url, auth)

  ' POST a body, with a content type (method defaults to POST).
  LET posted AS http::Result = http::write(net::toUrl("https://api.example.com/v1/items"), "{\"name\":\"a\"}", { "Content-Type" := "application/json" })
  io::print(toString(posted.status))                      ' 201

  ' Explicit method on the body-less verb (empty headers, then the method):
  LET deleted AS http::Result = http::read(url, {}, "DELETE")
  RETURN 0
END FUNC
```

No socket is visible to the caller: each `http::read`/`write` opens, exchanges,
and closes its connection internally, returning a plain `Result`.

---

# Part E — Divergences, errors, and non-goals

## E.1 Divergences from `json`/`csv`

| Aspect | `json`/`csv` | this plan |
|--------|--------------|-----------|
| Computation | pure | URL/HTTP parsing **+ native I/O** via `net`/`tls` |
| Package shape | one source package | `net` becomes **hybrid** (native + source companion); `http` is a source package |
| `.mfb` imports | `collections`, `strings` | `http` also imports `net`, `tls`, `errorCode` |
| Determinism | total fn of input | URL/parse pure; `read`/`write` depend on a remote server |
| Platform parity | identical | identical — `https` works on Linux and macOS via `tls` |
| `toString` | structural default | `net::Url` gets a `toString` overload (existing pattern, §A.3) |
| Resources | none | none in the public surface (sockets internal, closed before return) |

## E.2 Errors

No new error codes; reuse `7-705-*` and propagate `7-707-*`:

| Condition | Code |
|-----------|------|
| `net::toUrl` malformed href (no `://`, empty host, bad port) | `ErrInvalidFormat` (`77050003`) |
| `net::toUrl` non-`http(s)` scheme | `ErrUnsupported` (`77050007`) |
| Empty / non-token method | `ErrInvalidArgument` (`77050002`) |
| Malformed status line / header block / chunk framing | `ErrInvalidFormat` (`77050003`) |
| Response exceeds the size cap | `ErrOverflow` (`77050010`) |
| `http::header` lookup miss | `ErrNotFound` (`77050004`) |
| DNS / connect / TLS / read-write transport failures | propagated from `net`/`tls`: `77070001`–`77070006`, `77050008`, `77020004` |

A short read against a declared `Content-Length` surfaces the underlying
`ErrConnectionClosed` (`77070004`); a clean EOF for EOF-framed bodies is the
normal terminator, not an error (§B.5).

## E.3 Non-goals for v1

- **Response-header mutation / request-header folding.** Request headers are
  supported via the `headers` map (§B.3); the reserved `Content-Length`/`Connection`
  remain implementation-controlled, and multi-value folding is not provided.
- **Automatic redirect following** (`3xx` returned as-is).
- **Authentication** (userinfo parsed but unused; no cookies/auth schemes).
- **Binary response bodies** (`Result.body` is UTF-8 text; future
  `http::readBytes → List OF Byte`).
- **Per-call timeouts / streaming / keep-alive / pipelining** (each call is one
  connect-exchange-close; `timeoutMs` is the likely v1.1 add).
- **Compression / proxies / HTTP/2+.**
- **Percent-encoding/decoding and query-string parsing** (future `net`/`url`
  helpers).
- **A configurable response size cap** (fixed 64 MiB).
- **A scheme-generic `Url`** (v1 `net::Url` is `http`/`https`-flavored; the field
  layout is general enough to promote later for `ws`/`ftp`).

These keep v1 a clean, blocking, batteries-light client while honoring the
requested shape: a `Url` and `toUrl` in `net::` (with `toString` and `toAddress`),
and `http::read`/`write` — each method-named-and-defaulted, each over a
`net::Url`, each returning a `http::Result`.

---

# Part F — HTTP Server (routing, single-threaded)

Part F adds an HTTP **server** to the same `http` package. It is the server-side
sibling of the client (Parts A–E): same package, same source idiom (`http.rs` shim
+ `http_package.mfb`), and it **layers on the existing native `net` package** for
transport (`net::listenTcp`, `net::accept`, socket I/O, `net::close`). No new Rust
intrinsics or syscalls are introduced.

The two halves of `http` are **implementation-independent** — the server may land
before or after the client. They share only the package namespace and the `net`
transport. Names do not collide: the client contributes `read`/`write`/`header`/
`headerOr` and the `Result` record; the server contributes `server`/`handleRequest`/
`serverSSL`/`route`/the response constructors and the
`Request`/`Response`/`Route`/`RequestPart` records.

## F.1 Design in one paragraph

A server is a **single-threaded, blocking, user-driven accept loop**. There is no
event loop, no async, no threads, no per-connection workers. The user constructs a
`Server` resource, builds an **ordered** `List OF Route` mapping path patterns to
handler functions, and calls `http::handleRequest(server, routes)` in their own
`DO/LOOP`. The user obtains a listener (`http::server` returns a `net::Listener`
directly — no separate server resource, §F.2.5; `http::serverSSL` returns a
`tls::Listener` for HTTPS, §F.5.6). Each call accepts one
connection, parses the request into a plain
`Request` value, matches its **path** (query string stripped first) against the
routes **in list order — first match wins**, invokes the matched handler
(`FUNC(Request) AS Response`), writes the returned `Response`, and closes the
connection. A handler that errors becomes a `500`; no matching route becomes a
`404`; the loop never dies on a bad request.

This shape is possible because functions are **first-class** in MFBASIC: a plain
(non-`ISOLATED`) `FUNC` can be stored in a `List`/`Map` and called indirectly
through a variable (`mfbasic.md` function types; `tests/func_collection_*`). The
handler type `FUNC(http::Request) AS http::Response` is a normal function-typed
value.

## F.2 Types

All record types are flat, **copyable value records** (no resource fields). They
are declared in `http_package.mfb` and registered through `http::is_builtin_type`.
The server introduces **no new resource type** — `http::server` returns the
existing `net::Listener` (§F.2.5).

### F.2.1 `Request`

```basic
EXPORT TYPE Request
  method  AS String                          ' "GET", "POST", ... (uppercased)
  path    AS String                          ' "/test/42" — query already stripped, percent-decoded
  rawPath AS String                          ' the request-target as received (pre-decode, pre-strip)
  headers AS Map OF String TO String         ' field names lowercased; last-wins on duplicates
  query   AS Map OF String TO String         ' from "?a=1&b=2"; percent-decoded; last-wins
  params  AS Map OF String TO String         ' from ":testId" / ":bar?" / "*" route captures
  parts   AS Map OF String TO RequestPart    ' multipart/form-data parts, keyed by part name
  body    AS List OF Byte                     ' raw request body (empty for body-less methods)
END TYPE
```

- **`headers`** keys are normalized to **lowercase** (HTTP field names are
  case-insensitive), so `req.headers["host"]` always finds `Host`. Duplicate
  fields collapse **last-wins**; the multi-value `List` representation is a non-goal
  (§F.8).
- **`query`** is parsed from the part after the first `?`, split on `&` then `=`,
  with **percent-decoding** (`%20`, `+` → space) applied to keys and values.
  Last-wins on repeated keys.
- **`params`** is filled by route matching (§F.3): named segments (`:testId`),
  optional segments (`:bar?` — present only when matched), and the wildcard
  remainder under the key `"*"`.
- **`parts`** is populated only for `Content-Type: multipart/form-data` bodies
  (§F.4.3); empty otherwise. `body` always holds the raw bytes regardless.

### F.2.2 `RequestPart`

```basic
EXPORT TYPE RequestPart
  filename    AS String          ' from the part's Content-Disposition; "" if a plain field
  contentType AS String          ' the part's Content-Type; "" if absent
  body        AS List OF Byte    ' the part's raw bytes
END TYPE
```

One entry per multipart part, keyed in `Request.parts` by the part's `name` (from
its `Content-Disposition`). A file upload has a non-empty `filename`; a plain form
field has `filename = ""` and its text in `body`.

### F.2.3 `Response`

```basic
EXPORT TYPE Response
  status  AS Integer                     ' status code, e.g. 200, 404, 500
  reason  AS String                      ' reason phrase; "" → derived from status
  headers AS Map OF String TO String     ' response headers (names sent as written)
  body    AS List OF Byte                ' response body bytes
END TYPE
```

`Response` is a plain value record. It is **immutable in place** — MFBASIC has no
field-target assignment for plain records (assignment targets are identifiers
only). Handlers build and edit a response with **`WITH`** (functional update
producing a copy) over a `MUT` binding, which reads like mutation:

```basic
MUT resp AS http::Response = http::responseDefault()   ' 200 + "OK"
resp = WITH resp { status := 404 }
resp = WITH resp { body := http::bytes("nope") }
RETURN resp
```

The body is **`List OF Byte`** so binary responses (images, fonts) work; the
`http::ok`/`http::status` constructors and `http::bytes` provide the String-in
convenience (§F.5.3). `reason` may be left `""`; `handleRequest` derives the
standard phrase from `status` when emitting the status line.

### F.2.4 `Route`

```basic
EXPORT TYPE Route
  pattern AS String                              ' "/test/:testId", "/foo/:bar?", "/static/*"
  handler AS FUNC(Request) AS Response
END TYPE
```

`http::route(pattern, handler)` (§F.5.2) is a constructor convenience returning
`Route[pattern, handler]`; the literal form `Route["/", home]` works equally. A
program holds its routes in a `List OF http::Route`, **ordered** — matching is
first-in-list-wins (§F.3.3).

### F.2.5 No server resource — `http::server` returns `net::Listener`

The server introduces **no `http::Server` type**. `http::server(port, …)` returns
the existing `net::Listener` directly, and `http::handleRequest` accepts a
`net::Listener`. There is no server-level mutable state to carry — configuration is
passed per call (the `routes` list), the size cap is a fixed constant, and there is
no connection pool or keep-alive — so a wrapper resource would add a type, a close
function, and the resource-containing-resource question for zero benefit. The
listener is owned by the caller's `RES` binding and closed by lexical drop at scope
exit, exactly as `net::listenTcp` already provides.

Callers therefore `IMPORT net` to name the type (`RES s AS net::Listener`), the
same way the client's callers `IMPORT net` to name `net::Url` (§B.1). If a later
feature ever needs real server state (e.g. a keep-alive pool), promoting to a
dedicated resource is a backward-compatible change.

The **secure** variant `http::serverSSL` (§F.5.6) returns a `tls::Listener`
instead. That one *is* a state-bearing resource — it owns the bound socket **and**
the loaded server credentials (certificate + private key) — so unlike the plaintext
case it justifies a dedicated resource, provided by the `tls` package (not `http`).

## F.3 Routing & path matching

### F.3.1 Match on the path only

`handleRequest` splits the request target at the **first `?`**. Everything before
is the path (percent-decoded into `Request.path`); everything after is the query
(parsed into `Request.query`, §F.4.2). **Route patterns match against the path
only** — a pattern never sees `?...`. A request to `/test/42?debug=1` matches the
pattern `/test/:testId`, binds `testId = "42"`, and exposes `debug = "1"` via
`req.query`.

### F.3.2 Pattern syntax

Patterns are matched **segment by segment** (split on `/`). A segment is one of:

| Segment | Meaning | Binds |
|---------|---------|-------|
| literal (`foo`) | must equal the path segment exactly | — |
| `:name` | captures exactly one path segment | `params["name"]` |
| `:name?` | **trailing** optional segment — matches with or without it | `params["name"]` when present |
| `*` | **trailing** catch-all — captures the entire remaining path | `params["*"]` |

Rules:

1. **`:name?` and `*` are legal only as the final segment(s).** A mid-pattern
   optional or wildcard is rejected at registration (`ErrInvalidArgument`) — it
   makes matching ambiguous and is a non-goal. Multiple trailing optionals are
   allowed (`/foo/:a?/:b?`).
2. `:name` matches exactly one non-empty segment; `/test/:id` does **not** match
   `/test` or `/test/42/extra`.
3. `*` matches the rest of the path including embedded `/`; `/static/*` matches
   `/static/css/app.css` and binds `params["*"] = "css/app.css"`.
4. A trailing slash is **normalized away** before matching (`/foo/` ≡ `/foo`),
   except the root path `/` itself.

### F.3.3 Ordered, first-match-wins

Routes are tried **in the order they appear in the list**. The first pattern that
matches handles the request. Precedence is therefore explicit and entirely the
caller's choice — there is no "most specific wins" heuristic. To let a literal beat
a pattern, list it first:

```basic
collections::listOf(
  http::route("/foo/new",   newFoo),     ' literal — listed first, so it wins
  http::route("/foo/:bar?", showFoo)     ' optional-param fallback
)
```

The cost of this rule: a broad pattern placed before a specific literal silently
shadows the literal. This is documented behavior, not a defect — it is how the
order-based model is meant to work.

### F.3.4 No match → 404

If no route matches the path, `handleRequest` emits a built-in **`404`** response
(`text/plain`, body `"Not Found"`). The body/format of the default 404 is fixed in
v1; a user-supplied fallback route (`http::route("/*", myNotFound)` listed last) is
the way to customize it.

## F.4 Request parsing

`handleRequest` reads one full HTTP/1.1 request from the accepted socket and parses
it with pure string/byte code, mirroring the response parser in §B.4.

### F.4.1 Request line & headers

- **Request line** `METHOD <request-target> HTTP/<v>` → `method` (uppercased),
  `rawPath`/`path`/`query` (§F.3.1/§F.4.2). A malformed first line fails
  `ErrInvalidFormat` and is answered with `400`.
- **Headers** are read until the first empty line; each `name: value` becomes a
  `headers` entry with the **name lowercased** and the value OWS-trimmed.
  Duplicates collapse last-wins.

### F.4.2 Path & query decoding

- The request-target is split at the first `?`. The path portion is
  **percent-decoded** into `Request.path`; the raw target is preserved in
  `rawPath`.
- The query portion is split on `&`, each pair on the first `=`, with keys and
  values **percent-decoded** (`%XX` and `+` → space). Result → `Request.query`,
  last-wins on repeats.
- Percent-decoding and query splitting are **new helpers** (§E.3 lists
  query-string parsing as a future `net`/`url` addition). Per the layering
  philosophy in Part A ("the URL/address vocabulary lives with the transport"),
  they are added to **`net`** — `net::percentDecode(s) AS String` and
  `net::parseQuery(s) AS Map OF String TO String` — and consumed by `http`.

### F.4.3 Body & multipart parts

- **Body framing** follows the request headers, mirroring the client's response
  rules (§B.4): `Content-Length: N` reads exactly `N` bytes;
  `Transfer-Encoding: chunked` is de-chunked; body-less methods (`GET`, `HEAD`,
  `DELETE`, no `Content-Length`) yield an empty body. The raw bytes always land in
  `Request.body`.
- **Multipart** parsing runs only when `Content-Type` is `multipart/form-data` with
  a `boundary`. The body is split on the boundary delimiter; each part's own
  headers (`Content-Disposition`, `Content-Type`) are parsed to fill a
  `RequestPart` (`name` → map key, `filename`, `contentType`, `body`). Malformed
  multipart framing fails `ErrInvalidFormat` → `400`. This boundary parser is the
  single largest piece of genuinely new logic the server adds.
- **Size cap.** The request is bounded by the same internal cap the client uses
  (default 64 MiB, §B.4); exceeding it fails `ErrOverflow` → `413`.

## F.5 The server API

### F.5.1 Lifecycle

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::server` | `FUNC server(port AS Integer, host AS String = "0.0.0.0", backlog AS Integer = 128) AS net::Listener` | Binds and listens (plaintext) via `net::listenTcp`; returns the listener. Blocking; closed by lexical drop. |
| `http::serverSSL` | `FUNC serverSSL(port AS Integer, certPath AS String, keyPath AS String, host AS String = "0.0.0.0", backlog AS Integer = 128) AS tls::Listener` | Binds a **TLS** listener via `tls::listen`, loading the PEM server certificate (`certPath`) and private key (`keyPath`). Returns a `tls::Listener`. Blocking; closed by lexical drop. Works on Linux and macOS (§F.5.6). |
| `http::handleRequest` | `FUNC handleRequest(listener AS net::Listener, routes AS List OF Route) AS Nothing`<br>`FUNC handleRequest(listener AS tls::Listener, routes AS List OF Route) AS Nothing` | **Overloaded by listener type.** Accepts **one** connection (`net::accept`, or `tls::accept` performing the server-side handshake), parses the request, matches the path against `routes` in order, invokes the handler, writes the response, closes the connection. Both overloads share one pure parse/match/dispatch/emit core; only the transport (plain `Socket` vs `TlsSocket`) differs. |

`handleRequest` is **crash-proof**: the handler invocation is wrapped in a `TRAP`.
A handler that fails (any propagated error) is answered with a built-in **`500`**
(`text/plain`, `"Internal Server Error"`) and the loop continues. Connection-level
I/O errors (peer reset mid-read/-write) are caught, the connection is dropped, and
`handleRequest` returns normally — one bad client never tears down the server.

```basic
RES s AS http::Server = http::server(8080)
DO
  http::handleRequest(s, routes)
LOOP
```

### F.5.2 Routing constructor

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::route` | `FUNC route(pattern AS String, handler AS FUNC(Request) AS Response) AS Route` | Builds a `Route`; validates the pattern (trailing-only `:name?`/`*`, §F.3.2) and fails `ErrInvalidArgument` on a malformed pattern. |

### F.5.3 Response constructors & combinators

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::responseDefault` | `FUNC responseDefault() AS Response` | A `200` with reason `"OK"`, body `"OK"`, no extra headers. The convenient starting point for `WITH` edits (not required — `ok`/`status` build directly). |
| `http::ok` | `FUNC ok(body AS String) AS Response` | `200` with `body` (UTF-8) and `Content-Type: text/plain; charset=utf-8`. |
| `http::status` | `FUNC status(code AS Integer, body AS String) AS Response` | Arbitrary status with a text body. |
| `http::json` | `FUNC json(body AS String) AS Response` | `200` with `Content-Type: application/json`. |
| `http::withHeader` | `FUNC withHeader(resp AS Response, name AS String, value AS String) AS Response` | Returns a copy with one header set (sugar over `WITH resp { headers := ... }`). |
| `http::bytes` | `FUNC bytes(text AS String) AS List OF Byte` | UTF-8-encode a `String` into the `body` byte type. |

`Content-Length` and the status-line reason phrase are always supplied by
`handleRequest` on emit; a handler-set `Content-Length` is ignored to preserve
framing (same reservation policy as the client, §B.3).

### F.5.4 Reading the request — no accessors, use `collections::`

There are **no** `http`-specific request accessors. `Request`'s fields are public
maps, so handlers read them with the standard `collections::*` functions:

- path param: `collections::get(req.params, "testId")`
- optional-param presence: `collections::hasKey(req.params, "bar")`
- query value: `collections::get(req.query, "q")`
- header (keys are lowercased on parse, §F.2.1):
  `collections::get(req.headers, "content-type")`

This keeps the surface minimal and avoids duplicating `collections` behavior. (The
client's `http::header(Result, …)` therefore stays the only `header` function — no
`Request` overload is added.)

### F.5.5 Static file helpers

| Function | Signature | Behavior |
|----------|-----------|----------|
| `http::respondFile` | `FUNC respondFile(file AS RES File, contentType AS String = "") AS Response` | Reads the open file fully into a `200` response body; closes the file (the `RES` is consumed). When `contentType = ""`, defaults to `application/octet-stream`. The low-level primitive. |
| `http::respondPath` | `FUNC respondPath(req AS Request, root AS String) AS Response` | Resolves `req.params["*"]` (or `req.path`) under `root`, **safely**, opens it, infers `Content-Type` from the extension, and serves it; `404` when the file is absent. The function most handlers call. |

**Path-traversal safety is built in.** `respondPath` canonicalizes the requested
path and confines it to `root` *before* touching `fs::*`; any path that escapes
`root` (`../`, absolute, symlink-out) yields a `403`, never a file read. This is the
one security-critical behavior in the server and is not left to the caller.

`respondFile` takes ownership via `RES File`, so the file is closed by lexical drop
even on an error path. Both helpers **buffer the whole file** into `Response.body` —
acceptable for a single-threaded dev/static server; streaming and large-file
chunking are non-goals (§F.8). Note the single-threaded cost: serving a large file
blocks the entire server until it finishes.

### F.5.6 Secure server — `http::serverSSL`

`http::serverSSL` is the TLS counterpart of `http::server`. It binds a listening
socket and loads a **PEM server certificate** (`certPath`) and **private key**
(`keyPath`), returning a `tls::Listener` (§F.2.5). The accept loop is otherwise
identical — `http::handleRequest` is overloaded on the listener type (§F.5.1), so
the loop body and the route list are unchanged between plaintext and TLS:

```basic
RES s AS tls::Listener = http::serverSSL(8443, "cert.pem", "key.pem")
DO
  http::handleRequest(s, routes)   ' same routes, same handlers
LOOP
```

**New `tls` primitives required.** `tls` is client-connect-only today
(`tls::connect`). `serverSSL` depends on server-side additions to the `tls` package
(Phase S0): a `tls::Listener` resource owning the bound socket + loaded credentials,
`tls::listen(host, port, certPath, keyPath, backlog)`, and
`tls::accept(listener) AS TlsSocket` (TCP accept + **server-side** handshake). The
per-connection `tls::read`/`write`/`close` are the existing client functions, reused
unchanged.

**Platform.** `serverSSL` is supported on **both Linux and macOS** — no platform
gate. Phase S0 provides the `tls` server backend on each target (OpenSSL via
`dlopen` on Linux; Network.framework on macOS).

**Transport split.** Because `Socket` and `TlsSocket` are distinct resources that
cannot share one variable (§B.5), the two `handleRequest` overloads have separate
transport bodies (`net::accept`/… vs `tls::accept`/…), both feeding the shared pure
parse/match/dispatch/emit core — exactly the structure the client uses for its
plaintext vs TLS exchange branches.

## F.6 Implementation additions

Parallels the client's source-package plan (Part C). `net` gains two small pure
helpers; `http`'s shim and `.mfb` gain the server surface.

**Reuse inventory (beyond Part C's):**

| Server need | Reused as | Source |
|-------------|-----------|--------|
| Listen / accept / socket read / write / close | `net::listenTcp`, `net::accept`, `net::read`/`readText`, `net::write`/`writeText`, `net::close` | `src/builtins/net.rs` |
| TLS listen / accept / read / write / close | `tls::listen`/`tls::accept` (new — Phase S0), `tls::read`/`write`/`close` (existing) | `src/builtins/tls.rs` |
| Request line / header parsing, chunk decode | the client's response-parsing helpers, adapted (mirror image) | `http_package.mfb` (Part C) |
| First-class handler values in a `List` | function types + indirect call | `mfbasic.md`; `tests/func_collection_*` |
| `WITH` response edits | functional update | `tests/types-with-update-owned` |
| File open / read / close for static serving | `fs::openFile`/`read`/`close` (`RES File`) | `fs` |

**Genuinely new logic:** the path-pattern matcher (`:param`/`:param?`/`*`, segment
walk), percent-decoding + query parsing (`net`), the **multipart/form-data boundary
parser**, the path-traversal-safe static resolver, and the accept→parse→dispatch→
respond lifecycle with its `404`/`500`/crash-proof error routing.

### Phase S0 — `tls` server-side primitives (prerequisite for `serverSSL`)

`tls` is **client-only** today. `http::serverSSL` needs new server-side support in
the `tls` package (landing in the `tls` shim/codegen, not `http`):

- `tls::Listener` — a new resource owning the bound socket + loaded credentials
  (server certificate + private key).
- `FUNC listen(host AS String, port AS Integer, certPath AS String, keyPath AS String, backlog AS Integer = 128) AS Listener` — bind + load PEM credentials.
- `FUNC accept(listener AS Listener, timeoutMs AS Integer = 0) AS TlsSocket` — TCP
  accept + **server-side** TLS handshake, yielding the existing `tls::TlsSocket`
  (whose `read`/`write`/`close` are reused as-is).

These are **server-side** additions to the two `tls` backends that already exist
(`tls.rs:4-8` — OpenSSL via `dlopen` on Linux, Network.framework on macOS), so
`serverSSL` carries no platform gate. Hard prerequisite: without these primitives,
`http::serverSSL` cannot be implemented.

### Phase S1 — `net` decode helpers

In `src/builtins/net_package.mfb` (the source companion from Phase 1 — create it
there if not yet present):

- `__net_percentDecode(s AS String) AS String` + public `FUNC percentDecode`.
- `__net_parseQuery(s AS String) AS Map OF String TO String` + public
  `FUNC parseQuery`.

Register both in `src/builtins/net.rs` exactly as Phase 1 registers `toUrl`/
`toAddress` (consts, `is_net_call`, `call_param_names`, `call_return_type_name`,
`resolve_call`, `arity`, `implementation_name`).

### Phase S2 — `http` shim additions: `src/builtins/http.rs`

Add to the shim from Phase 2:

- Consts: `SERVER`, `SERVER_SSL`, `HANDLE_REQUEST`, `ROUTE`, `RESPONSE_DEFAULT`,
  `OK`, `STATUS`, `JSON`, `WITH_HEADER`, `BYTES`, `RESPOND_FILE`, `RESPOND_PATH`;
  type names `REQUEST_TYPE`, `REQUEST_PART_TYPE`, `RESPONSE_TYPE`, `ROUTE_TYPE`; the
  `__http_*` targets. (`SERVER`/`SERVER_SSL` are *call* names, not types — `server`
  returns `net::Listener`, `serverSSL` returns `tls::Listener`. No request-accessor
  calls — handlers use `collections::*` on the `Request` maps, §F.5.4.)
- `is_builtin_type` → add `Request | RequestPart | Response | Route` (no server
  type; listeners are the existing `net`/`tls` resources).
- `is_http_call` → add the new names.
- `call_param_names` / `call_return_type_name` / `resolve_call` /
  `expected_arguments` / `arity` for each (function-typed `handler` argument typed
  as `FUNC(Request) AS Response`).
- `default_argument_padding`: `server` (host, backlog), `respondFile`
  (contentType), `route` (none).
- `call_return_type_name`/`resolve_call`: `server` → `net::Listener`; `serverSSL`
  → `tls::Listener` (both existing transport resources — no new `http` resource).
  `handleRequest` is **overloaded by listener type**, resolving both
  `[net::Listener, List OF Route]` and `[tls::Listener, List OF Route]` → `Nothing`.
  `consumes_argument` for `respondFile`'s `RES File`.

### Phase S3 — `http_package.mfb` server implementation

Add to `http_package.mfb` (header already imports `net`, `strings`, `collections`,
`errorCode`; add `IMPORT fs`):

- Export `Request`, `RequestPart`, `Response`, `Route`.
- **Pure (shared with, or adapted from, the client):** request-line/header parser,
  chunk decoder, `__http_matchRoute(routes, path)` (the segment walker producing the
  bound `params`), the multipart parser, percent-decode/query via `net::*`.
- **Lifecycle:** `__http_server` (wraps `net::listenTcp`), `__http_serverSSL`
  (wraps `tls::listen`), and the two `handleRequest` overloads — a TCP body via
  `net::accept`/`read`/`write`/`close` and a TLS body via
  `tls::accept`/`read`/`write`/`close`. Both run accept → read-to-frame → parse →
  match → `TRAP`-wrapped handler call → emit → close over the shared pure core; the
  two transport bodies cannot share one socket variable (`Socket` vs `TlsSocket`,
  §B.5).
- **Constructors/static:** the §F.5.3 response constructors and §F.5.5 static
  helpers (reading the request needs no `http` code — handlers call
  `collections::*` on the `Request` maps, §F.5.4).

Honor the source-package constraints from §C Phase 3 (reserved words not
identifiers; ≤ 8 params per function — the handler signature is a single `Request`;
no field-target assignment, build with constructors / `WITH`, thread parser cursor
state through a small node record; cross-file visibility needs `EXPORT`; escape
`\r\n` in literals).

### Phase S4 — Wire into the compiler

Same touch-points as §C Phase 4 (`src/builtins/mod.rs`, `src/resolver.rs`,
`src/typecheck.rs`, `src/ir.rs`); this only **adds the new calls/types** to the
existing dispatch. Verify the function-typed `handler` argument typechecks and
lowers (the first builtin to take a `FUNC(...)` argument — confirm against the
`collections::sortBy`/`reduce` precedent, which already accept function values).

### Phase S5 — Man pages

`src/man/builtins/http/`: add `server.txt`, `handleRequest.txt`, `route.txt`,
`responseDefault.txt`, `ok.txt`, `status.txt`, `json.txt`, `withHeader.txt`,
`bytes.txt`, `respondFile.txt`, `respondPath.txt`, and a routing/types note in
`package.txt`. (No request-accessor pages — handlers use `collections::*`.) Wire `build.rs` and
`src/man/mod.rs` as §C Phase 5 describes.

### Phase S6 — User documentation

`specifications/standard_package.md`: extend the "Built-in HTTP Package" section
with a **Server** subsection — the `Request`/`RequestPart`/`Response`/`Route`/
`Server` blocks, the §F.5 API tables, the routing rules (§F.3), and the
request-parsing/lifecycle summaries. Add `net::percentDecode`/`net::parseQuery` to
§11. `error_codes.md`: note `http`(server) on `ErrInvalidFormat`,
`ErrInvalidArgument`, `ErrOverflow`, and the `7-707-*` transport rows.

### Phase S7 — Tests (golden)

Mirror `tests/func_http_*`. Split by network dependence.

**Offline (deterministic, the bulk):**

- `func_http_match_*` — drive `__http_matchRoute` over patterns: literal, `:param`,
  `:bar?` present/absent, `/static/*` remainder, trailing-slash normalization,
  order/first-wins, mid-pattern `*`/`?` rejected (`ErrInvalidArgument`).
- `func_http_parserequest_*` — fixed raw requests: `GET` with query, `POST` with
  `Content-Length`, chunked, malformed request line (`400`), oversize (`413`).
- `func_http_multipart_*` — a canned `multipart/form-data` body → `parts` map
  (field + file upload, filename/contentType/body); malformed boundary.
- `func_net_percentdecode_valid` / `func_net_parsequery_valid`.
- `func_http_response_*` — `responseDefault`/`ok`/`status`/`json`/`withHeader`/
  `bytes`; status-line/reason derivation; reserved `Content-Length` ignored.
- `func_http_respondpath_*` — extension→Content-Type; **traversal attempts (`../`,
  absolute) → `403`**; missing file → `404`.

**Networked (gated; on-device, not the default sweep):**

- `http_server_loopback` — a `http::server` on `127.0.0.1` with a small route list;
  drive it with the `http::read` client for `200`/`404`/`500`/path-param/query/
  static round-trips; hermetic, no external host.

## F.7 Worked example

```basic
IMPORT http
IMPORT net
IMPORT collections

' All handlers share FUNC(http::Request) AS http::Response.

FUNC home(req AS http::Request) AS http::Response
  RETURN http::ok("welcome")
END FUNC

FUNC showTest(req AS http::Request) AS http::Response
  RETURN http::ok("test id = " & collections::get(req.params, "testId"))
END FUNC

FUNC showFoo(req AS http::Request) AS http::Response
  ' /foo/:bar?  — bar may be absent
  IF collections::hasKey(req.params, "bar") THEN
    RETURN http::ok("foo bar = " & collections::get(req.params, "bar"))
  END IF
  RETURN http::ok("foo, no bar")
END FUNC

FUNC teapot(req AS http::Request) AS http::Response
  MUT resp AS http::Response = http::responseDefault()   ' 200 / "OK"
  resp = WITH resp { status := 418 }
  resp = WITH resp { body := http::bytes("I'm a teapot") }
  RETURN resp
END FUNC

FUNC serveStatic(req AS http::Request) AS http::Response
  RETURN http::respondPath(req, "./public")               ' traversal-safe
END FUNC

FUNC main() AS Integer
  LET routes AS List OF http::Route = collections::listOf(
    http::route("/",             home),
    http::route("/test/:testId", showTest),
    http::route("/foo/:bar?",    showFoo),
    http::route("/teapot",       teapot),
    http::route("/static/*",     serveStatic)
  )

  RES s AS net::Listener = http::server(8080)
  DO
    http::handleRequest(s, routes)   ' accept 1, strip query, match path,
  LOOP                               ' call handler (err->500), no match->404
END FUNC
```

A request to `GET /test/42?debug=1` matches `/test/:testId` (the `?debug=1` is
stripped before matching), binds `testId = "42"`, makes `debug = "1"` available via
`collections::get(req.query, "debug")`, and returns `200 test id = 42`.

## F.8 Server non-goals for v1

- **Concurrency.** Single-threaded, one request at a time, in the user's loop. No
  threads, no async, no per-connection workers. (A future thread-per-connection
  mode is a clean wrapper once the value-based request/response core is proven.)
- **mTLS / client certificates; SNI multi-cert selection.** `http::serverSSL`
  (§F.5.6) presents a single server certificate/key and works on Linux and macOS;
  mutual-TLS (verifying client certs) and per-hostname certificate selection are out
  of scope.
- **Keep-alive / pipelining.** Each `handleRequest` is one accept-parse-respond-
  close. `Connection: close` semantics.
- **Streaming bodies.** Request bodies are buffered to the size cap; responses
  (incl. static files) are buffered fully into `Response.body`. No chunked
  *response* generation, no large-file streaming.
- **Multi-value headers/query** (last-wins; no `Map OF String TO List`).
- **Mid-pattern `*`/`:param?`** (trailing-only).
- **Compression, HTTP/2+, WebSockets, range requests, ETag/conditional GET.**
- **Configurable size cap** (fixed 64 MiB).
- **Sessions, cookie-parsing helpers, templating, generic middleware chains** —
  each is a sibling package or a later addition, not a server concern. (Middleware,
  without closures, is better expressed as explicit handler composition or built-in
  `handleRequest` policy than as a generic chain.)

These keep the server v1 a small, blocking, batteries-light router honoring the
requested shape: an ordered `List OF Route` of path-pattern → `FUNC(Request) AS
Response`, matched in list order on the path alone, driven by a user-owned
`DO/LOOP` over `http::handleRequest(server, routes)`.
