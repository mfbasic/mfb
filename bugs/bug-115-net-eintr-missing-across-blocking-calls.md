# bug-115 — net blocking calls lack EINTR retry → signal interrupt maps to wrong hard errors

**Status:** OPEN. Filed 2026-07-11 (goal-02 review, G7).
**Severity:** LOW (latent) — window is small on current targets (installed
handlers terminate), but the policy diverges from the fs/io side.
**Class:** correctness.

## Finding

`src/target/shared/code/net/io.rs:302-306/423-442` (read →
ERR_CONNECTION_CLOSED), :563-588 (write → ERR_CONNECTION_CLOSED), :59-75
(accept → ERR_NETWORK_FAILED); net/poll.rs:67-87 (poll → ERR_RESOURCE_CLOSED);
net/mod.rs:545-547 (connect's bounding poll → ERR_NETWORK_FAILED).

bug-62 gave the fs/io read/write loops an EINTR retry; none of the net helpers
have one. `poll` in particular is never auto-restarted even under SA_RESTART,
so any handled signal landing during `net.poll` or a bounded connect yields a
spurious resource-closed/network-failed error.

Mitigation in practice: the only handlers the runtime installs (SIGINT/SIGTERM)
terminate the process, so the window is small — hence LOW/latent, but the
policy diverges from the fs/io side of the same runtime.

## Trigger

Program with a LINKed library (or future runtime feature) that installs a
benign handler; signal arrives during `net::poll(sock, 5000)` → false
ERR_RESOURCE_CLOSED.

## Fix

Wrap the net blocking syscalls (read/write/accept/poll/connect-poll) in the
same EINTR-retry idiom `emit_eintr_retry_or_error` gives the fs/io side.

## Prior art

bug-97/bug-62 cover io_helpers only; audit-1-fs-net-thread.md's "Checked and
OK" list doesn't address EINTR.
