#!/usr/bin/env python3
"""A preview server that does not let the browser hold on to anything.

`python3 -m http.server` sends no `Cache-Control`, so browsers fall back to
guessing how long a file stays fresh — and a stylesheet edited a second ago is
served from memory for minutes afterwards. Editing the CSS and seeing the old
page is not a thing worth debugging twice, so nothing here is cacheable.
"""

import sys
from http.server import HTTPServer, SimpleHTTPRequestHandler


class NoCache(SimpleHTTPRequestHandler):
    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store, must-revalidate")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 5173
    print(f"serving this directory on http://localhost:{port} — nothing cached")
    HTTPServer(("127.0.0.1", port), NoCache).serve_forever()
