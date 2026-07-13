# bug-157 — macOS `tls::write` addresses the Byte payload from COUNT instead of CAPACITY (sends wrong bytes)

Last updated: 2026-07-12
Severity: MEDIUM — silent wrong data over a TLS connection on macOS.
Class: Correctness (CAPACITY-vs-COUNT, same class as commit e7b48c0f).
Status: Open

## Finding

`src/target/shared/code/tls/macos.rs:1429-1437` (the `List OF Byte` branch of
`tls::write`/`writeText`). The payload base is computed as
`ARG1 + HEADER + COUNT*ENTRY` — `%v10` (`COLLECTION_OFFSET_COUNT`) is reused for
both `DLEN` and the `multiply_registers("%v13","%v10","%v12")` at :1433. But a
List's byte payload begins past the **CAPACITY**-sized entry array
(`HEADER + CAPACITY*ENTRY`). When `capacity > count`, `DATA` lands inside the
entry array and `dispatch_data_create(DATA, DLEN=count, …)` copies `count` bytes
from the wrong offset. The OpenSSL sibling does it correctly:
`src/target/shared/code/tls/openssl.rs:1912` loads `COLLECTION_OFFSET_CAPACITY`
for the base multiply. Reads stay in bounds (`count*ENTRY ≤ capacity*ENTRY`), so
this is data corruption, not OOB. Same class as commit e7b48c0f ("write read the
byte payload from CAPACITY, not COUNT") — that fix landed in net/openssl but the
macOS TLS write path was missed.

## Trigger

`tls::write(sock, list)` on macOS where `list` is a `List OF Byte` with
`CAPACITY > COUNT` — any append-built byte list, or `strings::toBytes` output
carrying spare capacity. The bytes sent over TLS differ from `list`'s contents.
(The macOS read-bytes path is unaffected: it builds a fresh collection with
`count == capacity`.)

## Fix

In the byte branch, load `COLLECTION_OFFSET_CAPACITY` into its own register and
use it for the `HEADER + CAPACITY*ENTRY` payload-base multiply, mirroring
`openssl.rs:1912`. Add a runtime test that TLS-writes a capacity>count byte list
and checks the peer receives the exact bytes.

## Prior art

Recurring CAPACITY-vs-COUNT class (commit e7b48c0f, plan-33-B/C d1c4bc19). This
is the remaining macOS-TLS instance.
