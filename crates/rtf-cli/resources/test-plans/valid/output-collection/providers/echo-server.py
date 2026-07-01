import http.server
import os

class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-type', 'text/plain')
        self.end_headers()

        msg = os.environ.get('ECHO_MESSAGE', 'no message')
        self.wfile.write(f'env: {msg}'.encode())

    def log_message(self, format, *args):
        pass  # Suppress request logging

print('Starting echo server on port 8080...')
http.server.HTTPServer(('', 8080), Handler).serve_forever()
