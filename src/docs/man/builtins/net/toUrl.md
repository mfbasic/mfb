# toUrl

Parse an absolute http or https URL into its components.

## Synopsis

```
net::toUrl(href AS String) AS Url
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required. `toUrl` is
one of the three `net` calls implemented in MFBASIC source rather than as a
native runtime helper; it lowers to the internal `__net_toUrl`.
[[src/builtins/net.rs:implementation_name]]

## Description

`net::toUrl` parses an absolute URL of the shape
`scheme://[user[:pass]@]host[:port]path[?query][#fragment]` into a `Url` value
record. Unlike `Socket` and its siblings, `Url` is an ordinary copyable record,
not a resource handle. [[src/builtins/net_package.mfb:__net_toUrl]]

Parsing splits at the first `://`. The scheme before it is lowercased and must be
`http` or `https`; anything else raises `ErrUnsupported`, and a missing `://`
raises `ErrInvalidFormat`. The authority runs to the first `/`, `?`, or `#`, or
to the end of the string. [[src/builtins/net_package.mfb:__net_authorityEnd]]

Userinfo is optional and is split off at the **last** `@` in the authority, not
the first, as RFC 3986 and the WHATWG URL standard require. That matters for an
authority carrying more than one `@`: `http://a@b@c/` yields username `a@b` and
host `c`, not host `b@c`. Within the userinfo the split is at the *first* colon:
before it is `username`, after it is `password`, both stored exactly as written
with no decoding. Userinfo with no colon is a username only.
[[src/builtins/net_package.mfb:__net_lastIndexOf]]

The host may be a DNS name, an IPv4 literal, or a bracketed IPv6 literal, whose
brackets are stripped so `[::1]` stores host `::1`. After a bracketed literal
only a `:port` may follow — anything else raises `ErrInvalidFormat`, as does an
unterminated bracket. An empty host is rejected. The host is otherwise **not**
validated: a name that is syntactically odd but non-empty is accepted here and
only fails later, at resolution time.
[[src/builtins/net_package.mfb:__net_toUrl]]

The port is optional and defaults to the scheme default — 443 for `https` and 80
for everything else, which given the scheme check means 80 for `http`. An
explicit port must be non-empty, must not carry a leading `+` or `-` (ports are
unsigned, and the shared radix parser would otherwise accept a sign), must parse
as base-10 digits, and must not exceed 65535; each of those raises
`ErrInvalidFormat`. [[src/builtins/net_package.mfb:__net_parsePort]]

What remains is split at the first `#` into a fragment and at the first `?` into a
query, each stored without its leading punctuation. An absent path becomes `"/"`.
No percent-decoding and no other normalization is performed anywhere in `toUrl` —
use `net::percentDecode` for a path component and `net::parseQuery` for the query
string. A universal `toString` on a `Url` renders it back to an href, omitting a
port equal to the scheme default and re-bracketing a host containing a colon.
[[src/builtins/net_package.mfb:__net_urlToString]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `href` | `String` | The absolute URL to parse. Also accepted under the alternate named-argument spellings `value` and `url`, so `net::toUrl(href := s)`, `net::toUrl(value := s)`, and `net::toUrl(url := s)` all bind position 0. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Url` | A value record with `scheme`, `username`, `password`, `host`, `path`, `query`, and `fragment` (all `String`) plus `port` (`Integer`). `scheme` is lowercased, `host` has any IPv6 brackets stripped, `path` is at least `"/"`, and `query` and `fragment` omit their leading `?` and `#`. [[src/builtins/net_package.mfb:__net_toUrl]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | The href has no `://` separator, an empty host, an unterminated bracketed IPv6 literal, trailing characters after a bracketed literal that are not a `:port`, an empty port, a port carrying a sign or a non-digit, or a port above 65535. [[src/builtins/net_package.mfb:__net_parsePort]] |
| `77050007` | `ErrUnsupported` | The scheme is neither `http` nor `https`. [[src/target/shared/code/error_constants.rs:ERR_UNSUPPORTED_CODE]] |
| `77010001` | `ErrOutOfMemory` | An intermediate string slice or the resulting `Url` record could not be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Parse a full URL and read its parts:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("https://api.example.com:8443/v1/items?limit=10#frag")
  io::print(u.host)
  io::print(toString(u.port))
  io::print(u.path)
  io::print(u.query)
  RETURN 0
END FUNC
```

Scheme defaults and round-tripping through `toString`:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("http://example.com")
  io::print(toString(u.port))
  io::print(u.path)
  io::print(toString(u))
  RETURN 0
END FUNC
```

## See also

- `mfb man net percentDecode`
- `mfb man net parseQuery`
- `mfb man net connectTcp`
- `mfb man net lookup`
