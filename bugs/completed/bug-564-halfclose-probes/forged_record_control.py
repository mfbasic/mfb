import socket, ssl, subprocess, sys, os, time
srv = subprocess.Popen([sys.argv[1]], stdout=subprocess.PIPE, text=True)
port = int(srv.stdout.readline().split('=')[1])
ctx = ssl.create_default_context(); ctx.check_hostname = False; ctx.verify_mode = ssl.CERT_NONE
raw = socket.create_connection(('127.0.0.1', port))
s = ctx.wrap_socket(raw)
os.write(s.fileno(), b'\x17\x03\x03\x00\x20' + b'G' * 32)   # a forged application-data record
time.sleep(0.5)
print('garbage', srv.stdout.readline().strip())
