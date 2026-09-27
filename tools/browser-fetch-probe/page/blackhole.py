#!/usr/bin/env python3
"""A raw-socket "black hole" server: accepts a connection and then never
writes and never closes it. Used by run.sh to reproduce bug-706 sub-issue 1
(fetchStyles's drive loop must not spin forever on a stylesheet request that
connects but then stalls)."""
import socket
import sys

port = int(sys.argv[1])
s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", port))
s.listen(5)
while True:
    conn, addr = s.accept()  # never write, never close
