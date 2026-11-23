package com.emthin.pad;

import java.io.ByteArrayOutputStream;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Random;

import javax.net.ssl.KeyManagerFactory;
import javax.net.ssl.SSLContext;
import javax.net.ssl.SSLServerSocket;
import javax.net.ssl.SSLSocket;
import javax.net.ssl.TrustManagerFactory;

/**
 * Host tests for the stream transport: framing, the upgrade validator, and a
 * real TLS round trip against an in-process server.
 *
 * <p>Usage: {@code WebSocketTest <keystore> <kwrong-keystore>}, both PKCS12 with
 * password {@code changeit}. The first certificate names {@code localhost}; the
 * second names a host we never ask for, for the endpoint-identification test.
 */
public final class WebSocketTest {
    private static final String PASSWORD = "changeit";
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static class Collect implements WebSocketFrame.Sink {
        final List<byte[]> messages = new ArrayList<byte[]>();
        boolean pong;
        int pings;
        int closeCode = -1;

        @Override public void onMessage(byte[] message) {
            messages.add(message);
        }

        @Override public void onPing(byte[] payload) {
            pings++;
        }

        @Override public void onPong() {
            pong = true;
        }

        @Override public boolean onClose(int code) {
            closeCode = code;
            return false;
        }
    }

    /** What the test server observed on its side of the stream. */
    private static final class Recorded {
        volatile byte[] message;
        volatile boolean pong;
        volatile String failure;
    }

    public static void main(String[] args) throws Exception {
        frames();
        handshakes();
        tls(args[0], args[1]);
        System.out.println("WebSocketTest: " + checks + " checks");
    }

    private static void expectProtocolError(byte[] frame, String why) {
        try {
            new WebSocketFrame.Reader().feed(frame, 0, frame.length, new Collect());
        } catch (WebSocketFrame.ProtocolException expected) {
            checks++;
            return;
        }
        throw new AssertionError("accepted " + why);
    }

    private static void frames() throws WebSocketFrame.ProtocolException {
        Random random = new Random(7);
        byte[] payload = "hello".getBytes(StandardCharsets.UTF_8);
        byte[] encoded = WebSocketFrame.binary(payload, random);
        check((encoded[0] & 0xff) == 0x82, "binary opcode, final bit");
        check((encoded[1] & 0x80) != 0, "client frames are masked");
        check((encoded[1] & 0x7f) == payload.length, "short length is inline");
        for (int i = 0; i < payload.length; i++) {
            check((byte) (encoded[6 + i] ^ encoded[2 + (i & 3)]) == payload[i], "mask round trip");
        }
        check(WebSocketFrame.encode(WebSocketFrame.OPCODE_BINARY, new byte[70000], random).length
                == 2 + 8 + 4 + 70000, "64-bit extended length");

        Collect server = new Collect();
        byte[] frame = new byte[4 + 130];
        frame[0] = (byte) 0x82;
        frame[1] = 126;
        frame[3] = (byte) 130;
        new WebSocketFrame.Reader().feed(frame, 0, frame.length, server);
        check(server.messages.size() == 1 && server.messages.get(0).length == 130,
                "16-bit extended length");

        Collect sink = new Collect();
        WebSocketFrame.Reader reader = new WebSocketFrame.Reader();
        byte[] first = fragment(0x02, false, "frag".getBytes(StandardCharsets.UTF_8));
        byte[] second = fragment(0x00, true, "mented".getBytes(StandardCharsets.UTF_8));
        reader.feed(first, 0, first.length, sink);
        check(sink.messages.isEmpty(), "a fragment is not yet a message");
        reader.feed(second, 0, second.length, sink);
        check(sink.messages.size() == 1
                && "fragmented".equals(new String(sink.messages.get(0), StandardCharsets.UTF_8)),
                "fragments reassemble");

        Collect split = new Collect();
        WebSocketFrame.Reader byteWise = new WebSocketFrame.Reader();
        byte[] whole = new byte[first.length + second.length];
        System.arraycopy(first, 0, whole, 0, first.length);
        System.arraycopy(second, 0, whole, first.length, second.length);
        for (byte b : whole) byteWise.feed(new byte[] {b}, 0, 1, split);
        check(split.messages.size() == 1, "one byte at a time still reassembles");

        Collect pinger = new Collect();
        byte[] ping = new byte[] {(byte) 0x89, 1, 42};
        new WebSocketFrame.Reader().feed(ping, 0, ping.length, pinger);
        check(pinger.pings == 1, "ping reaches the sink");

        Collect closer = new Collect();
        byte[] close = new byte[] {(byte) 0x88, 2, 0x03, (byte) 0xe8};
        new WebSocketFrame.Reader().feed(close, 0, close.length, closer);
        check(closer.closeCode == 1000, "close code");

        expectProtocolError(new byte[] {(byte) 0xc2, 0}, "reserved bits");
        expectProtocolError(new byte[] {(byte) 0x82, (byte) 0x80, 0, 0, 0, 0}, "masked server frame");
        expectProtocolError(new byte[] {(byte) 0x81, 1, 'x'}, "text message");
        expectProtocolError(new byte[] {(byte) 0x08, 0}, "fragmented control frame");
        expectProtocolError(new byte[] {(byte) 0x89, 126, 0x01, 0x00}, "oversized control frame");
        expectProtocolError(new byte[] {(byte) 0x83, 0}, "unknown opcode");
        expectProtocolError(new byte[] {(byte) 0x80, 1, 'x'}, "orphan continuation");
        try {
            WebSocketFrame.pong(new byte[126], random);
            throw new AssertionError("accepted an oversized pong payload");
        } catch (IllegalArgumentException expected) {
            checks++;
        }
    }

    private static byte[] fragment(int firstByte, boolean fin, byte[] payload) {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        out.write(firstByte | (fin ? 0x80 : 0));
        out.write(payload.length);
        out.write(payload, 0, payload.length);
        return out.toByteArray();
    }

    private static void handshakes() throws Exception {
        String key = WebSocketHandshake.newKey(new Random(3));
        check(key.length() == 24, "16-byte nonce as base64");
        check("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".equals(
                WebSocketHandshake.acceptFor("dGhlIHNhbXBsZSBub25jZQ==")), "RFC 6455 accept example");
        String request = WebSocketHandshake.request("fleet.leo.ts.net", "/v1/stream", key);
        check(request.startsWith("GET /v1/stream HTTP/1.1\r\n"), "request line");
        check(request.contains("Host: fleet.leo.ts.net\r\n"), "host header");
        check(request.contains("Sec-WebSocket-Protocol: emthin-terminal.v1\r\n"),
                "subprotocol offered");
        check(!request.contains("Origin:"), "no origin header");

        String accept = WebSocketHandshake.acceptFor(key);
        try {
            WebSocketHandshake.validate(response(accept, WebSocketHandshake.SUBPROTOCOL), key);
            checks++;
        } catch (IOException e) {
            throw new AssertionError("rejected a valid upgrade: " + e.getMessage());
        }
        bad("HTTP/1.1 200 OK\r\n\r\n", "200 instead of 101", key);
        bad("HTTP/1.1 302 Found\r\n" + response(accept, WebSocketHandshake.SUBPROTOCOL),
                "redirect", key);
        bad(response(accept, WebSocketHandshake.SUBPROTOCOL)
                .replace("Upgrade: websocket\r\n", ""), "missing upgrade header", key);
        bad(response(accept, WebSocketHandshake.SUBPROTOCOL)
                .replace("Connection: Upgrade\r\n", "Connection: keep-alive\r\n"),
                "missing connection token", key);
        bad(response("wrong", WebSocketHandshake.SUBPROTOCOL), "bad accept", key);
        bad(response(accept, "other.v1"), "wrong subprotocol", key);
        bad(response(accept, null), "no subprotocol", key);
        bad(response(accept, WebSocketHandshake.SUBPROTOCOL)
                + "Sec-WebSocket-Extensions: permessage-deflate\r\n\r\n",
                "unsolicited extension", key);
        bad("HTTP/2 101 Switching\r\n\r\n", "http/2 status line", key);
    }

    private static String response(String accept, String protocol) {
        StringBuilder out = new StringBuilder("HTTP/1.1 101 Switching Protocols\r\n");
        out.append("Upgrade: websocket\r\nConnection: Upgrade\r\n");
        out.append("Sec-WebSocket-Accept: ").append(accept).append("\r\n");
        if (protocol != null) out.append("Sec-WebSocket-Protocol: ").append(protocol).append("\r\n");
        out.append("\r\n");
        return out.toString();
    }

    private static void bad(String head, String why, String key) {
        try {
            WebSocketHandshake.validate(head, key);
        } catch (IOException expected) {
            checks++;
            return;
        }
        throw new AssertionError("accepted " + why);
    }

    private static SSLContext context(String path, boolean server) throws Exception {
        KeyStore store = KeyStore.getInstance("PKCS12");
        InputStream in = new FileInputStream(path);
        try {
            store.load(in, PASSWORD.toCharArray());
        } finally {
            in.close();
        }
        SSLContext context = SSLContext.getInstance("TLS");
        if (server) {
            KeyManagerFactory keys = KeyManagerFactory.getInstance(
                    KeyManagerFactory.getDefaultAlgorithm());
            keys.init(store, PASSWORD.toCharArray());
            context.init(keys.getKeyManagers(), null, null);
        } else {
            TrustManagerFactory trust = TrustManagerFactory.getInstance(
                    TrustManagerFactory.getDefaultAlgorithm());
            trust.init(store);
            context.init(null, trust.getTrustManagers(), null);
        }
        return context;
    }

    private static void tls(String goodStore, String wrongNameStore) throws Exception {
        SSLContext server = context(goodStore, true);
        SSLContext client = context(goodStore, false);

        SSLServerSocket listener = (SSLServerSocket) server.getServerSocketFactory()
                .createServerSocket(0);
        listener.setSoTimeout(10000);
        Recorded recorded = new Recorded();
        Thread thread = new Thread(new StreamServer(listener, recorded));
        thread.start();

        WssTransport transport = WssTransport.open("localhost", listener.getLocalPort(), client,
                5000, 5000);
        try {
            byte[] message = new byte[64];
            int total = 0;
            int read;
            while (total < "streamed hello".length()
                    && (read = transport.input().read(message, total, message.length - total)) > 0) {
                total += read;
            }
            check("streamed hello".equals(new String(message, 0, total, StandardCharsets.UTF_8)),
                    "fragmented server message arrives");
            transport.send("ping-reply".getBytes(StandardCharsets.UTF_8));
            check(transport.input().read() == -1, "server close ends the stream");
        } finally {
            transport.close();
            listener.close();
        }
        thread.join(5000);
        check(recorded.failure == null, "server thread: " + recorded.failure);
        check(recorded.message != null
                && "ping-reply".equals(new String(recorded.message, StandardCharsets.UTF_8)),
                "server received the client's message");
        check(recorded.pong, "server received the pong for its ping");

        // Fail closed: a TLS peer that answers 200 instead of upgrading.
        Expectation http200 = serveOnce(server, "HTTP/1.1 200 OK\r\n\r\n");
        expectOpenFailure(client, http200.port, "http 200 instead of a 101");
        http200.stop();

        // Fail closed: a plain TCP peer, i.e. no TLS at all.
        Expectation silent = servePlainOnce("HTTP/1.1 200 OK\r\n\r\n");
        IOException noise = expectOpenFailure(client, silent.port, "not a websocket server");
        check(noise instanceof javax.net.ssl.SSLException,
                "a plain peer fails at the TLS layer: " + noise);
        silent.stop();

        // Fail closed: an unknown certificate authority.
        SSLServerSocket untrusted = (SSLServerSocket) context(wrongNameStore, true)
                .getServerSocketFactory().createServerSocket(0);
        Expectation unknown = acceptOnly(untrusted);
        IOException untrustedError = expectOpenFailure(SSLContext.getDefault(), unknown.port,
                "unknown certificate");
        check(untrustedError instanceof javax.net.ssl.SSLHandshakeException,
                "unknown certificate fails the handshake, not the frame layer");
        unknown.stop();

        // Fail closed: a trusted certificate for a name we never asked for.
        SSLServerSocket wrongName = (SSLServerSocket) context(wrongNameStore, true)
                .getServerSocketFactory().createServerSocket(0);
        Expectation mismatch = acceptOnly(wrongName);
        IOException nameError = expectOpenFailure(context(wrongNameStore, false), mismatch.port,
                "certificate names another host");
        check(nameError instanceof javax.net.ssl.SSLHandshakeException
                && nameError.getMessage() != null
                && nameError.getMessage().contains("subject alternative"),
                "hostname mismatch is caught by endpoint identification: " + nameError);
        mismatch.stop();
    }

    /** A one-shot listener plus its accept thread. */
    private static final class Expectation {
        final int port;
        private final ServerSocket socket;
        private final Thread thread;

        Expectation(ServerSocket socket, Thread thread) {
            this.socket = socket;
            this.port = socket.getLocalPort();
            this.thread = thread;
        }

        void stop() {
            try {
                socket.close();
            } catch (IOException ignored) {
                // Best effort teardown.
            }
            try {
                thread.join(5000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
    }

    /** Accepts one connection and writes {@code response}, or nothing at all. */
    private static Expectation serveOnce(SSLContext context, final String response) {
        try {
            final SSLServerSocket listener = (SSLServerSocket) context.getServerSocketFactory()
                    .createServerSocket(0);
            listener.setSoTimeout(10000);
            Thread thread = new Thread(new Runnable() {
                @Override public void run() {
                    try {
                        SSLSocket peer = (SSLSocket) listener.accept();
                        peer.setSoTimeout(5000);
                        readHead(peer.getInputStream());
                        if (response != null) {
                            peer.getOutputStream().write(response.getBytes(StandardCharsets.US_ASCII));
                            peer.getOutputStream().flush();
                        }
                        peer.close();
                    } catch (IOException ignored) {
                        // The assertion reports the outcome.
                    }
                }
            });
            thread.start();
            return new Expectation(listener, thread);
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
    }

    /** Accepts one plain-TCP connection and writes {@code response}. */
    private static Expectation servePlainOnce(final String response) {
        try {
            final ServerSocket listener = new ServerSocket(0);
            listener.setSoTimeout(10000);
            Thread thread = new Thread(new Runnable() {
                @Override public void run() {
                    try {
                        java.net.Socket peer = listener.accept();
                        peer.setSoTimeout(5000);
                        peer.getOutputStream().write(response.getBytes(StandardCharsets.US_ASCII));
                        peer.getOutputStream().flush();
                        peer.close();
                    } catch (IOException ignored) {
                        // The assertion reports the outcome.
                    }
                }
            });
            thread.start();
            return new Expectation(listener, thread);
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
    }

    /**
     * Accepts one connection and holds it open. Closing here instead would abort
     * the handshake from the server side, and the client would report the broken
     * connection rather than its own certificate verdict.
     */
    private static Expectation acceptOnly(SSLServerSocket listener) {
        try {
            listener.setSoTimeout(10000);
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
        final SSLServerSocket socket = listener;
        Thread thread = new Thread(new Runnable() {
            @Override public void run() {
                SSLSocket peer = null;
                try {
                    peer = (SSLSocket) socket.accept();
                    peer.setSoTimeout(3000);
                    peer.startHandshake();
                    while (peer.getInputStream().read() >= 0) {
                        // The client decides the outcome; this just keeps TLS up.
                    }
                } catch (IOException ignored) {
                    // Handshake failures are the point of these cases.
                } finally {
                    if (peer != null) {
                        try {
                            peer.close();
                        } catch (IOException ignored) {
                            // Best effort teardown.
                        }
                    }
                }
            }
        });
        thread.start();
        return new Expectation(socket, thread);
    }

    /** Serves one stream: upgrade, a fragmented message, a ping, then reads. */
    private static final class StreamServer implements Runnable {
        private final SSLServerSocket listener;
        private final Recorded recorded;

        StreamServer(SSLServerSocket listener, Recorded recorded) {
            this.listener = listener;
            this.recorded = recorded;
        }

        @Override public void run() {
            SSLSocket peer = null;
            try {
                peer = (SSLSocket) listener.accept();
                peer.setSoTimeout(10000);
                byte[] key = readHead(peer.getInputStream());
                OutputStream out = peer.getOutputStream();
                out.write(response(WebSocketHandshake.acceptFor(
                        new String(key, StandardCharsets.ISO_8859_1).trim()),
                        WebSocketHandshake.SUBPROTOCOL).getBytes(StandardCharsets.US_ASCII));
                out.write(fragment(0x02, false, "streamed".getBytes(StandardCharsets.UTF_8)));
                out.write(fragment(0x00, true, " hello".getBytes(StandardCharsets.UTF_8)));
                out.write(new byte[] {(byte) 0x89, 1, 7});
                out.flush();

                Collect sink = new Collect() {
                    @Override public void onMessage(byte[] message) {
                        recorded.message = message;
                    }

                    @Override public void onPong() {
                        recorded.pong = true;
                    }
                };
                WebSocketFrame.Reader reader = new WebSocketFrame.Reader(false);
                byte[] buffer = new byte[64];
                while (recorded.message == null || !recorded.pong) {
                    int read = peer.getInputStream().read(buffer);
                    if (read < 0) break;
                    reader.feed(buffer, 0, read, sink);
                }
            } catch (Exception e) {
                recorded.failure = e.toString();
            } finally {
                if (peer != null) {
                    try {
                        peer.close();
                    } catch (IOException ignored) {
                        // Best effort teardown.
                    }
                }
            }
        }
    }

    private static IOException expectOpenFailure(SSLContext client, int port, String why) {
        try {
            WssTransport.open("localhost", port, client, 2000, 2000).close();
        } catch (IOException expected) {
            checks++;
            return expected;
        }
        throw new AssertionError("connected despite " + why);
    }

    private static byte[] readHead(InputStream input) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        int matched = 0;
        while (matched < 4) {
            int b = input.read();
            if (b < 0) throw new IOException("closed before the head ended");
            out.write(b);
            if ((matched == 0 || matched == 2) && b == '\r') {
                matched++;
            } else if ((matched == 1 || matched == 3) && b == '\n') {
                matched++;
            } else {
                matched = b == '\r' ? 1 : 0;
            }
        }
        for (String line : out.toString("ISO-8859-1").split("\r\n")) {
            if (line.toLowerCase(Locale.ROOT).startsWith("sec-websocket-key:")) {
                return line.substring(line.indexOf(':') + 1).getBytes(StandardCharsets.ISO_8859_1);
            }
        }
        throw new IOException("no key in the handshake");
    }
}
