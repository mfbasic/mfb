# bug-739 repro client. "trickle STEP_MS N": send the first N ClientHello bytes
# one at a time, STEP_MS apart, then finish the handshake. "late CONNECT_MS
# HELLO_MS": open TCP at CONNECT_MS, send the ClientHello at HELLO_MS.
import socket, ssl, sys, time
mode = sys.argv[1]  # late CONNECT HELLO | trickle STEP_MS NBYTES
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT); ctx.check_hostname=False; ctx.verify_mode=ssl.CERT_NONE
t0 = time.monotonic()
if mode == "late":
    time.sleep(float(sys.argv[2])/1000)
    raw = socket.create_connection(("127.0.0.1", 18737), timeout=60)
    time.sleep(max(0, t0 + float(sys.argv[3])/1000 - time.monotonic()))
    try:
        s = ctx.wrap_socket(raw, server_hostname="127.0.0.1"); print("client OK", s.recv(64))
    except Exception as e: print("client FAIL", e)
else:
    step = float(sys.argv[2])/1000; n = int(sys.argv[3])
    raw = socket.create_connection(("127.0.0.1", 18737), timeout=60)
    inc, out = ssl.MemoryBIO(), ssl.MemoryBIO()
    obj = ctx.wrap_bio(inc, out, server_hostname="127.0.0.1")
    try: obj.do_handshake()
    except ssl.SSLWantReadError: pass
    hello = out.read()
    for i in range(n):
        raw.sendall(hello[i:i+1]); time.sleep(step)
    raw.sendall(hello[n:])
    try:
        while True:
            try:
                obj.do_handshake(); break
            except ssl.SSLWantReadError:
                d = out.read()
                if d: raw.sendall(d)
                data = raw.recv(65536)
                if not data: raise Exception("eof")
                inc.write(data)
        d = out.read()
        if d: raw.sendall(d)
        print("client OK after %.0fms" % ((time.monotonic()-t0)*1000))
    except Exception as e: print("client FAIL", e, "after %.0fms" % ((time.monotonic()-t0)*1000))
