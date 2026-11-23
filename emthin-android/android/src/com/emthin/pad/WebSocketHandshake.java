package com.emthin.pad;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;
import java.util.Random;

/**
 * The client half of a `GET /v1/stream` upgrade.
 *
 * <p>Contract: TLS WebSocket upgrade on {@code /v1/stream} with subprotocol
 * {@code emthin-terminal.v1}. Clients send no {@code Origin}. Every failure is
 * closed: a status other than 101 (including a 3xx with a Location), a missing
 * {@code Upgrade}/{@code Connection} token, a mismatched
 * {@code Sec-WebSocket-Accept}, a different (or missing) subprotocol, or an
 * extension this client never offered.
 */
final class WebSocketHandshake {
    static final String SUBPROTOCOL = "emthin-terminal.v1";
    static final int MAX_HEAD = 8 * 1024;

    private WebSocketHandshake() {
    }

    static String newKey(Random random) {
        byte[] nonce = new byte[16];
        random.nextBytes(nonce);
        return Base64.getEncoder().encodeToString(nonce);
    }

    static String request(String host, String path, String key) {
        return "GET " + path + " HTTP/1.1\r\n"
                + "Host: " + host + "\r\n"
                + "Upgrade: websocket\r\n"
                + "Connection: Upgrade\r\n"
                + "Sec-WebSocket-Key: " + key + "\r\n"
                + "Sec-WebSocket-Version: 13\r\n"
                + "Sec-WebSocket-Protocol: " + SUBPROTOCOL + "\r\n"
                + "\r\n";
    }

    static String acceptFor(String key) {
        try {
            MessageDigest sha1 = MessageDigest.getInstance("SHA-1");
            byte[] digest = sha1.digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
                    .getBytes(StandardCharsets.US_ASCII));
            return Base64.getEncoder().encodeToString(digest);
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("SHA-1 unavailable", e);
        }
    }

    /** Index just past the response head, or -1 while it is still incomplete. */
    static int headEnd(byte[] buffer, int length) {
        for (int i = 3; i < length; i++) {
            if (buffer[i - 3] == '\r' && buffer[i - 2] == '\n'
                    && buffer[i - 1] == '\r' && buffer[i] == '\n') {
                return i + 1;
            }
        }
        return -1;
    }

    static void validate(String head, String key) throws IOException {
        String[] lines = head.split("\r\n", -1);
        if (lines.length == 0) throw new IOException("empty handshake");
        String[] status = lines[0].split(" ", 3);
        if (status.length < 2 || !"HTTP/1.1".equals(status[0])) {
            throw new IOException("bad status line");
        }
        int code;
        try {
            code = Integer.parseInt(status[1]);
        } catch (NumberFormatException e) {
            throw new IOException("bad status code");
        }
        if (code != 101) throw new IOException("upgrade refused: http " + code);

        Map<String, String> headers = new LinkedHashMap<String, String>();
        for (int i = 1; i < lines.length; i++) {
            String line = lines[i];
            if (line.isEmpty()) continue;
            int colon = line.indexOf(':');
            if (colon <= 0) throw new IOException("bad response header");
            String name = line.substring(0, colon).trim().toLowerCase(Locale.ROOT);
            String value = line.substring(colon + 1).trim();
            if (headers.put(name, value) != null) {
                throw new IOException("repeated response header: " + name);
            }
        }
        if (!hasToken(headers.get("upgrade"), "websocket")) throw new IOException("missing upgrade header");
        if (!hasToken(headers.get("connection"), "upgrade")) throw new IOException("missing connection upgrade token");
        if (!acceptFor(key).equals(headers.get("sec-websocket-accept"))) {
            throw new IOException("accept key mismatch");
        }
        String protocol = headers.get("sec-websocket-protocol");
        if (protocol == null || !SUBPROTOCOL.equals(protocol.trim())) {
            throw new IOException("subprotocol refused");
        }
        if (headers.containsKey("sec-websocket-extensions")) {
            throw new IOException("unsolicited extension");
        }
        if (headers.containsKey("location")) throw new IOException("redirects are not followed");
    }

    private static boolean hasToken(String value, String token) {
        if (value == null) return false;
        for (String part : value.split(",")) {
            if (token.equalsIgnoreCase(part.trim())) return true;
        }
        return false;
    }
}
