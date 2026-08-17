# done

Report whether an HTTP stream's response is complete.

## Synopsis

```
http::done(stream AS RES http::Stream STATE PendingState) AS Boolean
```

## Package

`http`

## Imports

```
IMPORT net
IMPORT http
```

`IMPORT net` is required because the stream's transport variants are `net::Socket`
and `net::TlsSocket`. The `Stream` union and `PendingState` are provided by `http`.

## Description

`done` returns `TRUE` when the exchange has finished and no further `http::pump`
is needed — the drive loop's exit condition. It is `TRUE` when any of three things
holds: a transport error was captured (`state.err <> 0`); the peer closed the
connection (`state.closed`, the `Connection: close` terminator); or the bytes
accumulated so far already form a complete response frame (Content-Length
satisfied, or the final `chunked` chunk seen). The frame check is an early-out, so
a well-framed reply completes before the peer's EOF is observed.
[[src/codegen/builtins/http/package.mfb:__http_done]] [[src/codegen/builtins/http/package.mfb:__http_frameComplete]]

`done` is a pure predicate over `stream.state`: it neither reads the socket nor
mutates STATE. Call it at the top of the drive loop; once it is `TRUE`, call
`http::finish` to obtain the `Response`.

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `RES http::Stream STATE PendingState` | The bound stream from `http::startRead`. Passed by reference; `done` neither consumes nor closes it. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` iff the response is complete (error, peer EOF, or a fully-framed reply); `FALSE` while more pumping is required. [[src/codegen/builtins/http/package.mfb:__http_done]] |

## Errors

`done` raises nothing; a captured transport error is reported by `http::finish`,
not `done`.

## Examples

```
IMPORT net
IMPORT http

SUB main()
  RES s AS http::Stream STATE PendingState = http::startRead(net::toUrl("http://example.com/"))
  WHILE http::done(s) = FALSE
    IF http::ready(s) THEN
      http::pump(s)
    END IF
  END WHILE
END SUB
```

## See also

- `mfb man http finish`
- `mfb man http pump`
- `mfb man http startRead`
