package com.emthin.pad;

import java.io.ByteArrayOutputStream;
import java.util.Random;

/**
 * RFC 6455 client framing for the gateway stream.
 *
 * <p>One emthin frame rides one binary message (docs/HERDR_GATEWAY_V1.md
 * §Frames and handshake), so this class only has to encode a masked binary
 * frame, reassemble fragmented messages, and answer control frames. Anything
 * outside that set — reserved bits, unknown opcodes, a masked server frame, a
 * control frame that is fragmented or larger than 125 bytes, a message past the
 * cap — is a protocol error, never a silent skip.
 */
final class WebSocketFrame {
    static final int OPCODE_CONTINUATION = 0x0;
    static final int OPCODE_TEXT = 0x1;
    static final int OPCODE_BINARY = 0x2;
    static final int OPCODE_CLOSE = 0x8;
    static final int OPCODE_PING = 0x9;
    static final int OPCODE_PONG = 0xa;

    static final int MAX_MESSAGE = 64 * 1024;

    private WebSocketFrame() {
    }

    interface Sink {
        void onMessage(byte[] message);

        void onPing(byte[] payload);

        /** A pong arrived (the transport has nothing to do with it). */
        default void onPong() {
        }

        /** Called when a close frame arrives; the socket is no longer usable. */
        boolean onClose(int code);
    }

    static final class ProtocolException extends Exception {
        private static final long serialVersionUID = 1L;

        ProtocolException(String message) {
            super(message);
        }
    }

    /** Encodes one masked, final binary frame. */
    static byte[] binary(byte[] payload, Random random) {
        return encode(OPCODE_BINARY, payload, random);
    }

    static byte[] pong(byte[] payload, Random random) {
        if (payload.length > 125) throw new IllegalArgumentException("control payload too large");
        return encode(OPCODE_PONG, payload, random);
    }

    static byte[] close(int code, Random random) {
        byte[] payload = code == 0
                ? new byte[0]
                : new byte[] {(byte) ((code >>> 8) & 0xff), (byte) (code & 0xff)};
        return encode(OPCODE_CLOSE, payload, random);
    }

    static byte[] encode(int opcode, byte[] payload, Random random) {
        ByteArrayOutputStream out = new ByteArrayOutputStream(payload.length + 14);
        out.write(0x80 | (opcode & 0x0f));
        int length = payload.length;
        if (length <= 125) {
            out.write(0x80 | length);
        } else if (length <= 0xffff) {
            out.write(0x80 | 126);
            out.write((length >>> 8) & 0xff);
            out.write(length & 0xff);
        } else {
            out.write(0x80 | 127);
            for (int shift = 56; shift >= 0; shift -= 8) {
                out.write((int) ((((long) length) >>> shift) & 0xff));
            }
        }
        byte[] mask = new byte[4];
        random.nextBytes(mask);
        out.write(mask, 0, 4);
        for (int i = 0; i < length; i++) {
            out.write(payload[i] ^ mask[i & 3]);
        }
        return out.toByteArray();
    }

    /**
     * Incremental reader: feed it whatever the socket produced and it hands back
     * whole messages. Keeps one reassembly buffer, so a fragmented message never
     * gets parsed before its last continuation arrives.
     */
    static final class Reader {
        private byte[] buffer = new byte[0];
        private int buffered;
        private final ByteArrayOutputStream message = new ByteArrayOutputStream();
        private final boolean readsServerFrames;
        private boolean inMessage;

        /** Reads server-to-client frames: unmasked, masked is a protocol error. */
        Reader() {
            this(true);
        }

        /**
         * @param readsServerFrames true for the client side of the stream, false
         *     for a server reading client frames (which must be masked).
         */
        Reader(boolean readsServerFrames) {
            this.readsServerFrames = readsServerFrames;
        }

        void feed(byte[] data, int offset, int length, Sink sink) throws ProtocolException {
            if (buffered + length > buffer.length) {
                int size = Math.max(buffer.length * 2, buffered + length);
                if (size > MAX_MESSAGE + 14 + 1024) size = MAX_MESSAGE + 14 + 1024;
                if (buffered + length > size) throw new ProtocolException("frame header buffer overflow");
                byte[] grown = new byte[size];
                System.arraycopy(buffer, 0, grown, 0, buffered);
                buffer = grown;
            }
            System.arraycopy(data, offset, buffer, buffered, length);
            buffered += length;

            int at = 0;
            while (true) {
                int consumed = parse(at, sink);
                if (consumed < 0) break;
                at += consumed;
            }
            if (at > 0) {
                System.arraycopy(buffer, at, buffer, 0, buffered - at);
                buffered -= at;
            }
        }

        /** Returns bytes consumed for one complete frame, or -1 when short. */
        private int parse(int at, Sink sink) throws ProtocolException {
            int available = buffered - at;
            if (available < 2) return -1;
            int first = buffer[at] & 0xff;
            int second = buffer[at + 1] & 0xff;
            boolean fin = (first & 0x80) != 0;
            int rsv = first & 0x70;
            int opcode = first & 0x0f;
            boolean masked = (second & 0x80) != 0;
            long length = second & 0x7f;
            int header = 2;
            if (rsv != 0) throw new ProtocolException("reserved bits set");
            if (readsServerFrames && masked) throw new ProtocolException("server frame is masked");
            if (!readsServerFrames && !masked) throw new ProtocolException("client frame is not masked");
            if (length == 126) {
                if (available < 4) return -1;
                length = ((buffer[at + 2] & 0xff) << 8) | (buffer[at + 3] & 0xff);
                header = 4;
            } else if (length == 127) {
                if (available < 10) return -1;
                length = 0;
                for (int i = 0; i < 8; i++) {
                    length = (length << 8) | (buffer[at + 2 + i] & 0xff);
                }
                header = 10;
                if (length < 0) throw new ProtocolException("negative length");
            }
            boolean control = opcode >= 0x8;
            if (control) {
                if (!fin) throw new ProtocolException("fragmented control frame");
                if (length > 125) throw new ProtocolException("control frame too large");
            } else if (opcode != OPCODE_CONTINUATION && opcode != OPCODE_BINARY && opcode != OPCODE_TEXT) {
                throw new ProtocolException("unknown opcode");
            }
            if (length > MAX_MESSAGE) throw new ProtocolException("message too large");
            byte[] mask = null;
            if (masked) {
                if (available < header + 4) return -1;
                mask = new byte[] {buffer[at + header], buffer[at + header + 1],
                        buffer[at + header + 2], buffer[at + header + 3]};
                header += 4;
            }
            if (available < header + (int) length) return -1;

            int payloadAt = at + header;
            int size = (int) length;
            if (mask != null) {
                for (int i = 0; i < size; i++) buffer[payloadAt + i] ^= mask[i & 3];
            }
            if (control) {
                byte[] payload = new byte[size];
                System.arraycopy(buffer, payloadAt, payload, 0, size);
                if (opcode == OPCODE_CLOSE) {
                    int code = size >= 2
                            ? ((payload[0] & 0xff) << 8) | (payload[1] & 0xff)
                            : 0;
                    sink.onClose(code);
                } else if (opcode == OPCODE_PING) {
                    sink.onPing(payload);
                } else if (opcode == OPCODE_PONG) {
                    sink.onPong();
                }
                return header + size;
            }
            if (opcode == OPCODE_TEXT) throw new ProtocolException("text messages are not part of this protocol");
            if (opcode == OPCODE_BINARY) {
                if (inMessage) throw new ProtocolException("new message before continuation");
                message.reset();
                inMessage = true;
            } else if (!inMessage) {
                throw new ProtocolException("continuation without a message");
            }
            if (size > MAX_MESSAGE - message.size()) throw new ProtocolException("message too large");
            message.write(buffer, payloadAt, size);
            if (fin) {
                inMessage = false;
                sink.onMessage(message.toByteArray());
                message.reset();
            }
            return header + size;
        }
    }
}
