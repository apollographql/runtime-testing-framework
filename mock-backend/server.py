import http.server
import sys
import threading
import urllib.parse

_store: dict[str, bytes] = {}
_lock = threading.Lock()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_PUT(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        key = urllib.parse.urlparse(self.path).path.lstrip("/")
        with _lock:
            _store[key] = body
        print(f"PUT {key} {len(body)} bytes", file=sys.stderr, flush=True)
        self.send_response(200)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self):
        key = urllib.parse.urlparse(self.path).path.lstrip("/")
        with _lock:
            data = _store.get(key)
        print(f"GET {key}", file=sys.stderr, flush=True)
        if data is None:
            body = b"not found"
            self.send_response(404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(200)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

    def log_message(self, format, *args):
        pass


if __name__ == "__main__":
    server = http.server.ThreadingHTTPServer(("", 8080), Handler)
    print("mock-backend listening on :8080", file=sys.stderr, flush=True)
    server.serve_forever()
