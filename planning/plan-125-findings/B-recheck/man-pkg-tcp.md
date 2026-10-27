### 1. Overview promises a `tcp::listen` that takes a `net::Address`; no such overload exists
UNIT:      man-pkg:tcp
PAGE:      package-wide (overview Description, paragraph 3)
CATEGORY:  overview-mismatch
CLAIM:     "Only reading an address's fields needs it: passing an address to tcp::connect or tcp::listen, and calling tcp::read or tcp::write, need nothing but IMPORT tcp."
VERDICT:   wrong
EVIDENCE:  `mfb man tcp listen` renders a single declaration, `tcp::listen(host AS String, port AS Integer, [backlog AS Integer])`; `src/codegen/builtins/tcp/func_listen.rs` registers one `RegistryFunction` named "listen" with `params` host/port/backlog and no `net::Address` form. Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-tcp/proj_listen_addr` (`RES s2 = tcp::listen(bound)`) fails to build: `error[2-203-0022 TYPE_CALL_ARITY_MISMATCH]: Call to tcp.listen has 1 argument(s), expected 2 to 3.` The source string is at `src/codegen/builtins/tcp/mod.rs:141` (`MODULE_DESC`).
SUGGESTED: "Only reading an address's fields needs it: passing an address to tcp::connect, and calling tcp::read or tcp::write, need nothing but IMPORT tcp." (tcp::listen takes a host String and a port, never an address.)

### 2. The unit never says tcp is IPv4-only
UNIT:      man-pkg:tcp
PAGE:      package-wide (overview; tcp::connect `host`; tcp::listen `host`)
CATEGORY:  coverage
CLAIM:     tcp::connect: "The peer's host name or textual IP address." tcp::listen: "'0.0.0.0' or '' binds every interface". Overview: "an address resolved by net::lookup can be handed straight to tcp::connect".
VERDICT:   missing
EVIDENCE:  The shared resolver every tcp connect/listen goes through fixes the address family: `src/codegen/os/socket/shared/mod.rs:411` ("`ai_family = AF_INET`, `ai_socktype = socktype` (and `AI_PASSIVE` when …)"), `mod.rs:13` ("platforms for `AF_INET`"), `mod.rs:149` (`AF_INET = "2"`); `src/codegen/builtins/tcp/func_connect.rs:99-101` and `func_listen.rs:88` call the `gen_shared::lower_net_{connect,listen}_tcp*` helpers built on it. `grep -rn "AF_INET6" src/codegen/builtins/tcp/` → nothing. The sibling page is explicit: `mfb man net lookup` says "Resolve a host name to a list of IPv4 network addresses … Only IPv4 results are returned". Nothing in `mfb man tcp --all` contains "IPv4" or "IPv6" (`grep -c -i ipv /tmp/plan-125-scratch/B-recheck/man-pkg-tcp/all.txt` → 0). A developer passing an IPv6 literal, an IPv6-only host name, or `"::"` to listen gets a raise the unit does not predict. Which error code surfaces (ErrAddressNotFound vs ErrAddressInvalid) is NEEDS-NETWORK-PROBE; the IPv4-only hint itself is read from source.
SUGGESTED: Overview, after the net::Address paragraph: "Connections are IPv4 only. A host name is resolved to its IPv4 addresses, as net::lookup does; a name with only IPv6 records, or an IPv6 literal, raises ErrAddressNotFound / ErrAddressInvalid, and tcp::listen binds every IPv4 interface with '0.0.0.0'." Repeat one clause on the `host` parameter of connect and listen.

### 3. No page says there is no half-close
UNIT:      man-pkg:tcp
PAGE:      package-wide (overview; tcp::close; tcp::write)
CATEGORY:  coverage
CLAIM:     Overview: "tcp::close closes one earlier — … to let a peer observe the end of the stream promptly". tcp::close: "Closing a Socket tears down the connection, so a peer reading from it observes the end of the stream."
VERDICT:   missing
EVIDENCE:  `ls src/codegen/builtins/tcp/` → accept, close, connect, listen, localAddress, poll, read, remoteAddress, setReadTimeout, setWriteTimeout, write; `grep -rn -i "shutdown\|SHUT_WR" src/codegen/builtins/tcp/` → nothing. The only way to signal end-of-stream to the peer is tcp::close, which also ends reading. A developer writing the commonest client shape — send request, signal "I'm done sending", then read the reply until the peer closes — will look for a shutdown/half-close and find nothing that says it is absent. The `drain` example on tcp::read closes the client to end the stream, which only works because that example never reads a reply.
SUGGESTED: In the overview's close paragraph and on tcp::close: "There is no half-close: tcp::close ends both directions at once, so a protocol that needs the peer to know a message is complete must frame it (a length prefix, a delimiter, or a fixed size) rather than rely on end-of-stream — unless the sender has nothing further to read."

### 4. The `RES` element rule that tcp::poll's list form depends on is stated only on the tls overview
UNIT:      man-pkg:tcp
PAGE:      package-wide (overview; tcp::poll; types)
CATEGORY:  consistency
CLAIM:     tcp::poll `socks`: "List OF RES tcp::Socket — The sockets to wait on". Types page: "Each of these is a RES handle … mfb man variable explains the handle model in full."
VERDICT:   inconsistent
EVIDENCE:  Probe `/tmp/plan-125-scratch/B-recheck/man-pkg-tcp/proj_bare_list` with `LET conns AS List OF tcp::Socket = [c]` fails: `error[2-203-0082 TYPE_RESOURCE_REQUIRES_RES]: Collection element type tcp.Socket is a resource; mark it RES (e.g. List OF RES File)`. The sibling overview spells this out — `mfb man tls`: "Neither can be carried in a record. Both may be collection elements when the element type is spelled RES (a bare List OF tls::Socket is rejected with TYPE_RESOURCE_REQUIRES_RES); List OF RES tls::Socket is what the tls::poll multiplex form takes." `grep -n "RES tcp::Socket\|record" /tmp/plan-125-scratch/B-recheck/man-pkg-tcp/all.txt` shows the spelling appears only inside poll's declaration and example, never explained, and the record restriction is never mentioned. Two sibling packages with identical handle rules describe them to two different depths; the developer who reads tcp first gets the diagnostic cold.
SUGGESTED: Add to the tcp overview's handle paragraph (or the types page) the same two sentences the tls overview carries: neither handle can be a record field; either may be a collection element only when the element type is spelled `RES`, which is what tcp::poll's list form takes.

### 5. Overview prose omits the two timeout setters, so "how do I stop read blocking forever?" is not answerable from the overview
UNIT:      man-pkg:tcp
PAGE:      package-wide (overview Description)
CATEGORY:  discoverability
CLAIM:     "tcp::connect opens an outbound stream; tcp::listen and tcp::accept run a server; tcp::read and tcp::write transfer bytes; tcp::poll waits for readability, over one socket or a list of them."
VERDICT:   misleading
EVIDENCE:  `mfb man tcp | grep -c setReadTimeout` → 1, and that one hit is the derived Functions table row, not the prose. Every blocking call in the unit is bounded differently: accept/connect/poll take a `timeoutMs` argument, read/write do not and are bounded only by tcp::setReadTimeout / tcp::setWriteTimeout (tcp::read: "How long a read waits is governed by tcp::setReadTimeout, not by an argument here. With no read timeout set the call blocks until data arrives"). The overview's tour names every function except the two setters, so the split — the first thing a developer has to learn to write a robust client — is discoverable only by reading tcp::read's page.
SUGGESTED: Append to the tour sentence: "tcp::accept, tcp::connect and tcp::poll take an optional timeoutMs; tcp::read and tcp::write take none and are bounded instead by tcp::setReadTimeout and tcp::setWriteTimeout, which are properties of the socket."

### 6. tcp::write says tls::write's empty-write behaviour is "identical"; tls::write documents an exception
UNIT:      man-pkg:tcp
PAGE:      tcp::write
CATEGORY:  consistency
CLAIM:     "An empty bytes list — or an empty String — is a no-op: nothing is sent and the call succeeds without touching the socket. tls::write behaves identically."
VERDICT:   inconsistent
EVIDENCE:  `mfb man tls write` `bytes` parameter: "An empty list sends nothing and succeeds, unless a read has already reported the peer's close." and Description: "Once tls::read on this socket has raised ErrConnectionClosed … every later write raises ErrConnectionClosed at once and sends nothing, including an empty one". The tcp page states the no-op unconditionally and then asserts identity with a sibling that is conditional. Whether a tcp empty write after a read-reported close is a no-op or raises is NEEDS-NETWORK-PROBE (needs a connected pair); either way one of the two pages is wrong about the other.
SUGGESTED: Either drop "tls::write behaves identically", or, once the probe settles it, state tcp's own post-close behaviour and say precisely where it matches tls.
