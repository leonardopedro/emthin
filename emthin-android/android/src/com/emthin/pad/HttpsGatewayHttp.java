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

    @Override public PairingProtocol.Reply request(String method, String url, String body)
            throws IOException {
        if (url == null || !url.startsWith("https://")) throw new IOException("not https");
        HttpsURLConnection connection = (HttpsURLConnection) java.net.URI.create(url).toURL().openConnection();
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
