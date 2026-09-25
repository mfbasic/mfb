import subprocess, sys, time
mode = sys.argv[1]  # 'exit' = wait peer exit before release, 'now' = release immediately after drained
import os
env=dict(os.environ)
if os.environ.get('PROBE'): env['DYLD_INSERT_LIBRARIES']='/tmp/b564p/probe.dylib'
srv = subprocess.Popen((['/usr/bin/time','-l'] if os.environ.get('TIMEL') else [])+['/tmp/b564p/'+os.environ.get('SRV','srv/build/b564p.out')+''], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env, stderr=subprocess.PIPE)
port = srv.stdout.readline().strip().split('=')[1]
import socket, threading
counts={'s2c':0}
lsock=socket.socket(); lsock.bind(('127.0.0.1',0)); lsock.listen(1); pport=lsock.getsockname()[1]
def relay(a,b,key):
    try:
        while True:
            d=a.recv(1<<16)
            if not d: break
            if key: counts[key]+=len(d)
            b.sendall(d)
    except Exception: pass
    try: b.shutdown(socket.SHUT_WR)
    except Exception: pass
def acceptor():
    c,_=lsock.accept(); u=socket.create_connection(('127.0.0.1',int(port)))
    threading.Thread(target=relay,args=(c,u,None),daemon=True).start()
    threading.Thread(target=relay,args=(u,c,'s2c'),daemon=True).start()
threading.Thread(target=acceptor,daemon=True).start()
peer = subprocess.Popen(['openssl','s_client','-msg','-connect','127.0.0.1:'+str(pport)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
time.sleep(0.5)
peer.stdin.write(b'hello\n'); peer.stdin.flush()
time.sleep(0.2)
peer.stdin.close()
drained = srv.stdout.readline().strip()
before=counts['s2c']
if mode == 'exit':
    peer.wait()
    time.sleep(0.2)
t=time.time()
srv.stdin.write('go\n'); srv.stdin.flush()
after = srv.stdout.readline().strip()
dt=time.time()-t
peer_alive_after_writes = peer.poll() is None
peer.wait()
out = peer.stdout.read().decode(errors='replace')
import re
recs=len(re.findall(r'<<< .*?RECORD HEADER|<<< ', out))
time.sleep(0.5)
err=srv.stderr.read() if os.environ.get('TIMEL') else ''
import re as _re
m=_re.search(r'(\d+)\s+maximum resident set size', err)
print('maxrss_MB=%s' % (int(m.group(1))//(1<<20) if m else '?'))
print('relayed_after_drain=%d' % (counts['s2c']-before)); print('peer_rx_trace_lines=%d peer_alive_after_writes=%s' % (out.count('<<<'), peer_alive_after_writes), mode, drained, '|', after, '| %.2fs'%dt, '| close_notify' if 'close_notify' in out or 'close notify' in out else '| NO close_notify', '| srv exit', srv.wait())
