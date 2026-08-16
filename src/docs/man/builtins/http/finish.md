# finish

Parse a completed HTTP stream's accumulated bytes into a `Response`.

## Synopsis

```
http::finish(stream AS RES http::Stream STATE PendingState) AS Response
```

## Package

`http`

## Imports

```
IMPORT net
IMPORT http
```

`IMPORT net` is required because the stream's transport variants are `net::Socket`
and `net::TlsSocket`. The `Stream` union, `PendingState`, and `Response` are
provided by `http`.

## Description

`finish` turns the bytes accumulated in `stream.state.raw` into an
`http::Response`. Call it once `http::done` reports the exchange complete. If a
transport failure was captured during the drive (`state.err <> 0`), `finish`
`FAIL`s with that error; otherwise it parses the accumulated bytes with the same
parser the blocking `http::read` uses — status line, header block (field names
lowercased, duplicates last-wins), and body (de-chunked when the reply was
`chunked`). [[src/builtins/http_package.mfb:__http_finish]] [[src/builtins/http_package.mfb:__http_parseResponse]]

`finish` does not close the stream: the handle stays bound and its socket is
closed exactly once when it leaves scope. The returned `Response` is a plain,
copyable value record — `status`, `reason`, `httpVersion`, `headers`, `body`, and
`ok` (`TRUE` only for a 2xx status) — identical to what a blocking `http::read`
over the same URL would return. Redirects are not followed; a 3xx reply is
returned as-is with `ok` `FALSE`. [[src/builtins/http_package.mfb:Response]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `RES http::Stream STATE PendingState` | The completed stream from `http::startRead` (after `http::done` is `TRUE`). Passed by reference; `finish` neither consumes nor closes it. |

## Return value

| Type | Description |
| --- | --- |
| `Response` | The parsed reply: `status`, `reason`, `httpVersion`, `headers` (lowercased field names), `body` (raw bytes, de-chunked when `chunked`), and `ok`. [[src/builtins/http_package.mfb:__http_parseResponse]] |

## Errors

| Code | Name | Raised when |
| --- | --- | --- |
| `77050003` | `ErrInvalidFormat` | The accumulated response's status line, header block, or `chunked` framing is malformed. [[src/codegen/builtins/errorcode/mod.rs:ErrInvalidFormat]] [[src/builtins/http_package.mfb:__http_parseStatusLine]] |
| `77050010` | `ErrOverflow` | A captured overflow: the accumulated response exceeded the internal 64 MiB size cap during pumping. [[src/codegen/builtins/errorcode/mod.rs:ErrOverflow]] [[src/builtins/http_package.mfb:__HTTP_MAX_RESPONSE]] |

Any transport failure captured during the drive (for example `ErrTimeout`,
`ErrNetworkFailed`, or `ErrTlsFailed`) is re-raised by `finish` with the captured
code. [[src/builtins/http_package.mfb:__http_finish]]

## Examples

```
IMPORT net
IMPORT http
IMPORT io

SUB main()
  RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
  END WHILE
  LET r AS http::Response = http::finish(s)
  io::print(toString(r.status) & " " & r.reason)
END SUB
```

## See also

- `mfb man http done`
- `mfb man http startRead`
- `mfb man http read`
- `mfb man http types`
