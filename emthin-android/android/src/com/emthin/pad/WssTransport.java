package com.emthin.pad;

import java.io.IOException;
import java.io.InputStream;
import java.net.InetSocketAddress;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.ArrayDeque;
import java.util.Random;

import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLParameters;
import javax.net.ssl.SSLSocket;

/**
 * The paired gateway's stream: TLS, then one RFC 6455 upgrade on
 * {@code /v1/stream}, then emthin frames as binary messages.
 *
 * <p>Hostname and certificate checks are the platform's — the socket is created
 * from the caller's {@link SSLContext} with the {@code HTTPS} endpoint
 * identification algorithm, so a wrong name or an untrusted chain fails the
 * handshake instead of reaching the frame layer. A silent peer is probed with a
 * ping every {@link #IDLE_PING_MS}; two consecutive silent periods end the
 * connection.
 */
final class WssTransport implements GatewayTransport {
    static final int IDLE_PING_MS = 20000;

    private final SSLSocket socket;
    private final InputStream raw;
    private final WebSocketFrame.Reader frames = new WebSocketFrame.Reader();
    private final Random random = new SecureRandom();
    private final Object writeLock = new Object();
    private final InputStream input;
    private final ArrayDeque<byte[]> messages = new ArrayDeque<byte[]>();
    private byte[] pending = new byte[0];
    private int pendingAt;
    private int silentPeriods;
    private boolean closed;
    private boolean shut;

    private WssTransport(SSLSocket socket) throws IOException {
        this.socket = socket;
        this.raw = socket.getInputStream();
        this.input = new MessageStream();
    }

    static WssTransport open(String host, int port, SSLContext context,
            int connectTimeoutMs, int readTimeoutMs) throws IOException {
        SSLSocket socket = (SSLSocket) context.getSocketFactory().createSocket();
        try {
            socket.connect(new InetSocketAddress(host, port), connectTimeoutMs);
            socket.setSoTimeout(readTimeoutMs);
            socket.setTcpNoDelay(true);
            SSLParameters parameters = socket.getSSLParameters();
            parameters.setEndpointIdentificationAlgorithm("HTTPS");
            socket.setSSLParameters(parameters);
            socket.startHandshake();
        } catch (IOException e) {
            socket.close();
            throw e;
        }
        WssTransport transport = new WssTransport(socket);
        try {
            transport.upgrade(host);
        } catch (IOException e) {
            transport.close();
            throw e;
        }
        return transport;
    }

    /** A connector for a stream that is already known to be at this host. */
    static GatewayTransport.Connector connector(final String host, final int port,
            final SSLContext context, final int connectTimeoutMs, final int readTimeoutMs) {
        return new GatewayTransport.Connector() {
            @Override public GatewayTransport open() throws IOException {
                return WssTransport.open(host, port, context, connectTimeoutMs, readTimeoutMs);
            }
        };
    }

    @Override public InputStream input() {
        return input;
    }

    @Override public void send(byte[] packet) throws IOException {
        write(WebSocketFrame.binary(packet, random));
    }

    @Override public void close() {
        if (shut) return;
        shut = true;
        closed = true;
        try {
            write(WebSocketFrame.close(1000, random));
        } catch (IOException ignored) {
            // Teardown is best effort.
        }
        try {
            socket.close();
        } catch (IOException ignored) {
            // Best effort teardown.
        }
    }

    private void upgrade(String host) throws IOException {
        String key = WebSocketHandshake.newKey(random);
        writeRaw(WebSocketHandshake.request(host, "/v1/stream", key)
                .getBytes(StandardCharsets.US_ASCII));
        byte[] head = new byte[WebSocketHandshake.MAX_HEAD];
        int length = 0;
        int end;
        while ((end = WebSocketHandshake.headEnd(head, length)) < 0) {
            if (length == head.length) throw new IOException("handshake head too long");
            int read = raw.read(head, length, head.length - length);
            if (read < 0) throw new IOException("closed during handshake");
            length += read;
        }
        WebSocketHandshake.validate(new String(head, 0, end, StandardCharsets.ISO_8859_1), key);
    }

    private void write(byte[] frame) throws IOException {
        synchronized (writeLock) {
            writeRaw(frame);
        }
    }

    private void writeRaw(byte[] bytes) throws IOException {
        socket.getOutputStream().write(bytes);
        socket.getOutputStream().flush();
    }

    /** Delivers one whole message, or returns null while a ping/pong is due. */
    private void pump() throws IOException {
        byte[] chunk = new byte[8192];
        int read;
        try {
            read = raw.read(chunk);
            silentPeriods = 0;
        } catch (SocketTimeoutException e) {
            if (++silentPeriods > 1) throw new IOException("unanswered ping");
            write(WebSocketFrame.encode(WebSocketFrame.OPCODE_PING, new byte[0], random));
            return;
        }
        if (read < 0) {
            // A peer that just drops the socket is an end of stream, not an error.
            closed = true;
            return;
        }
        try {
            frames.feed(chunk, 0, read, sink);
        } catch (WebSocketFrame.ProtocolException e) {
            throw new IOException("websocket protocol error: " + e.getMessage());
        }
    }

    private final WebSocketFrame.Sink sink = new WebSocketFrame.Sink() {
        @Override public void onMessage(byte[] message) {
            messages.add(message);
        }

        @Override public void onPing(byte[] payload) {
            try {
                write(WebSocketFrame.pong(payload, random));
            } catch (IOException e) {
                closed = true;
            }
        }

        @Override public boolean onClose(int code) {
            closed = true;
            return false;
        }
    };

    private final class MessageStream extends InputStream {
        @Override public int read() throws IOException {
            byte[] one = new byte[1];
            int count = read(one, 0, 1);
            return count < 0 ? -1 : one[0] & 0xff;
        }

        @Override public int read(byte[] out, int offset, int length) throws IOException {
            if (length == 0) return 0;
            while (pendingAt == pending.length) {
                byte[] next = messages.poll();
                if (next != null) {
                    pending = next;
                    pendingAt = 0;
                    continue;
                }
                if (closed) return -1;
                pump();
            }
            int count = Math.min(length, pending.length - pendingAt);
            System.arraycopy(pending, pendingAt, out, offset, count);
            pendingAt += count;
            return count;
        }
    }
}
