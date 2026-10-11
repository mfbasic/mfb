# URL Model

The `net` package's URL support is pure string work implemented in injected
MFBASIC source; sockets, DNS, and UDP stay native. This topic specifies the
`Url` value record, the `net::toUrl` parser, and the `toString(Url)` renderer —
the parse/render *model*, not the per-function API (that is `./mfb man net`).

The `Url` record and both helpers live in the source companion
injected MFBASIC source; only the nominal type name `Url` is registered natively,
and the universal `toString` is routed to the record renderer. [[src/codegen/builtins/net/mod.rs:Url]] [[src/codegen/builtins/net/mod.rs:URL_TYPE]]

## The `Url` record

`Url` is an `EXPORT TYPE` value record with eight scalar/String fields, in this
declaration order: [[src/codegen/builtins/net/mod.rs:Url]]

| Field      | Type      | Meaning |
|------------|-----------|---------|
| `scheme`   | `String`  | `"http"` or `"https"`, always lowercased |
| `username` | `String`  | userinfo before `:`; `""` if no userinfo |
| `password` | `String`  | userinfo after `:`; `""` if absent |
| `host`     | `String`  | validated registered name (as written), canonical dotted-decimal IPv4 address, or IPv6 address stored **without** brackets |
| `port`     | `Integer` | explicit port, or the scheme default (80 / 443) |
| `path`     | `String`  | begins with `/`; `"/"` when the href carried no path; never holds a `\` |
| `query`    | `String`  | raw query, **without** the leading `?`; `""` if none |
| `fragment` | `String`  | raw fragment, **without** the leading `#`; `""` if none |

Unlike `Address`/`Datagram`, `Url` has no Rust-side field table — its layout is
defined solely by the source `TYPE` declaration. The userinfo of a generic URL
is split into the two separate `username`/`password` fields rather than kept as a
single `userinfo` string.

Construction is positional: the parser returns
`Url[scheme, username, password, host, port, path, query, fragment]`. [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]

## Parsing model (`net::toUrl` → `__net_toUrl`)

`net::toUrl(href)` dispatches to the source helper `__net_toUrl`. The grammar it
accepts is a deliberately small subset of RFC 3986, scheme-required and
authority-required. For the URLs it accepts, the host and the authority/path
split follow the WHATWG URL standard (see Conformance below):

```
url        = scheme "://" authority pathpart
authority  = [ userinfo "@" ] hostport
userinfo   = username [ ":" password ]
hostport   = host [ ":" port ]
host       = regname | ipv4 | "[" ipv6 "]"
pathpart   = path [ "?" query ] [ "#" fragment ]
```

The component split indexes the string by **Unicode scalar** (via
`strings::find`/`strings::mid`, with `strings::contains` guarding a miss),
through half-open-slice, `-1`-on-miss wrappers; the host and port validators
walk UTF-8 **bytes** (`strings::toBytes`). Parsing proceeds: [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]] [[src/codegen/builtins/net/helper_index_of.rs:__net_indexOf]]

1. **Scheme.** Split at the first `"://"`. Absence fails. The scheme text is
   lowercased (`strings::lower`); only `"http"` and `"https"` are accepted —
   any other scheme fails as unsupported. [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]
2. **Authority span.** The authority runs from just after `"://"` to the first
   of `/`, `\`, `?`, `#`, or end-of-string, computed by `__net_authorityEnd`. The
   remainder is `pathPart`. `\` is a terminator because an http(s) URL reads it
   as `/`: `http://good.com\@evil.com/` has authority `good.com`, so the `@`
   after it is path text and never a userinfo delimiter. [[src/codegen/builtins/net/helper_authority_end.rs:__net_authorityEnd]]
3. **Userinfo.** If the authority contains `@`, the text before the **last**
   `@` is userinfo and the text after is `hostport`. Userinfo is split at its first `:` into
   `username`/`password`; with no `:`, the whole userinfo is the `username` and
   `password` stays `""`. No `@` → both stay `""`. [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]
4. **Host + port.** The host text is extracted (see Host extraction below); an
   empty `host` fails; the text is then validated and replaced by the value to
   store by `__net_host` (see Host validation below). [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]
5. **Port defaulting.** `port` is initialized to `__net_defaultPort(scheme)` and
   overwritten only when an explicit port text is present. [[src/codegen/builtins/net/helper_default_port.rs:__net_defaultPort]]
6. **Path / query / fragment.** From `pathPart`: split off `fragment` at the
   first `#`, then split off `query` at the first `?` of what remains; the rest
   is `path`. An empty `path` defaults to `"/"`, and every `\` in `path` is
   then replaced by `/`. The `?`/`#` delimiters are stripped; `query`/`fragment`
   are stored raw (no percent-decoding, no `+`-decoding, no key/value parsing,
   and a `\` in either is kept). [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]

### Scheme default ports

`__net_defaultPort` is the single source of truth for default ports, used by
both parse and render: [[src/codegen/builtins/net/helper_default_port.rs:__net_defaultPort]]

| scheme    | default port |
|-----------|--------------|
| `"https"` | 443 |
| anything else (`"http"`) | 80 |

### Host extraction

The host text is taken two ways depending on a leading `[`: [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]]

- **Bracketed (IPv6 literal).** If `hostport` starts with `[`, the host is the
  text between `[` and the first `]`; a missing `]` fails (unterminated literal).
  After `]`, a `:` introduces the port; any other trailing characters fail. The
  brackets are **stripped** — `host` holds the bare IPv6 address.
- **Unbracketed (reg-name / IPv4).** Otherwise the first `:` splits host from
  port text; with no `:`, the whole `hostport` is the host. (This means a bare
  unbracketed IPv6 address, which contains colons, is mis-parsed — IPv6 must be
  bracketed.)

### Host validation (`__net_host`)

`__net_host(text, bracketed)` either fails or returns the `host` to store. Every
failure is `77050003`. [[src/codegen/builtins/net/helper_host.rs:__net_host]]

**Bracketed text** must be an IPv6 address by the WHATWG IPv6 parser, checked by
`__net_isIpv6`: at most eight colon-separated groups of one to four hex digits;
at most one `::`, and exactly eight groups without one; a leading or trailing
single `:` fails; the last two groups may instead be a dotted-decimal IPv4
address of exactly four parts, each `0`..`255` with no leading zero, and only
when at most six groups precede it. A zone id (`%25eth0`) and an IPvFuture
literal (`v1.fe`) fail (message `invalid IPv6 literal`); `[]` fails before this
check, as an empty host. A valid address is stored **as written** — it is not
compressed, lowercased, or converted from the embedded-IPv4 form. [[src/codegen/builtins/net/helper_is_ipv6.rs:__net_isIpv6]]

**Any other host** is checked in three steps:

1. **Percent-decode.** If the text contains `%`, it is decoded by
   `__net_percentDecodeImpl`; a truncated or non-hex escape, or a decoding that
   is not valid UTF-8, fails (`invalid percent-escape in host`). The decoded
   form is used only for steps 2 and 3.
2. **Forbidden bytes.** A decoded byte `<= 0x20`, `0x7F`, or one of
   `# % / : < > ? @ [ \ ] ^ |` fails (`forbidden character in host`). This is
   the WHATWG forbidden-domain-code-point set. Tab, LF and CR are in it: no
   input cleanup is performed, so a host written with one fails. Bytes
   `>= 0x80` pass.
3. **IPv4.** `__net_ipv4Host` applies the WHATWG ends-in-a-number check and
   IPv4 parser to the decoded bytes, after ignoring one trailing `.`. The host
   *ends in a number* when its last dot-separated part is non-empty and is
   either all ASCII digits or a valid IPv4 number. A host that does not is a
   registered name and is stored **as written** (escapes and case kept). A host
   that does must parse: one to four parts, each a valid IPv4 number, every
   part but the last `<= 255`, and the last `< 256^(5 - parts)`; otherwise it
   fails (`invalid IPv4 address`). The stored host is the canonical
   dotted-decimal address. [[src/codegen/builtins/net/helper_ipv4_host.rs:__net_ipv4Host]]

An **IPv4 number** (`__net_ipv4Number`) is: with a `0x`/`0X` prefix,
hexadecimal; else with a leading `0` and at least two characters, octal; else
decimal. A prefix with nothing after it is `0`; an empty part or a character
that is not a digit of the radix is not a number. The accumulator saturates at
`4294967296`, so a digit run of any length cannot overflow it. [[src/codegen/builtins/net/helper_ipv4_number.rs:__net_ipv4Number]]

| href host | stored `host` |
|-----------|---------------|
| `0x7f.1`, `2130706433`, `127.1`, `0177.0.0.1`, `017700000001`, `127.0.0.1.` | `127.0.0.1` |
| `127.0.0.010` | `127.0.0.8` |
| `%30x7f.1` | `127.0.0.1` |
| `1.2.3` | `1.2.0.3` |
| `256.0.0.1`, `1.2.3.09`, `09`, `foo.09`, `foo.0x4`, `1.2.3.4.5`, `1..2`, `4294967296` | fails |
| `1.2.3.4.example`, `Example.COM`, `%41.com` | unchanged |

Because the canonical address is what is stored, it is also what
`http::read`/`http::write` connect to and send in `Host:`, and what
`toString(Url)` renders. [[src/codegen/builtins/http/helper_start_exchange.rs:__http_startExchange]] [[src/codegen/builtins/http/helper_host_header.rs:__http_hostHeader]]

**Not implemented: IDNA.** No UTS #46 mapping or validation is performed. A host
with a non-ASCII byte, raw or percent-encoded, is stored as written, and an
`xn--` label is not checked to be valid Punycode. The one exception follows from
step 3: a non-ASCII host whose last part is a number fails, because its other
parts are not IPv4 numbers (WHATWG would first map, for example, fullwidth
digits to ASCII).

### Explicit port validation (`__net_parsePort`)

Explicit port text is validated and bounds-checked: empty text, any byte that is
not an ASCII digit (so also a leading `+` or `-`), or a value exceeding 65535
each fail. The bytes are checked first (`non-digit in port`); the digits are then
parsed with `toInt(text, 10)` under an inline `TRAP`. An all-digit text can fail
that parse only by overflowing `Integer`, so the trap re-raises as
`port out of range`, the same message as a value above 65535. There is no
leading-zero special handling. [[src/codegen/builtins/net/helper_parse_port.rs:__net_parsePort]]

### Parse failures

All parse/validation failures `FAIL error(...)` with one of two codes:
`77050003` (malformed URL: missing `://`, empty/unterminated/garbage host or
port, port out of range) and `77050007` (unsupported scheme). [[src/codegen/builtins/net/func_to_url.rs:__net_toUrl]] [[src/codegen/builtins/net/helper_parse_port.rs:__net_parsePort]]

The messages, all prefixed `invalid URL: `, are: `missing scheme separator`,
`unterminated IPv6 literal`, `trailing characters after IPv6 literal`,
`empty host`, `invalid IPv6 literal`, `invalid percent-escape in host`,
`forbidden character in host`, `invalid IPv4 address`, `empty port`,
`non-digit in port`, `port out of range`; `77050007` carries
`unsupported URL scheme`. [[src/codegen/builtins/net/helper_host.rs:__net_host]]

### Conformance with the WHATWG URL standard

For an `http`/`https` href, `toUrl` is checked against Node's WHATWG `URL` by
`tools/oracles/url/run.sh <mfb> parse`. It agrees on which ASCII hosts are
valid (an `xn--` label excepted, see IDNA), on the canonical form of an IPv4
host, and on `\` acting as `/` in the authority and the path. It differs in
these ways. Each row but IDNA is a declared class-b group of that oracle; the
IDNA cases are reported by it as class-a divergences:

| Difference | Behaviour here | WHATWG |
|------------|----------------|--------|
| Schemes | only `http` and `https`; any other fails `77050007` | parses any scheme |
| `://` | required; `http:h`, `http:/h` and `http:\\h` fail | optional for http(s) |
| Slashes before the host | `http:///h` and `http://\h` fail as an empty host | skipped |
| Input cleanup | none: a tab, LF or CR anywhere, or a leading/trailing C0 control or space, is kept in its component, and fails when that component is the scheme, the host or the port | tab/LF/CR removed everywhere, ends trimmed |
| Normalization | none beyond the IPv4 host and the path's `\`: host case, percent-escapes, IPv6 spelling, and `.`/`..` path segments are kept; nothing is percent-encoded | lowercases the host, percent-decodes it, compresses IPv6, removes dot segments, percent-encodes each component |
| IDNA | not implemented (see Host validation) | UTS #46 `domain to ASCII` |
| Empty `?` / `#` | an empty query or fragment is not distinguished from an absent one, so `toString` drops a trailing `?` or `#` | kept |

## Rendering model (`toString(Url)` → `__net_urlToString`)

A universal `toString(value)` whose argument is a `Url` is routed to
`__net_urlToString` (the `__`-prefix internalizes the name so it never collides
with the builtin `toString`). [[src/codegen/builtins/mod.rs:73]] [[src/codegen/builtins/net/mod.rs:__net_urlToString]]

Rendering is the inverse of parsing and reconstructs an absolute href: [[src/codegen/builtins/net/mod.rs:__net_urlToString]]

```
out = scheme "://"
      [ username [ ":" password ] "@" ]      ; only if username|password nonempty
      host-or-[host]                          ; brackets re-added iff host contains ":"
      [ ":" port ]                            ; only if port != defaultPort(scheme)
      path
      [ "?" query ]                           ; only if query nonempty
      [ "#" fragment ]                        ; only if fragment nonempty
```

Round-trip notes and asymmetries:

- **Userinfo** is emitted when either `username` or `password` is non-empty; the
  `:password` segment is appended only when `password` is non-empty. (A
  password-only `Url` renders as `:password@`.)
- **IPv6 re-bracketing** is heuristic: brackets are re-added when `host` contains
  a `:` (`strings::contains`), not by tracking that the input was bracketed. For
  a `Url` produced by `toUrl` the two coincide: host validation forbids `:` in
  an unbracketed host and every valid IPv6 address contains one.
- **Port elision** drops the port exactly when it equals the scheme default, so
  `http://h:80/` round-trips to `http://h/`.
- No re-encoding of `path`/`query`/`fragment`; they are concatenated verbatim.

## Component Decoding

Two decoders support the `http` server's request parsing (they are also useful
standalone):

| Function | Signature |
| --- | --- |
| `net::percentDecode` | `percentDecode(s AS String) AS String` |
| `net::parseQuery` | `parseQuery(s AS String) AS Map OF String TO String` |

- **`net::percentDecode`** decodes `%XX` escapes in a request-target path
  component and validates the result as UTF-8. `+` is left **literal** (a path
  `+` is not a space). A truncated/invalid escape or non-UTF-8 result fails
  `ErrInvalidFormat` (`77050003`). It is implemented with only
  `strings`/`collections` (byte accumulation + `toString` UTF-8 validation), so
  `net` carries no extra package dependency.
- **`net::parseQuery`** parses an `a=1&b=2` query string (the part after the
  first `?`, already stripped) into a `Map OF String TO String`. It splits on
  `&`, then each pair on the first `=`, and query-decodes keys and values —
  `%XX` plus `+` → space (`application/x-www-form-urlencoded` semantics). A bare
  key maps to `""`, an empty pair is skipped, and repeated keys collapse
  last-wins. Decoding is **tolerant**: a component with a malformed escape is
  kept as its raw text rather than failing the whole parse.

[[src/codegen/builtins/net/func_percent_decode.rs:__net_percentDecode]] [[src/codegen/builtins/net/func_parse_query.rs:__net_parseQuery]]

## See Also

* ./mfb man net — per-function API for `net::toUrl` and the socket/DNS/UDP surface
* ./mfb spec stdlib http — the HTTP/1.1 client that consumes `Url`
* ./mfb spec unicode strings-model — grapheme indexing used by the parser
* ./mfb spec architecture frontend — how the `net` source package is injected
* ./mfb spec language types — value-record semantics for `Url`
