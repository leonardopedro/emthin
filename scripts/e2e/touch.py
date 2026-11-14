#!/usr/bin/env python3
"""Inject a two-finger pan into a live emthin session.

XTEST, wtype and ydotool cannot synthesize multitouch, so the live harness
feeds contacts through emthin's `inject_touch` control method. That method
calls the same down/motion/up handlers as a real touchscreen; this script
only owns the gesture shape and the before/after assertion.

Usage:
    python3 scripts/e2e/touch.py <ipc-socket> [dx] [dy]

The page must already be zoomed enough to overflow the viewport; the script
reports the before/after pan and exits 1 if the pan did not move.
"""

import json
import socket
import sys
import time

TIMEOUT = 5.0


def send(sock, method, params=None):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}
    ).encode()
    sock.sendall(b"Content-Length: %d\r\n\r\n" % len(body) + body)


def recv_exact(sock, n):
    buf = b""
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError("peer closed")
        buf += chunk
    return buf


def recv_msg(sock):
    header = b""
    while b"\r\n\r\n" not in header:
        chunk = sock.recv(1)
        if not chunk:
            return None
        header += chunk
    length = None
    for line in header.decode("utf-8", "replace").split("\r\n"):
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1].strip())
    if length is None:
        return None
    return json.loads(recv_exact(sock, length).decode("utf-8", "replace"))


def state(sock):
    send(sock, "list_state")
    deadline = time.time() + TIMEOUT
    while time.time() < deadline:
        msg = recv_msg(sock)
        if msg is None:
            break
        if msg.get("method") == "state":
            return msg["params"]
    raise TimeoutError("no state notification")


def touch(sock, phase, slot, x, y):
    send(
        sock,
        "inject_touch",
        {"phase": phase, "slot": slot, "x": x, "y": y},
    )


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    socket_path = sys.argv[1]
    dx = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0
    dy = float(sys.argv[3]) if len(sys.argv) > 3 else -80.0

    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(TIMEOUT)
    sock.connect(socket_path)

    before = state(sock)
    if before["view_zoom"] <= 1.001:
        print(
            f"touch: page zoom is {before['view_zoom']:g}x, so it cannot pan",
            file=sys.stderr,
        )
        return 2

    x0, y0 = 450.0, 420.0
    x1, y1 = 550.0, 420.0
    touch(sock, "down", 0, x0, y0)
    touch(sock, "down", 1, x1, y1)
    touch(sock, "motion", 0, x0 + dx, y0 + dy)
    touch(sock, "motion", 1, x1 + dx, y1 + dy)
    touch(sock, "up", 0, x0 + dx, y0 + dy)
    touch(sock, "up", 1, x1 + dx, y1 + dy)

    after = state(sock)
    moved = (
        abs(after["view_pan_x"] - before["view_pan_x"]) > 0.01
        or abs(after["view_pan_y"] - before["view_pan_y"]) > 0.01
    )
    print(
        "touch pan: "
        f"({before['view_pan_x']:.1f},{before['view_pan_y']:.1f}) -> "
        f"({after['view_pan_x']:.1f},{after['view_pan_y']:.1f}) "
        f"zoom={after['view_zoom']:g}x"
    )
    return 0 if moved else 1


if __name__ == "__main__":
    sys.exit(main())
