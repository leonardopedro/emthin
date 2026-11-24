#!/usr/bin/env python3
"""Scripted emthin gateway v1 peer for lease and revocation checks.

This is a fixture, not a production gateway.  It speaks the real frame
envelope from docs/HERDR_GATEWAY_V1.md over a plain TCP socket so an Android
device can reach it through `adb reverse`.  A production deployment uses the
TLS/WebSocket listener and a Core upstream; this peer deliberately has a tiny
scripted Core with one terminal.

Frames are ``channel:u32LE | kind:u8 | length:u32LE | payload``.  The control
channel is zero and carries JSON-RPC only.  A non-zero channel is one attach.
A control attach owns the single controller lease; an operator command on the
admin socket revokes it with a CLOSE frame.
"""

import argparse
import json
import socket
import struct
import threading
import time

DATA = 0
CREDIT = 1
CLOSE = 2
RESIZE = 3
FOCUS = 4
FACTS = 5
AUTOMATIC_INPUT = 6

CORE_ID = "6ab09a20fd1288983dfbbac2bb349231"
MAX_FRAME = 64 * 1024


def frame(channel, kind, payload=b""):
    return struct.pack("<IBI", channel, kind, len(payload)) + payload


def recv_exact(sock, length):
    out = bytearray()
    while len(out) < length:
        chunk = sock.recv(length - len(out))
        if not chunk:
            raise ConnectionError("peer closed")
        out.extend(chunk)
    return bytes(out)


def read_frame(sock):
    header = recv_exact(sock, 9)
    channel, kind, length = struct.unpack("<IBI", header)
    if length > MAX_FRAME:
        raise ValueError("frame payload too large")
    return channel, kind, recv_exact(sock, length)


class Peer:
    def __init__(self, server, sock, address):
        self.server = server
        self.sock = sock
        self.address = address
        # Which Core ID this connection said `hello` with; one process can
        # answer for more than one, so the catalogue is chosen per peer.
        self.core_id = None
        self.lock = threading.Lock()
        self.channels = {}
        self.next_channel = 1
        self.alive = True

    def send(self, channel, kind, payload=b""):
        if not self.alive:
            return
        packet = frame(channel, kind, payload)
        try:
            with self.lock:
                self.sock.sendall(packet)
        except OSError:
            self.alive = False

    def send_json(self, value):
        self.send(0, DATA, json.dumps(value, separators=(",", ":")).encode())

    def result(self, request_id, result):
        self.send_json({"jsonrpc": "2.0", "id": request_id, "result": result})

    def error(self, request_id, code, message, kind=None):
        error = {"code": code, "message": message}
        if kind:
            error["data"] = {"kind": kind}
        self.send_json({"jsonrpc": "2.0", "id": request_id, "error": error})

    def allocate(self):
        channel = self.next_channel
        self.next_channel += 1
        return channel

    def close_channel(self, channel, reason):
        self.channels.pop(channel, None)
        self.send(channel, CLOSE,
                  json.dumps({"reason": reason}, separators=(",", ":")).encode())

    def revoke(self, channel, reason="revoked"):
        self.channels.pop(channel, None)
        self.send(channel, CLOSE,
                  json.dumps({"reason": reason}, separators=(",", ":")).encode())

    def close(self):
        self.alive = False
        try:
            self.sock.close()
        except OSError:
            pass


class Gateway:
    def __init__(self, core_id=CORE_ID, second_core_id=None):
        self.core_id = core_id
        # A lane that pairs two hosts gets one process answering for both, so
        # the picker's merge sees two real catalogues rather than two copies of
        # one. The second identity is absent unless asked for, which keeps a
        # Core-ID mismatch a 409 for every other step.
        self.second_core_id = second_core_id
        self.cores = {core_id} if not second_core_id else {core_id, second_core_id}
        self.lock = threading.Lock()
        self.active = None  # (peer, channel, lease)
        self.peers = set()
        self.input_log = []
        self.handoffs = 0

    # -- admin ---------------------------------------------------------
    def status(self):
        with self.lock:
            active = None
            if self.active:
                peer, channel, lease = self.active
                active = {
                    "channel": channel,
                    "lease_id": lease,
                    "peer": peer.address[0] + ":" + str(peer.address[1]),
                }
            return {
                "active": active,
                "attachments": sum(len(peer.channels) for peer in self.peers),
                "input": "".join(self.input_log[-1000:]),
                "handoffs": self.handoffs,
            }

    def revoke(self):
        with self.lock:
            active = self.active
            self.active = None
        if not active:
            return {"state": "no_active_lease"}
        peer, channel, _lease = active
        peer.revoke(channel, "revoked")
        return {"state": "revoked"}

    # -- client protocol ----------------------------------------------
    def serve(self, sock, address):
        peer = Peer(self, sock, address)
        with self.lock:
            self.peers.add(peer)
        try:
            self._read_loop(peer)
        except (ConnectionError, OSError, ValueError):
            pass
        finally:
            with self.lock:
                if self.active and self.active[0] is peer:
                    self.active = None
                self.peers.discard(peer)
            peer.close()

    def _read_loop(self, peer):
        hello_seen = False
        while peer.alive:
            channel, kind, payload = read_frame(peer.sock)
            if channel == 0:
                if kind != DATA:
                    raise ValueError("control channel got a stream frame")
                self._control(peer, payload, hello_seen)
                hello_seen = True
                continue
            if kind == DATA:
                self._stream_data(peer, channel, payload)
            elif kind == CREDIT:
                if len(payload) != 4:
                    raise ValueError("CREDIT payload must be four bytes")
            elif kind == CLOSE:
                with self.lock:
                    if self.active and self.active[0] is peer and self.active[1] == channel:
                        self.active = None
                    peer.channels.pop(channel, None)
                if len(payload) <= 1024:
                    peer.close_channel(channel, "detached")
            elif kind in (RESIZE, FOCUS, FACTS, AUTOMATIC_INPUT):
                if not self._is_controller(peer, channel):
                    peer.close_channel(channel, "not_controller")
            else:
                raise ValueError("unknown frame kind")

    def _is_controller(self, peer, channel):
        with self.lock:
            return bool(self.active and self.active[0] is peer
                        and self.active[1] == channel)

    def _control(self, peer, payload, hello_seen):
        try:
            message = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            peer.error(None, -32700, "parse error")
            return
        if not isinstance(message, dict) or message.get("jsonrpc") != "2.0":
            peer.error(None, -32600, "invalid request")
            return
        request_id = message.get("id")
        method = message.get("method")
        params = message.get("params")
        if not hello_seen and method != "hello":
            peer.error(request_id, -32000, "hello required", "core_offline")
            return
        if method == "hello":
            if not isinstance(params, dict):
                peer.error(request_id, -32602, "invalid params")
                return
            version = params.get("version")
            core_id = params.get("core_id")
            if version != 1:
                peer.error(request_id, -32602, "unsupported version")
                return
            if core_id not in self.cores:
                peer.error(request_id, -32000, "core mismatch", "core_mismatch")
                return
            peer.core_id = core_id
            peer.result(request_id, {
                "version": 1,
                "core_id": core_id,
                "features": ["resize_notifications", "automatic_input"],
                "methods": ["hello", "sessions.list", "terminal.attach",
                            "control.request", "control.respond"],
            })
            return
        if method == "terminal.attach":
            self._attach(peer, request_id, params)
            return
        if method == "control.request":
            # One scripted controller is enough for this peer.  The handoff
            # response is still exercised by the admin revoke path.
            peer.error(request_id, -32000, "unavailable", "not_found")
            return
        if method == "sessions.list":
            second = self.second_core_id is not None and peer.core_id == self.second_core_id
            peer.result(request_id, {
                "revision": "r-fixture-2" if second else "r-fixture-1",
                "rows": [{
                    "address": "t9" if second else "t7",
                    "generation": "g-fixture-2" if second else "g-fixture-1",
                    "label": "Loft fixture" if second else "Lease fixture",
                    "grid": {"cols": 80, "rows": 24},
                    "lifecycle": "live",
                    "status": None,
                    "actions": ["watch", "control"],
                }],
                "next_cursor": None,
            })
            return
        peer.error(request_id, -32601, "method not found")

    def _attach(self, peer, request_id, params):
        if not isinstance(params, dict):
            peer.error(request_id, -32602, "invalid params")
            return
        mode = params.get("mode")
        address = params.get("address")
        generation = params.get("generation")
        if mode not in ("watch", "control"):
            peer.error(request_id, -32602, "invalid mode")
            return
        if not isinstance(address, str) or not address or len(address.encode()) > 256:
            peer.error(request_id, -32602, "invalid address")
            return
        if not isinstance(generation, str) or not generation:
            peer.error(request_id, -32602, "invalid generation")
            return

        with self.lock:
            if mode == "control" and self.active is not None and self.active[0] is not peer:
                peer.error(request_id, -32000, "in use", "in_use")
                return
            channel = peer.allocate()
            lease = "lease-" + str(channel) if mode == "control" else None
            peer.channels[channel] = mode
            if mode == "control":
                self.active = (peer, channel, lease)
        # The contract commits the attach response before channel data.
        peer.result(request_id, {
            "channel": channel,
            "mode": mode,
            "generation": generation,
            "grid": {"cols": 80, "rows": 24},
            "resize_notifications": True,
            "automatic_input": False,
            "lease_id": lease,
        })
        peer.send(channel, RESIZE, struct.pack("<HH", 80, 24))
        banner = ("\x1b[2J\x1b[Hemthin gateway lease\r\n"
                  + address + " " + mode + "\r\n$ ")
        peer.send(channel, DATA, banner.encode("utf-8"))

    def _stream_data(self, peer, channel, payload):
        if len(payload) > 16 * 1024:
            raise ValueError("stream DATA too large")
        if not self._is_controller(peer, channel):
            peer.close_channel(channel, "not_controller")
            return
        text = payload.decode("utf-8", "replace")
        with self.lock:
            self.input_log.append(text)
        # Echo proves the controller's bytes crossed the real frame path.
        echo = ("\r\necho: " + text + "\r\n$ ").encode("utf-8")
        peer.send(channel, DATA, echo)


def admin_loop(gateway, listener):
    while True:
        try:
            conn, _ = listener.accept()
        except OSError:
            return
        threading.Thread(target=admin_client, args=(gateway, conn),
                         daemon=True).start()


def admin_client(gateway, conn):
    with conn:
        data = b""
        while b"\n" not in data and len(data) < 1024:
            chunk = conn.recv(1024)
            if not chunk:
                return
            data += chunk
        command = data.split(b"\n", 1)[0].decode("ascii", "replace").strip()
        if command == "revoke":
            reply = gateway.revoke()
        elif command == "status":
            reply = gateway.status()
        elif command.startswith("pair "):
            # Pairing lives on the TLS fixture (`wss_fixture.py`); a plain-TCP
            # gateway has none, and says so instead of pretending to approve.
            pairing = getattr(gateway, "pairing", None)
            if pairing is None:
                reply = {"error": "pairing not enabled"}
            else:
                reply = pairing.admin(command.split(" ", 1)[1].strip())
        else:
            reply = {"error": "unknown command"}
        conn.sendall((json.dumps(reply, separators=(",", ":")) + "\n").encode())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", default="127.0.0.1:47654")
    parser.add_argument("--admin", default="127.0.0.1:47655")
    args = parser.parse_args()
    listen_host, listen_port = args.listen.rsplit(":", 1)
    admin_host, admin_port = args.admin.rsplit(":", 1)

    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind((listen_host, int(listen_port)))
    listener.listen(8)
    admin = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    admin.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    admin.bind((admin_host, int(admin_port)))
    admin.listen(4)

    gateway = Gateway()
    threading.Thread(target=admin_loop, args=(gateway, admin), daemon=True).start()
    print("lease fixture listening {} admin {}".format(args.listen, args.admin),
          flush=True)
    try:
        while True:
            conn, address = listener.accept()
            threading.Thread(target=gateway.serve, args=(conn, address),
                             daemon=True).start()
    except KeyboardInterrupt:
        pass
    finally:
        listener.close()
        admin.close()


if __name__ == "__main__":
    main()
