#!/usr/bin/env python3
"""TCP "blackhole" server for validating net::connectTcp timeout behavior.

Listens on 127.0.0.1 with a tiny accept backlog and then saturates that backlog
with pending half-open connections. Once the backlog is full the kernel drops
further SYNs, so a new connect() receives no SYN-ACK and must block until its
deadline -- exactly the condition a connect timeout must bound. Without this a
closed port would answer with RST (ECONNREFUSED) and an open port would complete
the handshake; neither exercises the timeout path.

Prints the chosen port on stdout (one line) and then sleeps so a client can
attempt to connect. Intended to be started in the background by
check-net-connect-timeout.sh.

Usage: net_blackhole_server.py [hold_seconds]
"""
import socket
import sys
import time

HOLD_SECONDS = int(sys.argv[1]) if len(sys.argv) > 1 else 30
BACKLOG_FILLERS = 512

server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", 0))
server.listen(1)
port = server.getsockname()[1]

# Saturate the listen/accept queue with pending connections and never accept
# them, so additional inbound SYNs are dropped rather than answered.
fillers = []
for _ in range(BACKLOG_FILLERS):
    client = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    client.setblocking(False)
    try:
        client.connect(("127.0.0.1", port))
    except BlockingIOError:
        pass
    except OSError:
        break
    fillers.append(client)

print(port, flush=True)
time.sleep(HOLD_SECONDS)
