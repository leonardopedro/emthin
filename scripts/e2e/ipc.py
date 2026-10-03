#!/usr/bin/env python3
"""Minimal emthin IPC client: Content-Length framed JSON-RPC 2.0.

Usage: ipc.py <socket> <method> [json-params]
       ipc.py <socket> --script <file>     # one method per line, blank/# ignored
"""
import json
import socket
import sys
import time

SOCK = "/tmp/e2e/run/emthin.ipc"


def connect(path=SOCK, timeout=5.0):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(path)
    return s


def send(s, method, params=None):
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}
    ).encode()
    s.sendall(b"Content-Length: %d\r\n\r\n" % len(body) + body)


def recv_exact(s, n):
    buf = b""
    while len(buf) < n:
        chunk = s.recv(n - len(buf))
        if not chunk:
            raise EOFError("peer closed")
        buf += chunk
    return buf


def recv_msg(s):
    """Read one framed message; returns the parsed body."""
    # Headers.
    hdr = b""
    while b"\r\n\r\n" not in hdr:
        c = s.recv(1)
        if not c:
            return None
        hdr += c
    length = None
    for line in hdr.decode("utf-8", "replace").split("\r\n"):
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1].strip())
    if length is None:
        return None
    return json.loads(recv_exact(s, length).decode("utf-8", "replace"))


def collect(s, seconds=1.5):
    """Drain every message that arrives within `seconds`."""
    out = []
    end = time.time() + seconds
    while True:
        left = end - time.time()
        if left <= 0:
            break
        s.settimeout(left)
        try:
            m = recv_msg(s)
        except (socket.timeout, TimeoutError, EOFError, OSError):
            break
        if m is None:
            break
        out.append(m)
    return out


def main():
    args = sys.argv[1:]
    if args and args[0] == "--script":
        path = args[1]
        s = connect()
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#"):
                    continue
                method, _, raw = line.partition(" ")
                params = json.loads(raw) if raw.strip() else {}
                send(s, method, params)
                time.sleep(0.4)
                for m in collect(s, 1.0):
                    print(json.dumps(m, indent=2, sort_keys=True))
        return
    path = args[0]
    method = args[1]
    params = json.loads(args[2]) if len(args) > 2 else {}
    s = connect(path)
    send(s, method, params)
    for m in collect(s, 2.0):
        print(json.dumps(m, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()