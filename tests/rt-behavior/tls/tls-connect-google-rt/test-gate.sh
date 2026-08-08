#!/usr/bin/env bash
# Environment gate for tls-connect-google-rt (consumed by scripts/test-accept.sh).
#
# The fixture performs a real TLS handshake against Google Public DNS
# (8.8.8.8:443, whose dns.google certificate names the bare IP) and diffs stable
# booleans against its golden. It can only pass on a machine that can actually
# open that connection. On an offline, firewalled, or captive-portal host the
# handshake fails and the fixture's build.log diverges from the golden — a red
# that says nothing about the compiler. This gate probes TCP reachability to
# 8.8.8.8:443; when it cannot be reached it exits non-zero, and test-accept.sh
# skips the fixture (recording this reason) instead of failing it.
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
