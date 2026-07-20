# percentDecode

Percent-decode a URL path component.

## Synopsis

```
net::percentDecode(s AS String) AS String
```

## Package

`net`

## Imports

```
IMPORT net
```

`net` is a built-in package, so no manifest dependency is required.
`percentDecode` is one of the three `net` calls implemented in MFBASIC source
rather than as a native runtime helper; it lowers to the internal
`__net_percentDecode`. [[src/builtins/net.rs:implementation_name]]

## Description

`net::percentDecode` decodes the `%XX` escapes in a request-target path component
and returns the result as a `String`. It walks `s` one grapheme at a time: a `%`
consumes the next two characters and contributes the byte they name in
hexadecimal, and every other grapheme contributes its own UTF-8 bytes unchanged.
The accumulated bytes are then validated as UTF-8, so the result is always
well-formed text. [[src/builtins/net_package.mfb:__net_percentDecodeImpl]]

Unlike query decoding, a literal `+` is left as a `+`. A `+` in a path segment is
an ordinary character, not a space; only `application/x-www-form-urlencoded`
query data gives it that meaning. Use `net::parseQuery` for a query string, whose
keys and values do decode `+` to a space.
[[src/builtins/net_package.mfb:__net_percentDecode]]

Decoding here is **strict**, which is the other way it differs from
`net::parseQuery`. A `%` with fewer than two characters after it, a `%` followed
by something that is not a hexadecimal pair, or a decoded byte sequence that is
not valid UTF-8 all raise `ErrInvalidFormat`. The implementation routes every
failure inside the decode — including the UTF-8 validation failure, which the
inline-trap analysis cannot see — through a single function-level trap, so
`ErrInvalidFormat` is the only error this function raises: nothing else, not even
an allocation failure, escapes with a different code.
[[src/builtins/net_package.mfb:__net_percentDecodeImpl]]

This is the decoder the built-in `http` server applies to a request path.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `s` | `String` | The percent-encoded path component to decode. Also accepted under the alternate named-argument spellings `text` and `value`, so `net::percentDecode(s := p)`, `net::percentDecode(text := p)`, and `net::percentDecode(value := p)` all bind position 0. [[src/builtins/net.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `String` | The decoded component: every `%XX` replaced by the byte it names, every other grapheme carried through unchanged, and the whole validated as UTF-8. An empty input yields an empty string. [[src/builtins/net.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | A `%` escape is truncated or is not followed by a hexadecimal pair, or the decoded bytes are not valid UTF-8. Every failure inside the decode is reported under this one code. [[src/builtins/net_package.mfb:__net_percentDecodeImpl]] |

## Examples

Decode an escaped path:

```
IMPORT net
IMPORT io

FUNC main AS Integer
  io::print(net::percentDecode("/a%20b/c"))
  RETURN 0
END FUNC
```

Report the error code for a malformed escape:

```
IMPORT net

FUNC decodeOrCode(s AS String) AS String
  RETURN net::percentDecode(s)
  TRAP(e)
    RETURN toString(e.code)
  END TRAP
END FUNC

SUB main()
  ' Returns the decoded text, or the error code — 77050003 (ErrInvalidFormat) for a
  ' truncated or non-hex escape, or a non-UTF-8 result.
END SUB
```

## See also

- `mfb man net parseQuery`
- `mfb man net toUrl`
- `mfb man encoding percentDecode`
