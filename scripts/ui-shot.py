#!/usr/bin/env python3
"""Headless-ish screenshots of the built UI in real WebKitGTK (WebKit2 4.1).

Serves ui/dist on a loopback port (the UI falls back to its mock API outside
Tauri), opens a route, optionally runs JS, and saves a PNG of the viewport.

    pnpm --dir ui build
    python3 scripts/ui-shot.py --route '#/library' --out shot.png
    python3 scripts/ui-shot.py -W 760 -H 520 --route '#/watch/G/key' \
        --js 'document.querySelector("video").pause()' --out small.png
    # play files: serve a folder laid out like the fixture paths (see
    # scripts/ui-media-stub.py) as the mock media server
    python3 scripts/ui-shot.py --media-root /path/to/media --route ... --out p.png

Needs a display (X11/Wayland) and python3-gobject with WebKit2 4.1.
"""
import argparse
import functools
import http.server
import os
import re
import threading

import gi

gi.require_version("WebKit2", "4.1")
gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk, WebKit2  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class RangeHandler(http.server.SimpleHTTPRequestHandler):
    """Static handler with single-range support so <video> can seek."""

    def log_message(self, *args):
        pass

    def end_headers(self):
        # The preview page and the media root are served on different ports.
        self.send_header("Access-Control-Allow-Origin", "*")
        super().end_headers()

    def send_head(self):
        rng = self.headers.get("Range")
        path = self.translate_path(self.path)
        if not rng or not os.path.isfile(path):
            return super().send_head()
        m = re.match(r"bytes=(\d*)-(\d*)", rng)
        size = os.path.getsize(path)
        start = int(m.group(1) or 0) if m else 0
        end = int(m.group(2)) if m and m.group(2) else size - 1
        end = min(end, size - 1)
        f = open(path, "rb")
        f.seek(start)
        self.send_response(206)
        self.send_header("Content-Type", self.guess_type(path))
        self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Content-Length", str(end - start + 1))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        self.range_left = end - start + 1
        return f

    def copyfile(self, source, outputfile):
        left = getattr(self, "range_left", None)
        if left is None:
            return super().copyfile(source, outputfile)
        while left > 0:
            chunk = source.read(min(65536, left))
            if not chunk:
                break
            try:
                outputfile.write(chunk)
            except (BrokenPipeError, ConnectionResetError):
                break
            left -= len(chunk)


class QuietServer(http.server.ThreadingHTTPServer):
    def handle_error(self, request, client_address):
        pass  # the webview drops media connections when it has enough data


def serve(directory):
    handler = functools.partial(RangeHandler, directory=directory)
    httpd = QuietServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd.server_address[1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-W", "--width", type=int, default=1280)
    ap.add_argument("-H", "--height", type=int, default=800)
    ap.add_argument("--route", default="#/")
    ap.add_argument("--js", default="", help="JS run after the route has rendered")
    ap.add_argument("--wait", type=int, default=1800, help="ms before running --js")
    ap.add_argument("--after", type=int, default=900, help="ms between --js and the shot")
    ap.add_argument("--media-root", help="library folder served as the media base")
    ap.add_argument("--dist", default=os.path.join(ROOT, "ui", "dist"))
    ap.add_argument("--page", default="index.html", help="page inside --dist to open")
    ap.add_argument("--transparent", action="store_true", help="keep transparency (icons)")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()

    port = serve(args.dist)
    query = ""
    if args.media_root:
        mport = serve(os.path.expanduser(args.media_root))
        query = f"?media=http://127.0.0.1:{mport}"
    url = f"http://127.0.0.1:{port}/{args.page}{query}{args.route}"

    webview = WebKit2.WebView.new_with_context(WebKit2.WebContext.new_ephemeral())
    settings = webview.get_settings()
    settings.set_enable_media(True)
    settings.set_media_playback_requires_user_gesture(False)
    snap_opts = WebKit2.SnapshotOptions.NONE
    if args.transparent:
        from gi.repository import Gdk  # noqa: E402

        webview.set_background_color(Gdk.RGBA(0, 0, 0, 0))
        snap_opts = WebKit2.SnapshotOptions.TRANSPARENT_BACKGROUND
    win = Gtk.OffscreenWindow() if os.environ.get("UI_SHOT_OFFSCREEN") else Gtk.Window()
    win.add(webview)
    win.set_default_size(args.width, args.height)
    win.show_all()
    webview.load_uri(url)

    def run_js():
        if args.js:
            webview.evaluate_javascript(args.js, -1, None, None, None, None, None)
        GLib.timeout_add(args.after, snap)
        return False

    def snap():
        def done(view, res):
            surface = view.get_snapshot_finish(res)
            surface.write_to_png(args.out)
            print("saved", args.out)
            Gtk.main_quit()

        webview.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, snap_opts, None, done)
        return False

    GLib.timeout_add(args.wait, run_js)
    GLib.timeout_add(args.wait + args.after + 20000, Gtk.main_quit)
    Gtk.main()


if __name__ == "__main__":
    main()
