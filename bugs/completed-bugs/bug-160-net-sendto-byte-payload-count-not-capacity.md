# bug-160 — `net.sendTo`/`sendTextTo` reads the `List OF Byte` payload from COUNT instead of CAPACITY (corrupt datagrams)

Last updated: 2026-07-12
Severity: HIGH — normal-case UDP datagram corruption (any append-built byte list).
Class: Correctness (CAPACITY-vs-COUNT, same class as bug-157 / commit e7b48c0f).
Status: FIXED
Regression Test: `tests/rt-behavior/net/bug160_sendto_capacity_gt_count` (an
append-built, capacity>count `List OF Byte` is `sendTo`'d to a local UDP socket;
the peer receives the exact bytes `[65,66,67]`). Fix loads
`COLLECTION_OFFSET_CAPACITY` for the data-region multiply, mirroring `net.write`.

## Finding

`src/target/shared/code/net/io.rs:1550-1558` (the `List OF Byte` path of
`net.sendTo`/`sendTextTo`). The data pointer is computed as
`collection + HEADER + count*ENTRY_SIZE`
(`multiply_registers("%v13", "%v10"=COUNT, ENTRY_SIZE)` at :1553-1556). The
inline byte data region begins past the **CAPACITY**-sized entry array
(`collection + HEADER + capacity*ENTRY_SIZE`). When `capacity > count`, the
computed pointer lands inside the entry-descriptor array, so `sendto` transmits
descriptor bytes instead of the real payload — a corrupted datagram (reads stay
in-bounds, so corruption not OOB). `net.write` does this correctly
(`net/io.rs:563-577`, using `COLLECTION_OFFSET_CAPACITY`, with a comment warning
"Using count instead mis-addresses an append-built list that carries spare
capacity"). The stale comment at `net/io.rs:1548-1549` is itself wrong.

## Trigger

`net.sendTo(sock, addr, bytes)` / `net.sendTextTo` where `bytes` is a
`List OF Byte` with `capacity > count` — the normal case for any list built via
`append` (which reserves headroom) or `strings::toBytes`. The peer receives
wrong bytes.

## Fix

Load `COLLECTION_OFFSET_CAPACITY` (like `net/io.rs:571`) instead of COUNT
(`%v10`) for the data-region multiply in the sendTo byte path; fix the stale
comment. Add a runtime test that `sendTo`s a capacity>count byte list to a local
UDP socket and asserts the received bytes.

## Prior art

Same recurring CAPACITY-vs-COUNT class as bug-157 (macOS TLS write), commit
e7b48c0f (net/tls write), plan-33-B/C. sendTo was missed by those fixes.
