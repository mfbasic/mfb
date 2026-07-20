# bytes

UTF-8 encode a `String` into the byte list a request or response body holds

## Synopsis

```
http::bytes(text AS String) AS List OF Byte
```

## Package

`http`

## Imports

```
IMPORT http
```

`http` is a built-in package, so `IMPORT http` needs no manifest dependency.
[[src/builtins/http.rs:augmented_project]]

## Description

`http::bytes` encodes `text` as UTF-8 and returns the result as a
`List OF Byte`, which is the type of the `body` field on both `http::Response`
and `http::Request`. It is a direct wrapper over `strings::toBytes`, so the
result is exactly the raw UTF-8 bytes backing the string — one list element per
byte, not per character. [[src/builtins/http_package.mfb:__http_bytes]]

The encoding is unconditional and lossless in both directions: nothing is
escaped, trimmed, length-limited, or inspected, and no header is set or implied.
An empty `String` yields an empty `List OF Byte`. `toString` on a
`List OF Byte` is the inverse, which is how a received `body` is read back as
text. [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_to_bytes]]

This exists for the case where you are editing a body directly — typically with
`WITH` on an existing response — because the field is bytes and a `String`
cannot be assigned to it. When you are *constructing* a response you do not need
it: `http::ok`, `http::status`, and `http::json` all take a `String` body and
encode it for you. [[src/builtins/http_package.mfb:__http_responseWith]]

`http::bytes` is a pure function. It reads no state, performs no I/O, and
mutates nothing; the same input always produces the same output.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `text` | `String` | The text to encode. Any string is accepted, including the empty string. [[src/builtins/http.rs:call_param_names]] |

## Return value

| Type | Description |
| --- | --- |
| `List OF Byte` | The UTF-8 bytes of `text`, one element per byte. An empty `String` yields an empty list. [[src/builtins/http.rs:call_return_type_name]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77010001` | `ErrOutOfMemory` | The `List OF Byte` holding the encoded bytes cannot be allocated. [[src/target/shared/code/error_constants.rs:ERR_OUT_OF_MEMORY_CODE]] [[src/target/shared/code/builder_strings_builtins.rs:lower_strings_to_bytes]] |

## Examples

Replace the body of an existing response:

```
IMPORT http

FUNC teapot(req AS http::Request) AS http::Response
  LET base AS http::Response = http::status(418, "")
  RETURN WITH base { body := http::bytes("I'm a teapot") }
END FUNC
```

Round-trip a body back to text:

```
IMPORT http
IMPORT io

SUB main
  LET body AS List OF Byte = http::bytes("hello")
  io::print(toString(len(body)))
  io::print(toString(body))
END SUB
```

## See also

- `mfb man http ok`
- `mfb man http status`
- `mfb man http json`
- `mfb man http responseDefault`
- `mfb man http withHeader`
