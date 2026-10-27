I have everything needed. Writing up the findings now.

### 1. `net::Address` cannot be built or edited by the program, and no page says so
UNIT:      man-pkg:net
PAGE:      package-wide (overview, types, lookup)
CATEGORY:  coverage
CLAIM:     Overview: "net has no handles to open or close — every call takes and returns ordinary values." Types page: `net::Address` has no record description at all; lookup says port "is stored on the results ... so that the net::Address can be handed straight to tcp::connect".
VERDICT:   missing
EVIDENCE:  Probe `LET a = net::Address[host := "127.0.0.1", port := 80]` compiled with `mfb build -q proj` → `error[2-203-0039 TYPE_READ_ONLY_RECORD_CONSTRUCTOR]: TYPE net.Address is compiler-owned and cannot be constructed.` Probe `LET b = WITH a { port := 8080 }` on a `net::lookup` result → `error[2-203-0038 TYPE_READ_ONLY_RECORD_UPDATE]: TYPE net.Address is read-only and cannot be updated.` By contrast `net::Url[scheme := "http", ..., port := 80, ...]` and `WITH u { port := 8080 }` both compile and print `http://x.com/` / `http://x.com:8080/`. Source: `src/codegen/builtins/net/mod.rs` `pkg.add_record(RegistryRecord { name: ADDRESS_TYPE, ... description: "" ...`. `grep -n -i "read-only\|cannot be constructed\|WITH" all.txt types.txt` finds no statement of either restriction.
SUGGESTED: Give `net::Address` a record description on the types page, e.g. "The endpoint record shared by tcp, udp and tls. It is read-only: a program cannot build one with `net::Address[...]` or change one with `WITH`; the only sources are `net::lookup` (which stamps the port you pass), a received datagram's `from` field, and a socket's local/remote address query." On the lookup page, make explicit that this is *why* the `port` parameter exists: it is the only way to put a port on an Address, since the record cannot be edited afterward. The overview's "ordinary values" sentence should carry the same caveat, because `net::Url` is editable and `net::Address` is not.

### 2. The same failure, "host does not resolve", raises two different errors in two sibling functions, and neither page warns about the other
UNIT:      man-pkg:net
PAGE:      lookup, ping
CATEGORY:  consistency
CLAIM:     lookup: "Note that the resolver failing outright is an error, not an empty list." (Errors table: ErrAddressNotFound). ping: "A host that does not resolve raises ErrAddressInvalid."
VERDICT:   inconsistent
EVIDENCE:  Probe program (proj2) built with `mfb build -q proj2`, run: `net::lookup("no-such-host.invalid")` → `lookup err 77070002 Network host name or address could not be resolved.`; `net::ping("no-such-host.invalid", 0)` → `ping err 77070001 Network host, address, or port is invalid.` Source: `gen_io.rs` `resolve_fail` label → `emit_fail(... "ErrAddressNotFound" ...)`; `gen_ping/posix/mod.rs:975` emits `"ErrAddressInvalid"` and ping's `errors` vec in `func_ping.rs:181` has no `ErrAddressNotFound`. Both pages are individually true; the unit as a whole leaves a developer who writes `TRAP` on `ErrAddressNotFound` after reading lookup silently missing the same condition from ping.
SUGGESTED: On the ping page, after "A host that does not resolve raises ErrAddressInvalid", add: "— not ErrAddressNotFound as net::lookup does; resolve with net::lookup first if you need to tell an unknown name apart from a malformed one." On the lookup page's description, add the mirror sentence: "net::ping reports the same unresolvable host as ErrAddressInvalid." (If the divergence is unintended, that is a code bug to file, not a page fix; the pages must describe current behavior either way.)

### 3. The unit decodes URL text but never points to where encoding lives
UNIT:      man-pkg:net
PAGE:      package-wide (overview, percentDecode, parseQuery)
CATEGORY:  discoverability
CLAIM:     Overview: "net::percentDecode / net::parseQuery decode request-target components." percentDecode and parseQuery See-also lists reference only each other and net::toUrl.
VERDICT:   missing
EVIDENCE:  `grep -rln "percentEncode\|formUrlEncode" src/codegen/builtins/` → `src/codegen/builtins/encoding/func_percent_encode.rs`, `func_form_url_encode.rs`, and `encoding/func_percent_decode.rs`. `mfb man encoding percentDecode` describes a strict decoder with the same no-plus rule as net::percentDecode, and `mfb man encoding` lists `formUrlEncode/formUrlDecode`. Nothing in `all.txt` mentions `encoding` (`grep -c "encoding::" all.txt` → 0). A developer who parsed a URL with toUrl and wants to build one (encode a path segment or a query value) has no pointer from this unit to the package that does it, and will not guess there is a second `percentDecode` with a different package prefix.
SUGGESTED: In the overview's URL paragraph: "The inverse direction — percent-encoding a path segment or form-encoding a query — lives in the encoding package (encoding::percentEncode, encoding::formUrlEncode)." Add `encoding::percentEncode` to percentDecode's description (so it lands in See also) and `encoding::formUrlEncode` to parseQuery's, and one sentence on percentDecode noting that `encoding::percentDecode` applies the same strict rule, so either may be used on a path.

### 4. The types page describes `Address` fields as "the peer's", which is wrong for two of its three sources inside this package
UNIT:      man-pkg:net
PAGE:      types
CATEGORY:  consistency
CLAIM:     `host` — "The peer's textual IP address." `port` — "The peer's port."
VERDICT:   misleading
EVIDENCE:  `mod.rs:253,258` are the only prose on the record. Within this unit an Address is produced by `net::lookup` (a resolved host, no peer exists) and by `net::ping` (`result.address` is "the responder for Ok, the reporting router for Unreachable/TtlExceeded, and the destination that was aimed at for Timeout" per `PingResult.address`); the overview also lists "a socket's local ... address query" as a source, which is this program's end, not a peer. Only `tcp::remoteAddress` and udp's `from` are actually peers. The field text was written for one source and contradicts the overview's own list.
SUGGESTED: `host`: "The textual IPv4 address of the endpoint — a resolved host, a datagram's sender, a socket's own or remote end, or a ping responder." `port`: "The endpoint's port; 0 when the source has none (net::lookup without a port, net::ping)."

### 5. No page states how to get from a parsed `net::Url` to a connection or request
UNIT:      man-pkg:net
PAGE:      toUrl, overview
CATEGORY:  coverage
CLAIM:     Overview: "The transports live in their own packages: tcp for byte streams, ... http for requests and responses." toUrl See also: `net::Url`, `net::parseQuery`, `net::percentDecode` only.
VERDICT:   missing
EVIDENCE:  `mfb man http` overview and function table contain no reference to `net::Url`, `toUrl`, `parseQuery` or `percentDecode` (`mfb man http | grep -i "net::\|url\|query"` → no hits in the description). The net overview's URL paragraph never says what a `net::Url` is *for* (it is the only package that parses one), and the toUrl page shows a round trip through `toString` but never `u.host`/`u.port` feeding `net::lookup`/`tcp::connect` or `http::read`. The developer's arriving question — "I have an href, how do I fetch it / connect to it?" — gets no answer anywhere in the unit.
SUGGESTED: On the toUrl page, add a third example or a sentence: "`u.host` and `u.port` are the arguments `net::lookup(u.host, u.port)` and `tcp::connect(u.host, u.port)` take; `u.scheme = "https"` tells you to use tls instead." Name `tcp::connect` and `net::lookup` in the description so they land in See also. (If http::read takes an href string directly, say that too so a reader knows toUrl is not a prerequisite for it — I could not verify http::read's input without a socket.)
