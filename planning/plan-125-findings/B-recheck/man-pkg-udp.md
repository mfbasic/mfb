### 1. The package never says it is IPv4-only
UNIT:      man-pkg:udp
PAGE:      package-wide (overview, bind, send)
CATEGORY:  coverage
CLAIM:     bind's `host`: "The local interface to bind. "0.0.0.0" or "" binds every interface; "127.0.0.1" receives only local datagrams." — no udp page mentions address family at all.
VERDICT:   missing
EVIDENCE:  `grep -n "emit_hints" src/codegen/builtins/udp/gen_io.rs` (bind at :74, send at :739) → `src/codegen/os/socket/shared/mod.rs:242-244` `HINTS_FAMILY_WORD` is `AF_INET (2) << 32`, so both `getaddrinfo` calls are pinned to IPv4. `mfb man net lookup` states "Only IPv4 results are returned", but `mfb man udp --all | grep -i ipv` prints nothing. A developer who binds `"::1"` or `"::"` will get `ErrAddressInvalid` with no page explaining why. Confirming the raised error needs a bind: NEEDS-NETWORK-PROBE (`udp::bind("::1", 0)`).
SUGGESTED: In the overview, next to "Endpoints are net::Address values": "Only IPv4 is supported: bind, send and receive all work with IPv4 addresses, and an IPv6 literal such as "::1" is refused with ErrAddressInvalid." Echo in bind's `host` description ("an IPv4 literal or a resolvable host name").

### 2. Host names are resolved on every send and bind, undocumented
UNIT:      man-pkg:udp
PAGE:      udp::send, udp::bind
CATEGORY:  coverage
CLAIM:     send's `to`: "The destination address, typically from net::lookup or from a received datagram's from field." The Errors table lists ErrAddressNotFound ("Network host name or address could not be resolved") but no sentence on the page says when it fires.
VERDICT:   missing
EVIDENCE:  `src/codegen/builtins/udp/gen_io.rs:658-660` doc comment: "resolve the destination `Address` with `getaddrinfo` and send a single datagram"; `:758` `getaddrinfo(host, NULL, &hints, &res)` and `:900` `resolve_fail` → `ErrAddressNotFound` (:903). So `to.host` may be a name like "localhost" and is resolved per call, which can block on DNS. The same holds for bind (`:30` "resolve the local host with `getaddrinfo`"), yet bind's `host` lists only literals. `mfb man net types` describes `Address.host` as "The peer's textual IP address", so nothing in the unit tells a developer a name is accepted or that it costs a resolution each send. Observing the resolution needs a socket: NEEDS-NETWORK-PROBE (`udp::send(sock, {host:="localhost", port:=p}, "x")` and `udp::send` with host "no.such.host.invalid" → ErrAddressNotFound).
SUGGESTED: send: "to.host may be an IPv4 literal or a host name. A name is resolved on every send, so resolve once with net::lookup and send to the result; a name that does not resolve raises ErrAddressNotFound." bind: "host is an IPv4 literal or a name that resolves to a local interface."

### 3. List-form poll reports a moved socket as "closed", contradicting the overview and close page
UNIT:      man-pkg:udp
PAGE:      udp::poll (vs overview, udp::close)
CATEGORY:  consistency
CLAIM:     Overview: "a socket may be handed to another thread with thread::transfer — any use of it afterwards on the sending thread raises ErrResourceMoved." close: "a socket handed to another thread is refused with ErrResourceMoved — which says what actually happened — rather than ErrResourceClosed." poll's Errors table lists ErrResourceMoved for overload 1 only.
VERDICT:   inconsistent
EVIDENCE:  `src/codegen/builtins/udp/func_poll.rs:143` overload 2 errors: `["ErrInvalidArgument", "ErrOutOfMemory", "ErrResourceClosed", "ErrTimeout"]` — no ErrResourceMoved. `src/codegen/os/socket/poll.rs:186` `lower_net_poll_list_helper` never calls `closed_or_moved_guard` (`grep -n closed_or_moved_guard src/codegen/os/socket/poll.rs` hits :75 and :488, both outside 186-460); its only resource error is at :227-235, "a stale/closed fd → ErrResourceClosed". Since `thread::transfer` leaves the fd itself valid (`src/codegen/resource/mod.rs:259-265`: it stores `moved|closed` in the flag word), a list containing a transferred socket will either poll the other thread's live fd or raise ErrResourceClosed, never ErrResourceMoved. This is a probable code bug as well as a doc mismatch; needs a bind plus a thread to demonstrate: NEEDS-NETWORK-PROBE.
SUGGESTED: Until the lowering is fixed, poll's description must carve out the list form: "In the list form a socket that was handed to another thread is reported as ErrResourceClosed, not ErrResourceMoved." And soften the overview's "any other use" to exclude it, or (preferably) fix the lowering and file the bug.

### 4. setReadTimeout says poll "raises nothing"
UNIT:      man-pkg:udp
PAGE:      udp::setReadTimeout (vs udp::poll)
CATEGORY:  consistency
CLAIM:     "Use udp::poll instead when the question is "has anything arrived?" rather than "receive, but not forever": poll answers with a Boolean and raises nothing."
VERDICT:   wrong
EVIDENCE:  `mfb man udp poll` Errors table: ErrInvalidArgument (negative timeout, empty list), ErrResourceClosed, ErrResourceMoved, and for the list form ErrTimeout; `src/codegen/builtins/udp/func_poll.rs:129,143`. The poll page itself says "a deadline that expires with none ready raises ErrTimeout" for the list form. Two sibling pages describe one function's error contract differently.
SUGGESTED: "poll answers with a Boolean, and an expired deadline is FALSE rather than an error."

### 5. Overview never points at the waiting tools it says you need
UNIT:      man-pkg:udp
PAGE:      overview
CATEGORY:  discoverability
CLAIM:     "a datagram that is lost is simply never received" — the Description names only bind, send, receive and close; poll, setReadTimeout, setWriteTimeout and localAddress appear only in the alphabetical function table.
VERDICT:   misleading
EVIDENCE:  `mfb man udp | sed -n 1,40p` — no occurrence of `poll`, `setReadTimeout` or `localAddress` in the Description. Yet `mfb man udp setReadTimeout` says "Any protocol expecting a reply should set a timeout and treat its expiry as loss", and `mfb man udp bind` says binding port 0 then reading `localAddress` "is the right way for a client to bind". The overview makes the loss claim that motivates both, then leaves the reader to discover the remedy from a one-line table summary.
SUGGESTED: After the loss sentence add: "Because a lost datagram never arrives, a receive with no deadline can wait forever: use udp::setReadTimeout to bound it, or udp::poll to ask whether anything is queued before receiving. A client should bind port 0 and read the port the OS chose with udp::localAddress."
