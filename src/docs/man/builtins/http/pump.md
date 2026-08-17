# pump

Perform one non-blocking read on an HTTP stream, accumulating the response.

## Synopsis

```
http::pump(stream AS RES http::Stream STATE PendingState)
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

`pump` does one non-blocking read of whatever bytes are available on `stream` and
appends them to `stream.state.raw`. It is internally gated on readiness (it calls
the same probe as `http::ready`), so it never blocks: when no bytes are available
it returns immediately having done nothing. A read that returns zero bytes marks
the stream `state.closed = TRUE` (the peer closed, the `Connection: close`
terminator); a transport failure is captured in `state.err` rather than raised,
so the drive loop stays simple and the error surfaces from `http::finish`.
[[src/codegen/builtins/http/package.mfb:__http_pump]] [[src/codegen/builtins/http/package.mfb:__http_readNet]]

Each call reads at most one 64 KiB chunk, so a large reply is accumulated across
several `pump` calls — the point of the cooperative API. If the accumulated
`state.raw` exceeds the internal 64 MiB response cap, `state.err` is set to the
overflow code and the exchange ends. `pump` is a `SUB`: it advances the stream's
STATE in place and returns nothing. [[src/codegen/builtins/http/package.mfb:__HTTP_MAX_RESPONSE]] [[src/codegen/builtins/http/package.mfb:__http_readTls]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `RES http::Stream STATE PendingState` | The bound stream from `http::startRead`. Passed by reference; `pump` mutates its STATE (`raw`/`closed`/`err`) and neither consumes nor closes it. |

## Return value

`pump` is a `SUB` and returns nothing; its effect is the mutation of
`stream.state`.

## Errors

`pump` raises nothing: a transport failure is recorded in `stream.state.err` (and
reported later by `http::finish`), and a peer close is recorded in
`stream.state.closed`. [[src/codegen/builtins/http/package.mfb:__http_pump]]

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
  io::print(toString(len(s.state.raw)) & " bytes accumulated")
END SUB
```

## See also

- `mfb man http ready`
- `mfb man http done`
- `mfb man http finish`
- `mfb man http startRead`
