package com.emthin.pad;

import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;

import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManagerFactory;

/**
 * Drives the WSS fixture end to end from the host JVM: TLS, upgrade, framing,
 * and the scripted gateway's hello/attach handshake.
 *
 * <p>Usage: {@code WssFixtureProbe <ca-keystore> <host> <port>}. The keystore is
 * PKCS12, password {@code changeit}, holding the fixture's certificate. Exits
 * non-zero with a message on any mismatch, so a lane can treat silence as
 * success.
 */
public final class WssFixtureProbe {
    private static final int DATA = 0;
    private static final int RESIZE = 3;
    private static final String CORE = "6ab09a20fd1288983dfbbac2bb349231";
    private static final String ADDRESS = "t3";
    private static final String GENERATION = "g-wss-1";
    private static int checks;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    public static void main(String[] args) throws Exception {
        KeyStore store = KeyStore.getInstance("PKCS12");
        InputStream in = new FileInputStream(args[0]);
        try {
            store.load(in, "changeit".toCharArray());
        } finally {
            in.close();
        }
        TrustManagerFactory trust = TrustManagerFactory.getInstance(
                TrustManagerFactory.getDefaultAlgorithm());
        trust.init(store);
        SSLContext context = SSLContext.getInstance("TLS");
        context.init(null, trust.getTrustManagers(), null);

        WssTransport transport = WssTransport.open(args[1], Integer.parseInt(args[2]),
                context, 5000, 5000);
        try {
            send(transport, 0, DATA, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\","
                    + "\"params\":{\"version\":1,\"core_id\":\"" + CORE + "\"}}");
            Frame hello = await(transport, "hello");
            check(hello.channel == 0, "hello rides the control channel");
            check(hello.text().contains("\"core_id\":\"" + CORE + "\""), "hello echo");
            check(hello.text().contains("\"features\""), "hello features");

            send(transport, 0, DATA, "{\"jsonrpc\":\"2.0\",\"id\":2,"
                    + "\"method\":\"terminal.attach\",\"params\":{\"address\":\"" + ADDRESS
                    + "\",\"generation\":\"" + GENERATION + "\",\"mode\":\"control\"}}");
            Frame attached = await(transport, "attach");
            check(attached.text().contains("\"lease_id\":\"lease-"), "attach returns a lease");
            check(attached.text().contains("\"mode\":\"control\""), "attach mode");

            Frame resize = null;
            Frame banner = null;
            long deadline = System.currentTimeMillis() + 10000;
            while ((resize == null || banner == null) && System.currentTimeMillis() < deadline) {
                Frame frame = read(transport);
                if (frame.kind == RESIZE) resize = frame;
                if (frame.kind == DATA) banner = frame;
            }
            check(resize != null && java.util.Arrays.equals(resize.payload,
                    new byte[] {0x50, 0x00, 0x18, 0x00}),
                    "resize frame is (80, 24) little-endian");
            check(banner != null, "banner arrives over the stream channel");
            check(banner.text().contains("emthin gateway lease"), "banner text: " + banner.text());
            check(banner.channel > 0, "banner rides the attach channel");
        } finally {
            transport.close();
        }
        System.out.println("WssFixtureProbe: " + checks + " checks");
    }

    private static void send(WssTransport transport, int channel, int kind, String json)
            throws IOException {
        byte[] payload = json.getBytes(StandardCharsets.UTF_8);
        byte[] packet = new byte[9 + payload.length];
        putLe32(packet, 0, channel);
        packet[4] = (byte) kind;
        putLe32(packet, 5, payload.length);
        System.arraycopy(payload, 0, packet, 9, payload.length);
        transport.send(packet);
    }

    private static final class Frame {
        final int channel;
        final int kind;
        final byte[] payload;

        Frame(int channel, int kind, byte[] payload) {
            this.channel = channel;
            this.kind = kind;
            this.payload = payload;
        }

        /** Control JSON, or raw bytes for a stream frame. */
        String text() {
            return new String(payload, StandardCharsets.UTF_8);
        }
    }

    private static Frame await(WssTransport transport, String what) throws IOException {
        long deadline = System.currentTimeMillis() + 10000;
        while (System.currentTimeMillis() < deadline) {
            Frame frame = read(transport);
            if (frame.channel == 0 && frame.kind == DATA) return frame;
        }
        throw new IOException("no control reply for " + what);
    }

    private static Frame read(WssTransport transport) throws IOException {
        byte[] header = readFully(transport.input(), 9);
        int channel = le32(header, 0);
        int kind = header[4] & 0xff;
        int length = le32(header, 5);
        if (length < 0 || length > 64 * 1024) throw new IOException("frame too large: " + length);
        return new Frame(channel, kind, readFully(transport.input(), length));
    }

    private static byte[] readFully(InputStream input, int length) throws IOException {
        byte[] out = new byte[length];
        int at = 0;
        while (at < length) {
            int read = input.read(out, at, length - at);
            if (read < 0) throw new IOException("stream ended mid-frame");
            at += read;
        }
        return out;
    }

    private static void putLe32(byte[] out, int at, int value) {
        out[at] = (byte) (value & 0xff);
        out[at + 1] = (byte) ((value >>> 8) & 0xff);
        out[at + 2] = (byte) ((value >>> 16) & 0xff);
        out[at + 3] = (byte) ((value >>> 24) & 0xff);
    }

    private static int le32(byte[] in, int at) {
        return (in[at] & 0xff) | ((in[at + 1] & 0xff) << 8)
                | ((in[at + 2] & 0xff) << 16) | ((in[at + 3] & 0xff) << 24);
    }
}
