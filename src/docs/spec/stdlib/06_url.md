# URL Model

The `net` package's URL support is pure string work implemented in injected
MFBASIC source; sockets, DNS, and UDP stay native. This topic specifies the
`Url` value record, the `net::toUrl` parser, and the `toString(Url)` renderer —
the parse/render *model*, not the per-function API (that is `./mfb man net`).

The `Url` record and both helpers live in the source companion
`src/builtins/net_package.mfb`; only the nominal type name `Url` is registered
on the Rust side (`URL_TYPE` in `src/builtins/net.rs`), and the universal
`toString` is routed to the renderer in `src/builtins/mod.rs`. [[src/builtins/net_package.mfb:Url]] [[src/builtins/net.rs:URL_TYPE]]

## The `Url` record

`Url` is an `EXPORT TYPE` value record with eight scalar/String fields, in this
declaration order: [[src/builtins/net_package.mfb:Url]]

| Field      | Type      | Meaning |
|------------|-----------|---------|
| `scheme`   | `String`  | `"http"` or `"https"`, always lowercased |
| `username` | `String`  | userinfo before `:`; `""` if no userinfo |
| `password` | `String`  | userinfo after `:`; `""` if absent |
| `host`     | `String`  | registered name or IP literal; IPv6 stored **without** brackets |
| `port`     | `Integer` | explicit port, or the scheme default (80 / 443) |
| `path`     | `String`  | begins with `/`; `"/"` when the href carried no path |
| `query`    | `String`  | raw query, **without** the leading `?`; `""` if none |
| `fragment` | `String`  | raw fragment, **without** the leading `#`; `""` if none |

Unlike `Address`/`Datagram`, `Url` has no Rust-side field table — its layout is
defined solely by the source `TYPE` declaration. The userinfo of a generic URL
is split into the two separate `username`/`password` fields rather than kept as a
single `userinfo` string.

Construction is positional: the parser returns
`Url[scheme, username, password, host, port, path, query, fragment]`. [[src/builtins/net_package.mfb:__net_toUrl]]

## Parsing model (`net::toUrl` → `__net_toUrl`)

`net::toUrl(href)` dispatches to the source helper `__net_toUrl`. The grammar it
accepts is a deliberately small subset of RFC 3986, scheme-required and
authority-required:

```
url        = scheme "://" authority pathpart
authority  = [ userinfo "@" ] hostport
userinfo   = username [ ":" password ]
hostport   = host [ ":" port ]
host       = regname | ipv4 | "[" ipv6 "]"
pathpart   = path [ "?" query ] [ "#" fragment ]
```

All string indexing is by **grapheme** (via `strings::graphemes`/`find`/`mid`),
through the `__net_indexOf` / `__net_slice` wrappers (half-open slices,
`-1`-on-miss). Parsing proceeds: [[src/builtins/net_package.mfb:__net_toUrl]] [[src/builtins/net_package.mfb:__net_indexOf]]

1. **Scheme.** Split at the first `"://"`. Absence fails. The scheme text is
   lowercased (`strings::lower`); only `"http"` and `"https"` are accepted —
   any other scheme fails as unsupported. [[src/builtins/net_package.mfb:__net_toUrl]]
2. **Authority span.** The authority runs from just after `"://"` to the first
   of `/`, `?`, `#`, or end-of-string, computed by `__net_authorityEnd`. The
   remainder is `pathPart`. [[src/builtins/net_package.mfb:__net_authorityEnd]]
3. **Userinfo.** If the authority contains `@`, the text before it is userinfo
   and the text after is `hostport`. Userinfo is split at its first `:` into
   `username`/`password`; with no `:`, the whole userinfo is the `username` and
   `password` stays `""`. No `@` → both stay `""`. [[src/builtins/net_package.mfb:__net_toUrl]]
4. **Host + port.** See IPv6 handling below. After extraction, an empty `host`
   fails. [[src/builtins/net_package.mfb:__net_toUrl]]
5. **Port defaulting.** `port` is initialized to `__net_defaultPort(scheme)` and
   overwritten only when an explicit port text is present. [[src/builtins/net_package.mfb:__net_defaultPort]]
6. **Path / query / fragment.** From `pathPart`: split off `fragment` at the
   first `#`, then split off `query` at the first `?` of what remains; the rest
   is `path`. An empty `path` defaults to `"/"`. The `?`/`#` delimiters are
   stripped; `query`/`fragment` are stored raw (no percent-decoding,
   no `+`-decoding, no key/value parsing). [[src/builtins/net_package.mfb:__net_toUrl]]

### Scheme default ports

`__net_defaultPort` is the single source of truth for default ports, used by
both parse and render: [[src/builtins/net_package.mfb:__net_defaultPort]]

| scheme    | default port |
|-----------|--------------|
| `"https"` | 443 |
| anything else (`"http"`) | 80 |

### IPv6 host handling

The host is parsed two ways depending on a leading `[`: [[src/builtins/net_package.mfb:__net_toUrl]]

- **Bracketed (IPv6 literal).** If `hostport` starts with `[`, the host is the
  text between `[` and the first `]`; a missing `]` fails (unterminated literal).
  After `]`, a `:` introduces the port; any other trailing characters fail. The
  brackets are **stripped** — `host` holds the bare IPv6 address.
- **Unbracketed (reg-name / IPv4).** Otherwise the first `:` splits host from
  port text; with no `:`, the whole `hostport` is the host. (This means a bare
  unbracketed IPv6 address, which contains colons, is mis-parsed — IPv6 must be
  bracketed.)

### Explicit port validation (`__net_parsePort`)

Explicit port text is validated and bounds-checked: empty text, a leading `+`
or `-`, any non-digit content, or a value exceeding 65535 each fail. A leading
sign is rejected up front (ports are unsigned; `toInt`'s signed parse would
otherwise accept one), then the digits are parsed with `toInt(text, 10)` under
an inline `TRAP` — a parse failure re-raises as an invalid-port error. There is
no leading-zero or upper-bound-on-zero special handling beyond `<= 65535`. [[src/builtins/net_package.mfb:__net_parsePort]]

### Parse failures

All parse/validation failures `FAIL error(...)` with one of two codes:
`77050003` (malformed URL: missing `://`, empty/unterminated/garbage host or
port, port out of range) and `77050007` (unsupported scheme). [[src/builtins/net_package.mfb:__net_toUrl]] [[src/builtins/net_package.mfb:__net_parsePort]]

## Rendering model (`toString(Url)` → `__net_urlToString`)

A universal `toString(value)` whose argument is a `Url` is routed to
`__net_urlToString` (the `__`-prefix internalizes the name so it never collides
with the builtin `toString`). [[src/builtins/mod.rs:60]] [[src/builtins/net_package.mfb:__net_urlToString]]

Rendering is the inverse of parsing and reconstructs an absolute href: [[src/builtins/net_package.mfb:__net_urlToString]]

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
  a `:` (`strings::contains`), not by tracking that the input was bracketed.
- **Port elision** drops the port exactly when it equals the scheme default, so
  `http://h:80/` round-trips to `http://h/`.
- No re-encoding of `path`/`query`/`fragment`; they are concatenated verbatim.

## See Also

* ./mfb man net — per-function API for `net::toUrl` and the socket/DNS/UDP surface
* ./mfb spec stdlib http — the HTTP/1.1 client that consumes `Url`
* ./mfb spec unicode strings-model — grapheme indexing used by the parser
* ./mfb spec architecture frontend — how the `net` source package is injected
* ./mfb spec language types — value-record semantics for `Url`
