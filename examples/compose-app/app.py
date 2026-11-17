import http.server, os

VERSION = os.environ.get("APP_VERSION", "dev")

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            ok = os.environ.get("BREAK_HEALTH") != "1"
            self.send_response(200 if ok else 500)
            self.end_headers()
            self.wfile.write(b"ok\n" if ok else b"unhealthy\n")
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.end_headers()
        self.wfile.write(f"compose app {VERSION}\n".encode())

http.server.ThreadingHTTPServer(("0.0.0.0", 8000), Handler).serve_forever()
