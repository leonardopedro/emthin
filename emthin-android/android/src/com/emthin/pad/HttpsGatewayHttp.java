package com.emthin.pad;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;

import javax.net.ssl.HttpsURLConnection;

/**
 * The pairing HTTP calls, with the contract's transport rules baked in: HTTPS
 * only, zero redirects followed, `Cache-Control: no-store`, and a body cap so a
 * hostile or broken peer cannot stream into the app.
 */
final class HttpsGatewayHttp implements PairingProtocol.Http {
    private static final int CONNECT_MS = 5000;
    private static final int READ_MS = 5000;

    /** Null in production; the device lane sets an origin it can reach. */
    private final String base;
    private final javax.net.ssl.SSLContext context;

    HttpsGatewayHttp() {
        this(null, null);
    }

    /**
     * A lane-only variant: every request keeps its path and query but is sent
     * to {@code base} instead of the link's tailnet origin, trusting
     * {@code context} (the fixture's certificate) rather than the platform.
     * The link itself is still parsed and validated as a real tailnet link, so
     * the step exercises the production parse and the production protocol.
     */
    HttpsGatewayHttp(String base, javax.net.ssl.SSLContext context) {
        this.base = base == null || base.isEmpty() ? null : base;
        this.context = context;
    }

    /** {@code https://host/v1/pair/7} -> {@code base + "/v1/pair/7"}. */
    static String rebase(String base, String url) {
        int scheme = url.indexOf("://");
        int pathAt = scheme < 0 ? -1 : url.indexOf('/', scheme + 3);
        return pathAt < 0 ? base : base + url.substring(pathAt);
    }

    @Override public PairingProtocol.Reply request(String method, String url, String body)
            throws IOException {
        if (url == null || !url.startsWith("https://")) throw new IOException("not https");
        if (base != null) {
            if (!base.startsWith("https://")) throw new IOException("fixture origin is not https");
            url = rebase(base, url);
        }
        HttpsURLConnection connection = (HttpsURLConnection) java.net.URI.create(url).toURL().openConnection();
        if (context != null) connection.setSSLSocketFactory(context.getSocketFactory());
        connection.setInstanceFollowRedirects(false);
        connection.setConnectTimeout(CONNECT_MS);
        connection.setReadTimeout(READ_MS);
        connection.setUseCaches(false);
        connection.setRequestMethod(method);
        connection.setRequestProperty("Accept", "application/json");
        connection.setRequestProperty("Cache-Control", "no-store");
        if (body != null) {
            byte[] payload = body.getBytes(StandardCharsets.UTF_8);
            connection.setDoOutput(true);
            connection.setFixedLengthStreamingMode(payload.length);
            connection.setRequestProperty("Content-Type", "application/json");
            OutputStream out = connection.getOutputStream();
            try {
                out.write(payload);
            } finally {
                out.close();
            }
        }
        int status = connection.getResponseCode();
        InputStream stream = status >= 400 ? connection.getErrorStream() : connection.getInputStream();
        if (stream == null) return new PairingProtocol.Reply(status, "");
        try {
            return new PairingProtocol.Reply(status, read(stream, PairingProtocol.MAX_BODY));
        } finally {
            stream.close();
            connection.disconnect();
        }
    }

    private static String read(InputStream stream, int limit) throws IOException {
        byte[] buffer = new byte[512];
        StringBuilder out = new StringBuilder();
        while (true) {
            int count = stream.read(buffer);
            if (count < 0) return out.toString();
            if (out.length() + count > limit) throw new IOException("reply too long");
            out.append(new String(buffer, 0, count, StandardCharsets.UTF_8));
        }
    }
}
