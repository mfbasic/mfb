### 1. Windows backend is invisible in the overview and connect
UNIT:      man-pkg:tls
PAGE:      package-wide (overview Description; tls::connect)
CATEGORY:  consistency
CLAIM:     "TLS is implemented on Linux by driving the system OpenSSL library (libssl.so.3, falling back to libssl.so.1.1) so a single binary spans OpenSSL 1.1.1 and 3.x; the macOS backend drives Network.framework through a synchronous bridge." (overview, repeated verbatim on connect, followed there by "If the TLS layer cannot be initialized — neither OpenSSL library can be loaded ... connect raises ErrTlsFailed.")
VERDICT:   inconsistent
EVIDENCE:  Only two platforms are named, yet sibling pages assume a third: connect's allowSelfSigned note says "while Linux and Windows accept it", localAddress says "On Linux and Windows the bound address is read back", listen says "on every platform". `grep -n "Schannel" src/codegen/builtins/tls/mod.rs` shows `SlotBackend::Schannel` and `gen_schannel` (lines 11-12, 34, 207, 263); `ls src/codegen/builtins/tls/` lists gen_schannel*.rs. A Windows developer reading the overview concludes there is no Windows TLS. The paragraph is also internal-leaning (library file names) for a man page.
SUGGESTED: Replace the backend paragraph in both places with one platform-neutral sentence: "TLS is provided by the operating system's own TLS library on every platform (OpenSSL on Linux, Schannel on Windows, Network.framework on macOS); if that library cannot be loaded, tls::connect and tls::listen raise ErrTlsFailed." Keep it in the overview only and drop the duplicate from connect.

### 2. Timeouts, poll and the address reporters are unreachable from the pages that need them
UNIT:      man-pkg:tls
PAGE:      package-wide (overview Description; tls::read; tls::write)
CATEGORY:  discoverability
CLAIM:     The overview prose names connect, listen, accept, read, write, close and nothing else; tls::read and tls::write both list ErrTimeout in their Errors tables with no sentence saying what raises it, and neither page's See also points at tls::setReadTimeout / tls::setWriteTimeout.
VERDICT:   missing
EVIDENCE:  `grep -n "Timeout" src/codegen/builtins/tls/func_read.rs src/codegen/builtins/tls/func_write.rs` hits only the `errors:` vectors (func_read.rs:95, func_write.rs:111,132), never the prose. Rendered overview Description (`mfb man tls`, paragraphs 1-5) never mentions poll, setReadTimeout, setWriteTimeout, localAddress or remoteAddress; the sibling `mfb man tcp` overview names tcp::poll, tcp::localAddress, tcp::remoteAddress and the IMPORT net requirement in its second and third paragraphs. A developer asking "how do I stop read blocking forever?" on the read page finds ErrTimeout in the table and no path to the setter.
SUGGESTED: Overview: add one paragraph after the read/write paragraph: "tls::poll waits for readability over one socket or a list; tls::setReadTimeout and tls::setWriteTimeout put a deadline on a socket's reads and writes (a read or write that reaches it raises ErrTimeout); tls::localAddress and tls::remoteAddress report net::Address values, so a file that reads their fields must also IMPORT net." read desc: add "A read raises ErrTimeout only when a deadline set by tls::setReadTimeout elapses." write desc: the equivalent for setWriteTimeout. This also makes the See also derive those links.

### 3. setReadTimeout says poll "raises nothing"; the poll page lists six errors
UNIT:      man-pkg:tls
PAGE:      tls::setReadTimeout vs tls::poll
CATEGORY:  consistency
CLAIM:     "Use tls::poll instead when the question is "is there data?" rather than "read, but not forever": poll answers with a Boolean and raises nothing."
VERDICT:   inconsistent
EVIDENCE:  func_set_read_timeout.rs:32 carries the sentence. The poll page's own prose: "A negative timeoutMs is rejected with ErrInvalidArgument"; "expiry raises ErrTimeout (a producing call)" for the list form; its Errors table (func_poll.rs:146,179) lists ErrInvalidArgument, ErrNetworkFailed, ErrOutOfMemory, ErrResourceClosed, ErrResourceMoved, ErrTimeout, ErrTlsFailed. Only the single-socket form's expiry is a FALSE rather than an error.
SUGGESTED: "...poll on a single socket answers with a Boolean, returning FALSE on expiry instead of raising ErrTimeout."

### 4. No page says which TLS versions the server side accepts
UNIT:      man-pkg:tls
PAGE:      tls::listen / tls::accept (coverage gap)
CATEGORY:  coverage
CLAIM:     tls::connect states "the handshake negotiates TLS 1.2 or later — older protocol versions are refused"; listen and accept say nothing about the protocol floor, so a developer running a server cannot answer "will a TLS 1.0/1.1 client get in?"
VERDICT:   missing
EVIDENCE:  `grep -n "1.2\|version" src/codegen/builtins/tls/func_listen.rs src/codegen/builtins/tls/func_accept.rs` returns only "This version ..." phrasing, no protocol statement. The implementation enforces a floor: gen_openssl/listen.rs:313-330 sets SSL_CTRL_SET_MIN_PROTO_VERSION to TLS1_2_VERSION (pinned by test `listen_lowers_with_min_proto_check`), gen_schannel_server.rs:602 enables SP_PROT_TLS1_2_SERVER only. macOS server floor: UNVERIFIED — `grep -rn "TLS1_2\|tls_protocol_version" src/codegen/builtins/tls/gen_macos/server` returned nothing, so whether Network.framework's server default is 1.2 I could not confirm from source; a live handshake would be needed (NEEDS-NETWORK-PROBE).
SUGGESTED: On listen (and echoed in the overview's "no mutual TLS" sentence): "The listener accepts TLS 1.2 and later only; a client offering an older protocol fails its handshake in tls::accept with ErrTlsFailed." Confirm the macOS floor before stating "on every platform".
