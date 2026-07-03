import http.server
import json
import sys
import threading
import urllib.parse

_store: dict[str, bytes] = {}
_lock = threading.Lock()

GCS_PREFIX = "/gcs/"
QUERY_RANGE_PATH = "/api/v1/query_range"


class Handler(http.server.BaseHTTPRequestHandler):
    def do_PUT(self):
        path = urllib.parse.urlparse(self.path).path
        if not path.startswith(GCS_PREFIX):
            self._respond(404, b"not found")
            return

        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        with _lock:
            _store[path] = body
        print(f"PUT {path} {len(body)} bytes", file=sys.stderr, flush=True)
        self._respond(200, b"")

    def do_GET(self):
        parsed = urllib.parse.urlparse(self.path)

        if parsed.path == QUERY_RANGE_PATH:
            self._handle_query_range(parsed.query)
            return

        if parsed.path.startswith(GCS_PREFIX):
            with _lock:
                data = _store.get(parsed.path)
            print(f"GET {parsed.path}", file=sys.stderr, flush=True)
            self._respond(200, data) if data is not None else self._respond(404, b"not found")
            return

        self._respond(404, b"not found")

    def _handle_query_range(self, raw_query: str):
        params = urllib.parse.parse_qs(raw_query)
        query = params.get("query", [""])[0]
        start = params.get("start", ["0"])[0]
        end = params.get("end", ["0"])[0]

        print(f"GET {QUERY_RANGE_PATH} query={query!r}", file=sys.stderr, flush=True)

        body = json.dumps(
            {
                "status": "success",
                "data": {
                    "resultType": "matrix",
                    "result": [
                        {
                            "metric": {"query": query},
                            "values": [[float(start), "1"], [float(end), "1"]],
                        }
                    ],
                },
            }
        ).encode()

        self._respond(200, body, content_type="application/json")

    def _respond(self, status: int, body: bytes, content_type: str | None = None):
        self.send_response(status)
        if content_type:
            self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def log_message(self, format, *args):
        pass


if __name__ == "__main__":
    server = http.server.ThreadingHTTPServer(("", 8080), Handler)
    print("mock-backend listening on :8080", file=sys.stderr, flush=True)
    server.serve_forever()
