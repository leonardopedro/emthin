#!/usr/bin/env python3
"""TLS/WebSocket front end for lease_fixture.Gateway.

The scripted Core from ``lease_fixture.py`` is reused verbatim: this module
supplies a socket-shaped object whose ``recv``/``sendall`` ride WebSocket
binary messages, so ``Gateway.serve`` sees the same byte stream it sees over
plain TCP.  That keeps one implementation of the protocol and varies only the
transport, which is exactly what a client's WSS path needs to be tested
against.

Usage:
    wss_fixture.py --listen 127.0.0.1:47656 --admin 127.0.0.1:47657 \\
        --cert cert.pem --key key.pem

The certificate is presented as-is: a test client trusts it explicitly.  This
fixture does not implement pairing, tailnet names, or the Core upstream; it
speaks the stream half of docs/HERDR_GATEWAY_V1.md so the pad's TLS, upgrade,
framing and lease logic can be exercised end to end.
"""

import argparse
import base64
import hashlib
import socket
import ssl
import struct
import sys
import threading

sys.path.insert(0, __file__.rsplit("/", 1)[0])
sys.dont_write_bytecode = True  # the fixture directory is tracked; no .pyc churn
import lease_fixture  # noqa: E402  (path is set up directly above)

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
SUBPROTOCOL = "emthin-terminal.v1"
PATH = "/v1/stream"
MAX_HEAD = 8 * 1024
MAX_MESSAGE = 64 * 1024

OPCODE_CONTINUATION = 0x0
OPCODE_TEXT = 0x1
OPCODE_BINARY = 0x2
OPCODE_CLOSE = 0x8
OPCODE_PING = 0x9
OPCODE_PONG = 0xA


class Channel:
    """One WebSocket connection, presented as a byte stream to the gateway."""

    def __init__(self, sock):
        self.sock = sock
        self.buffer = bytearray()
        self.closed = False
        self.lock = threading.Lock()

    # -- socket-shaped surface used by lease_fixture -------------------
    def recv(self, length):
        while not self.buffer:
            if self.closed:
                return b""
            if not self._read_message():
                return b""
        out = bytes(self.buffer[:length])
        del self.buffer[:length]
        return out

    def sendall(self, data):
        with self.lock:
            if self.closed:
                raise OSError("closed")
            try:
                self.sock.sendall(encode(opcode=OPCODE_BINARY, payload=bytes(data), mask=False))
            except OSError:
                self.closed = True
                raise

    def close(self):
        with self.lock:
            if self.closed:
                return
            self.closed = True
            try:
                self.sock.sendall(encode(opcode=OPCODE_CLOSE,
                                         payload=struct.pack(">H", 1000), mask=False))
            except OSError:
                pass
            try:
                self.sock.close()
            except OSError:
                pass

    # -- framing ------------------------------------------------------
    def _recv_exact(self, length):
        out = bytearray()
        while len(out) < length:
            chunk = self.sock.recv(length - len(out))
            if not chunk:
                self.closed = True
                return None
            out.extend(chunk)
        return bytes(out)

    def _read_message(self):
        payload = bytearray()
        started = False
        while True:
            header = self._recv_exact(2)
            if header is None:
                return False
            first, second = header[0], header[1]
            fin = bool(first & 0x80)
            opcode = first & 0x0F
            masked = bool(second & 0x80)
            length = second & 0x7F
            if length == 126:
                extended = self._recv_exact(2)
                if extended is None:
                    return False
                length = struct.unpack(">H", extended)[0]
            elif length == 127:
                extended = self._recv_exact(8)
                if extended is None:
                    return False
                length = struct.unpack(">Q", extended)[0]
            if length > MAX_MESSAGE:
                raise ValueError("message too large")
            if not masked:
                # A client must mask every frame (RFC 6455 §5.3).
                raise ValueError("client frame is not masked")
            key = self._recv_exact(4)
            if key is None:
                return False
            body = self._recv_exact(length)
            if body is None:
                return False
            body = bytes(b ^ key[i & 3] for i, b in enumerate(body))

            if opcode == OPCODE_CLOSE:
                self.close()
                return False
            if opcode == OPCODE_PING:
                with self.lock:
                    self.sock.sendall(encode(opcode=OPCODE_PONG, payload=body, mask=False))
                continue
            if opcode == OPCODE_PONG:
                continue
            if opcode == OPCODE_TEXT:
                raise ValueError("text messages are not part of this protocol")
            if opcode == OPCODE_CONTINUATION and not started:
                raise ValueError("continuation without a message")
            if opcode == OPCODE_BINARY:
                if started:
                    raise ValueError("new message before continuation")
                started = True
            payload.extend(body)
            if fin:
                self.buffer.extend(payload)
                return True


def encode(opcode, payload, mask):
    out = bytearray()
    out.append(0x80 | opcode)
    length = len(payload)
    flag = 0x80 if mask else 0
    if length <= 125:
        out.append(flag | length)
    elif length <= 0xFFFF:
        out.append(flag | 126)
        out.extend(struct.pack(">H", length))
    else:
        out.append(flag | 127)
        out.extend(struct.pack(">Q", length))
    if mask:
        key = b"\x00\x00\x00\x00"
        out.extend(key)
        out.extend(payload)
    else:
        out.extend(payload)
    return bytes(out)


def read_head(sock):
    data = bytearray()
    while b"\r\n\r\n" not in data:
        if len(data) > MAX_HEAD:
            raise ValueError("handshake head too long")
        chunk = sock.recv(1024)
        if not chunk:
            raise ConnectionError("peer closed during the handshake")
        data.extend(chunk)
    head, _, rest = data.partition(b"\r\n\r\n")
    return head.decode("iso-8859-1"), rest


def upgrade(sock):
    head, rest = read_head(sock)
    lines = head.split("\r\n")
    if not lines or not lines[0].startswith("GET "):
        raise ValueError("not a GET")
    _, path, version = (lines[0].split(" ") + ["", ""])[:3]
    if path != PATH:
        raise ValueError("unexpected path {!r}".format(path))
    if version != "HTTP/1.1":
        raise ValueError("unexpected version")
    headers = {}
    for line in lines[1:]:
        if not line:
            continue
        name, _, value = line.partition(":")
        headers[name.strip().lower()] = value.strip()
    if headers.get("upgrade", "").lower() != "websocket":
        raise ValueError("missing upgrade header")
    if "upgrade" not in [part.strip().lower()
                         for part in headers.get("connection", "").split(",")]:
        raise ValueError("missing connection upgrade token")
    if headers.get("sec-websocket-version") != "13":
        raise ValueError("bad version")
    offered = [part.strip() for part in
               headers.get("sec-websocket-protocol", "").split(",") if part.strip()]
    if SUBPROTOCOL not in offered:
        raise ValueError("subprotocol not offered")
    key = headers.get("sec-websocket-key")
    if not key:
        raise ValueError("missing key")
    accept = base64.b64encode(hashlib.sha1((key + GUID).encode()).digest()).decode()
    response = (
        "HTTP/1.1 101 Switching Protocols\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        "Sec-WebSocket-Accept: {}\r\n"
        "Sec-WebSocket-Protocol: {}\r\n"
        "\r\n"
    ).format(accept, SUBPROTOCOL)
    sock.sendall(response.encode("iso-8859-1"))
    return rest


def serve_channel(gateway, sock, address):
    try:
        leftover = upgrade(sock)
        channel = Channel(sock)
        channel.buffer.extend(leftover)
        gateway.serve(channel, address)
    except (ValueError, ConnectionError, OSError):
        try:
            sock.close()
        except OSError:
            pass


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="127.0.0.1:47656")
    parser.add_argument("--admin", default="127.0.0.1:47657")
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    args = parser.parse_args()
    listen_host, listen_port = args.listen.rsplit(":", 1)
    admin_host, admin_port = args.admin.rsplit(":", 1)

    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(args.cert, args.key)
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind((listen_host, int(listen_port)))
    listener.listen(8)
    tls_listener = context.wrap_socket(listener, server_side=True)

    admin = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    admin.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    admin.bind((admin_host, int(admin_port)))
    admin.listen(4)

    gateway = lease_fixture.Gateway()
    threading.Thread(target=lease_fixture.admin_loop, args=(gateway, admin),
                     daemon=True).start()
    print("wss fixture listening {} admin {}".format(args.listen, args.admin),
          flush=True)
    try:
        while True:
            try:
                conn, address = tls_listener.accept()
            except ssl.SSLError:
                # A client that rejects our certificate is not a fixture fault.
                continue
            threading.Thread(target=serve_channel, args=(gateway, conn, address),
                             daemon=True).start()
    except KeyboardInterrupt:
        pass
    finally:
        listener.close()
        admin.close()


if __name__ == "__main__":
    main()
