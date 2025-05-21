#!/usr/bin/env python3
import base64
import http.server
import json
import os

from urllib.parse import urlparse

PORT = 8000
RESPONSES = {
    "/hello-world": (200, { "hello": "world" })
}

class Handler(http.server.BaseHTTPRequestHandler):
    def set_headers(self, status_code):
        self.send_response(status_code)
        self.send_header('Content-type', 'application/json')
        self.end_headers()

    def do_GET(self):
        auth = self.headers.get('Authorization')

        if auth == None or auth != f'Basic {self.server.key}':
            self.set_headers(401)
            resp = { 'error': 'unauthorized' }
            self.wfile.write(bytes(json.dumps(resp), 'utf-8'))
            return

        path = urlparse(self.path).path
        (status_code, resp) = RESPONSES.get(path, (404, { "error": "not found" }))
        self.set_headers(status_code)
        self.wfile.write(bytes(json.dumps(resp), 'utf-8'))


class CustomHTTPServer(http.server.HTTPServer):
    key = ''

    def __init__(self, address, userpass):
        super().__init__(address, Handler)
        self.key = base64.b64encode(
            bytes(userpass, 'utf-8')).decode('utf-8')


if __name__ == '__main__':
    userpass = os.environ['AUTH_HEADER']
    server = CustomHTTPServer(('', PORT), userpass)
    server.serve_forever()
