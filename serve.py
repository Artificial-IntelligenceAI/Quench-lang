#!/usr/bin/env python3
"""A preview server that does not let the browser hold on to anything.

`python3 -m http.server` sends no `Cache-Control`, so browsers fall back to
guessing how long a file stays fresh — and a stylesheet edited a second ago is
served from memory for minutes afterwards. Editing the CSS and seeing the old
page is not a thing worth debugging twice, so nothing here is cacheable.

Threaded, like the standard library's own command line is. A browser asks for the
page, the stylesheet and the script at once; a server that answers one at a time
leaves the rest waiting, and with nothing cached that happens on every single
load rather than only the first.
"""

import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer


class NoCache(SimpleHTTPRequestHandler):
    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store, must-revalidate")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 5173
    print(f"serving this directory on http://localhost:{port} — nothing cached")
    ThreadingHTTPServer(("127.0.0.1", port), NoCache).serve_forever()
