#!/usr/bin/env bash
# Environment gate for tls-read-eof-raises-rt (consumed by scripts/test-accept.sh).
#
# Same gate, same host, same reasoning as `tls-connect-google-rt`: this fixture
# drives a real TLS session against Google Public DNS (8.8.8.8:443, whose
# dns.google certificate names the bare IP) and reads it to the server's close,
# so it can only pass where that connection can actually be opened. On an
# offline, firewalled, or captive-portal host the handshake fails and the
# fixture's build.log diverges from its golden — a red that says nothing about
# the compiler.
#
# The EOF contract this pins is transport-independent (`tls/gen_shared.rs` and
# `tcp/gen_io.rs` both branch a zero-length receive to `ErrConnectionClosed`),
# but proving it needs a peer that really closes. Loopback is not an option: an
# MFBASIC TLS server needs a certificate/key pair, which a static fixture cannot
# carry. The plaintext half of the same contract runs with no network at all —
# see `rt-behavior/tcp/tcp-read-eof-raises-rt`.
#
# perl performs the probe: it is already a hard dependency of test-accept.sh (the
# per-fixture watchdog) and, unlike timeout(1), ships on macOS where this suite
# runs. The alarm bounds a stuck connect so the gate can never hang the run.
set -u

HOST=8.8.8.8
PORT=443
TIMEOUT=5

if perl -e '
    use strict; use warnings;
    use IO::Socket::INET;
    local $SIG{ALRM} = sub { exit 1 };
    alarm shift;
    my $s = IO::Socket::INET->new(
        PeerAddr => shift, PeerPort => shift, Proto => "tcp");
    alarm 0;
    exit($s ? 0 : 1);
  ' "$TIMEOUT" "$HOST" "$PORT" 2>/dev/null; then
  exit 0
fi

echo "no network: cannot reach $HOST:$PORT within ${TIMEOUT}s (live-TLS fixture needs outbound HTTPS to Google Public DNS)"
exit 1
