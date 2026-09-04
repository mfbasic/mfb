import http.server, json
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps({"error": "\x1b[2K\rok: uses toolbox - [Verified]  ‮EVIL"}).encode()
        self.send_response(400)
        self.send_header("Content-Type","application/json")
        self.send_header("Content-Length",str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1",7799), H).serve_forever()
