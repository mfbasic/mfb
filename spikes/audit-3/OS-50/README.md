# OS-50 spike — remote peer-controlled memory disclosure via `tcp::write(sock, f(...))`

audit-3 OS-50 (`planning/audit-3-os-runtime.md`), bug-497. **CRITICAL.**

`tcp::write` / `tls::write` / `udp::send` pick the `List OF Byte` lowering
whenever the payload's static type is unknown — which it is for *any* user
function call (`static_type_name` answers `None`). A `String` returned by a call
is then read as a collection block: the write length is the string's first 8
bytes, the source is 40 bytes past the header. The remote peer supplies those
first 8 bytes, so the peer chooses how much process memory is sent back.

## Run

```
mfb build spikes/audit-3/OS-50
./spikes/audit-3/OS-50/build/mfb_project.out &      # listens on 127.0.0.1:18082
python3 spikes/audit-3/OS-50/peer.py
```

`peer.py` sends 22 bytes whose first 8 encode 1024 little-endian.

## Observed (defect present)

```
SENT 22 bytes; PEER GOT 1024 bytes
# leak.bin contains live program strings ("echoing 22 chars") and arena/heap bytes
```

Setting the first 8 bytes to 1_000_000 returns 65_536 bytes before hitting an
unmapped page. Every MFBASIC HTTP server built on `http::handleRequest` is on
this path (it writes `__http_serializeHead(resp)`, a call node) — and because the
HTTP head starts with `HTTP/1.1` (first 8 bytes ≈ 3.5e18) the write fails and the
server answers nothing, which is the symptom filed as open bug-476.

## Expected (after fix)

The peer receives exactly the 22 bytes echoed. The lowering must select the text
form for a `String` payload (resolving the callee's declared return type when
`static_type_name` is `None`), and the byte sink must reject a non-collection
block rather than read a length out of payload bytes.
