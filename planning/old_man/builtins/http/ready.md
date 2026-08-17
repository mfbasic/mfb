# ready

Report whether an HTTP stream has data available to read without blocking.

## Synopsis

```
http::ready(stream AS RES http::Stream STATE PendingState) AS Boolean
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

`ready` returns `TRUE` when a non-blocking read of `stream` would return bytes or
observe end-of-stream right now, and `FALSE` when it would have to wait. It is a
pure readiness probe with a zero timeout — it never blocks and never consumes
bytes — layered on the scalar `net::poll`/`tls::poll` of the active transport
variant. Use it to gate `http::pump` so a cooperative drive loop only reads when
progress is possible and otherwise does the caller's own work.
[[src/codegen/builtins/http/package.mfb:__http_ready]] [[src/codegen/builtins/net/func_poll.rs:register]]

`ready` does not itself advance the exchange or change `stream`'s STATE; it only
reports readiness. A closed peer reads as ready (the terminating zero-byte read is
available), so a loop gated on `ready` still reaches `http::done`.
[[src/codegen/builtins/tls/func_poll.rs:poll]]

## Parameters

| Parameter | Type | Description |
| --- | --- | --- |
| `stream` | `RES http::Stream STATE PendingState` | The bound stream from `http::startRead`. Passed by reference; `ready` neither consumes nor closes it. |

## Return value

| Type | Description |
| --- | --- |
| `Boolean` | `TRUE` iff a non-blocking read would return bytes or EOF now; `FALSE` if it would block. [[src/codegen/builtins/http/package.mfb:__http_ready]] |

## Errors

`ready` raises nothing of its own; a poll failure propagates unchanged from the
underlying `net`/`tls` call. [[src/codegen/builtins/http/package.mfb:__http_ready]]

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

- `mfb man http pump`
- `mfb man http startRead`
- `mfb man net poll`
