# parseQuery

Parse a URL query string into a map of decoded keys and values.

## Synopsis

```
net::parseQuery(s AS String) AS Map OF String TO String
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required. `parseQuery`
is one of the three `net` calls implemented in MFBASIC source rather than as a
native runtime helper; it lowers to the internal `__net_parseQuery`.
[[src/builtins/net.rs:implementation_name]]

## Description

`net::parseQuery` parses an `a=1&b=2` query string into a
`Map OF String TO String`. The leading `?` must already have been stripped by the
caller — `net::toUrl` does exactly that, storing the raw query without it, so
`net::parseQuery(net::toUrl(href).query)` is the intended pairing. An empty input
returns an empty map. [[src/builtins/net_package.mfb:__net_parseQuery]]

The input is split on `&`, and each pair is split at its first `=`. The part
before the `=` is the key and the part after is the value; a bare key with no `=`
at all maps to the empty string, which is how a valueless flag such as `?debug`
appears. An empty pair — produced by `&&`, or by a leading or trailing `&` — is
skipped rather than yielding an empty key. Repeated keys collapse last-wins: the
final occurrence in the string is the one in the map.
[[src/builtins/net_package.mfb:__net_parseQuery]]

Keys and values are both query-decoded: `%XX` escapes become the bytes they name
and a literal `+` becomes a space, which is `application/x-www-form-urlencoded`
semantics. Note that the `+` rule applies to keys as well as values, and that it
is exactly the rule `net::percentDecode` does *not* apply, since a `+` in a path
segment is a literal `+`. [[src/builtins/net_package.mfb:__net_decodeQueryComponent]]

Decoding here is **tolerant**, which is the deliberate difference from
`net::percentDecode`. A component whose escapes are malformed — a truncated `%`,
a non-hexadecimal pair, or bytes that do not form valid UTF-8 — is kept as its
raw undecoded text instead of failing, so `"k=%2"` yields the value `"%2"`. One
bad component therefore never sinks an otherwise valid query, which is what lets
the built-in `http` server route framing errors to a 400 response without letting
soft query-decode failures do the same.
[[src/builtins/net_package.mfb:__net_decodeQueryComponent]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `s` | `String` | The query string, without its leading `?`. Also accepted under the alternate named-argument spellings `query` and `value`, so `net::parseQuery(s := q)`, `net::parseQuery(query := q)`, and `net::parseQuery(value := q)` all bind position 0. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `Map OF String TO String` | One entry per non-empty pair, with keys and values query-decoded and repeated keys resolved last-wins. A bare key maps to the empty string. An empty input yields an empty map. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The split pair list, a decoded component, or the result map could not be allocated. Malformed escapes do *not* raise an error; they fall back to the raw text. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] |

## Examples

Parse a query and read its values:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET q = net::parseQuery("name=a+b&n=42&raw=%2Fx")
  io::print(collections::getOr(q, "name", "?"))
  io::print(collections::getOr(q, "n", "?"))
  io::print(collections::getOr(q, "raw", "?"))
  RETURN 0
END FUNC
```

Parse the query carried by a URL, including a bare key:

```
IMPORT collections
IMPORT net
IMPORT io

FUNC main AS Integer
  LET u AS net::Url = net::toUrl("https://example.com/search?q=a+b&debug")
  LET q = net::parseQuery(u.query)
  io::print(collections::getOr(q, "q", "?"))
  io::print(toString(len(collections::keys(q))))
  RETURN 0
END FUNC
```

## See also

- `mfb man net percentDecode`
- `mfb man net toUrl`
- `mfb man collections getOr`
