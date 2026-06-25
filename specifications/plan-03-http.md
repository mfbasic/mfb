# plan-03 — Built-in HTTP Package (with `net` URL support)

Last updated: 2026-06-25

This document is the **normative definition and implementation plan** for a new
built-in `http` package: a **blocking** HTTP/1.1 client. It is built as a
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
| macOS aarch64 | ✅ via `net` | ⛔ `tls` macOS backend deferred (`tls.rs:6`) |

On macOS, an `https` request fails `ErrUnsupported` (`77050007`) until the macOS
TLS backend lands; plaintext `http` works on both targets. This reuses the
capability gate `tls::connect` already has — `http` adds no platform code.

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
- `http_https_unsupported_macos` — `https` fails `ErrUnsupported` on macOS.

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
| Platform parity | identical | `https` Linux-only until macOS `tls` lands |
| `toString` | structural default | `net::Url` gets a `toString` overload (existing pattern, §A.3) |
| Resources | none | none in the public surface (sockets internal, closed before return) |

## E.2 Errors

No new error codes; reuse `7-705-*` and propagate `7-707-*`:

| Condition | Code |
|-----------|------|
| `net::toUrl` malformed href (no `://`, empty host, bad port) | `ErrInvalidFormat` (`77050003`) |
| `net::toUrl` non-`http(s)` scheme; `https` on macOS (deferred) | `ErrUnsupported` (`77050007`) |
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
- **macOS `https`** until the `tls` macOS backend (Network.framework) ships.

These keep v1 a clean, blocking, batteries-light client while honoring the
requested shape: a `Url` and `toUrl` in `net::` (with `toString` and `toAddress`),
and `http::read`/`write` — each method-named-and-defaulted, each over a
`net::Url`, each returning a `http::Result`.
