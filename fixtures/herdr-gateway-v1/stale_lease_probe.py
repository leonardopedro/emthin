#!/usr/bin/env python3
"""Raw gateway client that proves a revoked controller is refused on the wire.

The pad blocks input locally once a lease is revoked, so a device step that
only watches the app cannot show what the *gateway* does with a controller's
bytes after the operator revoke: nothing ever crosses the transport.  This
probe is that sender.  It connects to the TLS/WebSocket fixture
(``wss_fixture.py``), attaches as the controller, waits to be revoked, and
then keeps writing stream DATA on the revoked channel — which the gateway
must answer with a CLOSE carrying ``not_controller``, without echoing the
bytes.

Usage:
    stale_lease_probe.py --listen 127.0.0.1:47658 --ca cert.pem

Prints one ``probe:`` line per phase on stdout and exits non-zero with a
``probe:`` line explaining the first thing that did not happen, so the calling
lane can quote it instead of guessing.
"""

import argparse
import base64
import hashlib
import json
import os
import socket
import ssl
import struct
import sys
import time

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
SUBPROTOCOL = "emthin-terminal.v1"
PATH = "/v1/stream"

OPCODE_BINARY = 0x2
OPCODE_CLOSE = 0x8
OPCODE_PING = 0x9
OPCODE_PONG = 0xA

DATA = 0
CLOSE = 2

CONTROL = 0


def frame(channel, kind, payload=b""):
    return struct.pack("<IBI", channel, kind, len(payload)) + payload


class TimedOut(Exception):
    pass


def say(line):
    print("probe: " + line, flush=True)


def die(line):
    print("probe: " + line, flush=True)
    sys.exit(1)


def ws_encode(opcode, payload):
    """Encode one masked client frame (RFC 6455 §5.3)."""
    header = bytearray([0x80 | opcode])
    length = len(payload)
    if length < 126:
        header.append(0x80 | length)
    elif length < 65536:
        header.append(0x80 | 126)
        header += struct.pack(">H", length)
    else:
        header.append(0x80 | 127)
        header += struct.pack(">Q", length)
    mask = os.urandom(4)
    header += mask
    body = bytes(b ^ mask[i & 3] for i, b in enumerate(payload))
    return bytes(header) + body


class Connection:
    def __init__(self, sock):
        self.sock = sock
        self.stream = bytearray()  # decoded emthin-frame bytes
        self.closed_reason = None

    # -- websocket ----------------------------------------------------
    def _recv_exact(self, length):
        out = bytearray()
        while len(out) < length:
            try:
                chunk = self.sock.recv(length - len(out))
            except TimeoutError:
                raise TimedOut("no frame arrived")
            except OSError as error:
                raise TimedOut("the gateway connection failed: {}".format(error))
            if not chunk:
                raise TimedOut("the gateway closed the connection")
            out.extend(chunk)
        return bytes(out)

    def _read_message(self):
        while True:
            first, second = self._recv_exact(2)
            opcode = first & 0x0F
            length = second & 0x7F
            if second & 0x80:
                raise TimedOut("the gateway masked a server frame")
            if length == 126:
                length = struct.unpack(">H", self._recv_exact(2))[0]
            elif length == 127:
                length = struct.unpack(">Q", self._recv_exact(8))[0]
            body = self._recv_exact(length)
            if opcode == OPCODE_CLOSE:
                raise TimedOut("the gateway sent a websocket CLOSE")
            if opcode == OPCODE_PING:
                self.sock.sendall(ws_encode(OPCODE_PONG, body))
                continue
            if opcode == OPCODE_PONG:
                continue
            self.stream.extend(body)
            return

    def send(self, channel, kind, payload=b""):
        self.sock.sendall(ws_encode(OPCODE_BINARY, frame(channel, kind, payload)))

    def next_frame(self, timeout):
        deadline = time.monotonic() + timeout
        while True:
            if len(self.stream) >= 9:
                channel, kind, length = struct.unpack("<IBI", self.stream[:9])
                if len(self.stream) >= 9 + length:
                    payload = bytes(self.stream[9:9 + length])
                    del self.stream[:9 + length]
                    return channel, kind, payload
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimedOut("no frame arrived")
            self.sock.settimeout(remaining)
            self._read_message()


def handshake(sock, host, port):
    key = base64.b64encode(os.urandom(16)).decode()
    request = (
        "GET {path} HTTP/1.1\r\n"
        "Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        "Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "Sec-WebSocket-Protocol: {proto}\r\n"
        "\r\n"
    ).format(path=PATH, host=host, port=port, key=key, proto=SUBPROTOCOL)
    sock.sendall(request.encode())
    head = bytearray()
    while b"\r\n\r\n" not in head:
        try:
            chunk = sock.recv(1)
        except TimeoutError:
            raise TimedOut("the upgrade response never arrived")
        except OSError as error:
            raise TimedOut("the upgrade failed: {}".format(error))
        if not chunk:
            raise TimedOut("the gateway closed during the upgrade")
        head.extend(chunk)
        if len(head) > 8 * 1024:
            raise TimedOut("the upgrade response is too long")
    lines = head.split(b"\r\n")
    if not lines[0].startswith(b"HTTP/1.1 101"):
        raise TimedOut("the upgrade was refused: "
                       + lines[0].decode("latin-1", "replace"))
    expected = base64.b64encode(
        hashlib.sha1((key + GUID).encode()).digest()).decode()
    for line in lines[1:]:
        name, _, value = line.partition(b":")
        if name.strip().lower() == b"sec-websocket-accept":
            if value.strip().decode() != expected:
                raise TimedOut("the upgrade accept key does not match")
            return
    raise TimedOut("the upgrade carried no accept key")


def rpc(conn, request_id, method, params=None, timeout=5.0):
    """Send one JSON-RPC request on the control channel and return the reply."""
    message = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        message["params"] = params
    conn.send(CONTROL, DATA, json.dumps(message, separators=(",", ":")).encode())
    while True:
        channel, kind, payload = conn.next_frame(timeout)
        if channel != CONTROL or kind != DATA:
            continue
        reply = json.loads(payload.decode("utf-8"))
        if reply.get("id") != request_id:
            continue
        return reply


def attach_control(conn, address, generation, deadline):
    """Attach as controller, retrying while the device still holds the lease."""
    attempt = 0
    while time.monotonic() < deadline:
        attempt += 1
        reply = rpc(conn, 100 + attempt, "terminal.attach", {
            "address": address,
            "generation": generation,
            "mode": "control",
        })
        if "result" in reply:
            result = reply["result"]
            return int(result["channel"]), result.get("lease_id")
        error = reply.get("error") or {}
        data = error.get("data") or {}
        if data.get("kind") != "in_use":
            raise TimedOut("control attach refused: "
                           + json.dumps(error, separators=(",", ":")))
        time.sleep(0.2)
    raise TimedOut("control attach stayed in_use for the whole window")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", required=True, help="127.0.0.1:port")
    parser.add_argument("--ca", required=True, help="PEM certificate to trust")
    parser.add_argument("--core", default="6ab09a20fd1288983dfbbac2bb349231")
    parser.add_argument("--address", default="t7")
    parser.add_argument("--generation", default="g-fixture-1")
    parser.add_argument("--marker", default="stalecontroller")
    parser.add_argument("--attach-window", type=float, default=20.0)
    parser.add_argument("--revoke-window", type=float, default=60.0)
    args = parser.parse_args()

    host, port = args.listen.rsplit(":", 1)
    port = int(port)

    context = ssl.create_default_context(cafile=args.ca)
    raw = socket.create_connection((host, port), timeout=10)
    try:
        sock = context.wrap_socket(raw, server_hostname=host)
    except (ssl.SSLError, OSError) as error:
        die("TLS setup failed: {}".format(error))

    conn = Connection(sock)
    try:
        handshake(sock, host, port)
        say("upgraded to the stream endpoint")
        sock.settimeout(10)

        reply = rpc(conn, 1, "hello", {"version": 1, "core_id": args.core})
        if "result" not in reply:
            die("hello was refused: "
                + json.dumps(reply.get("error"), separators=(",", ":")))
        say("hello accepted by core {}".format(args.core))

        try:
            channel, lease = attach_control(
                conn, args.address, args.generation,
                time.monotonic() + args.attach_window)
        except TimedOut as error:
            die(str(error))
        if not lease:
            die("the control attach named no lease")
        say("control channel {} lease {}".format(channel, lease))

        # The lane revokes this lease; the CLOSE is the revocation reaching us.
        try:
            while True:
                frame_channel, kind, payload = conn.next_frame(args.revoke_window)
                if frame_channel == channel and kind == CLOSE:
                    break
        except TimedOut as error:
            die("the lease was not revoked: {}".format(error))
        say("lease {} revoked by the operator".format(lease))

        # The whole point: bytes on a revoked controller channel. The gateway
        # must close it as not_controller and must not echo the marker.
        conn.send(channel, DATA, args.marker.encode())
        echo = b""
        reason = None
        deadline = time.monotonic() + 10.0
        try:
            while time.monotonic() < deadline:
                frame_channel, kind, payload = conn.next_frame(
                    max(deadline - time.monotonic(), 0.1))
                if frame_channel != channel:
                    continue
                if kind == CLOSE:
                    reason = json.loads(payload.decode("utf-8")).get("reason")
                    break
                if kind == DATA:
                    echo += payload
        except TimedOut:
            pass
        if reason != "not_controller":
            die("the gateway did not refuse the revoked controller"
                + ("" if reason is None else " (close reason {!r})".format(reason)))
        if args.marker.encode() in echo:
            die("the gateway echoed the revoked controller's bytes")
        say("stale controller refused not_controller, no echo")
    except TimedOut as error:
        die(str(error))
    finally:
        try:
            sock.close()
        except OSError:
            pass


if __name__ == "__main__":
    main()
