#!/usr/bin/env python3
"""UDP echo peer for validating udp::send / udp::receive against a real peer.

The acceptance suite can only test `udp` against itself: every existing fixture
binds two MFBASIC sockets and sends between them, which proves the two halves
agree but not that either half matches the wire. This peer is an ordinary
POSIX-sockets program, so a round trip through it exercises the real datagram
path -- including that the sender address MFBASIC reports in `Datagram.from` is
the address this peer actually sent from.

Echoes each datagram back to its sender verbatim. A datagram whose payload is
exactly ``QUIT`` is echoed and then ends the server, so a caller does not have
to kill it. A zero-length datagram is ordinary and is echoed as such.

Prints the chosen port on stdout (one line) before serving, so the caller can
read it without guessing.

Usage: net_udp_echo_server.py [hold_seconds]
"""
import socket
import sys
import time

HOLD_SECONDS = float(sys.argv[1]) if len(sys.argv) > 1 else 30.0
MAX_DATAGRAM = 65535

server = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", 0))
port = server.getsockname()[1]

print(port, flush=True)

deadline = time.monotonic() + HOLD_SECONDS
while True:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        break
    server.settimeout(remaining)
    try:
        payload, peer = server.recvfrom(MAX_DATAGRAM)
    except socket.timeout:
        break
    server.sendto(payload, peer)
    if payload == b"QUIT":
        break
