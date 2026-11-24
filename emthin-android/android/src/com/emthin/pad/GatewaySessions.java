package com.emthin.pad;

import java.io.ByteArrayOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * One paired host's {@code sessions.list}, fetched and closed.
 *
 * <p>The lease client keeps a stream open and correlates replies as they
 * arrive; a catalogue read has no such life — hello, one list, close — so it is
 * a separate synchronous call rather than a mode of {@link GatewayLeaseClient}.
 * It speaks the same envelope over the same {@link GatewayTransport}, which is
 * what lets the TLS/WebSocket path and the device fixture reach it alike.
 *
 * <p>Parsing uses {@link GatewayJson}, not org.json: the host Java suite has no
 * Android framework, and this is the layer its tests drive against a real
 * socket.
 */
final class GatewaySessions {
    /** Contract ceiling for one `sessions.list` page. */
    static final int MAX_ROWS = 64;
    /** A catalogue page is bounded; a peer cannot stream into the app. */
    static final int MAX_BODY = 256 * 1024;
    /** Total budget for hello + list on one host. */
    static final int DEADLINE_MS = 8000;

    private static final int DATA = 0;
    private static final int MAX_FRAME = 1024 * 1024;

    private GatewaySessions() {
    }

    /**
     * The host's whole catalogue as public references.
     *
     * @throws IOException when the transport, the handshake or the reply fails;
     *     the caller records the host as unreachable rather than failing the
     *     picker
     */
    static List<SessionCatalog.Entry> list(GatewayTransport.Connector connector, String coreId,
            String hostLabel) throws IOException {
        GatewayTransport transport = connector.open();
        long deadline = System.currentTimeMillis() + DEADLINE_MS;
        try {
            InputStream input = transport.input();
            send(transport, hello(coreId));
            boolean listed = false;
            while (true) {
                if (System.currentTimeMillis() > deadline) throw new IOException("no answer");
                int[] header = readHeader(input);
                byte[] payload = readExact(input, header[2]);
                if (header[0] != 0 || header[1] != DATA) continue;
                Map<String, Object> reply = parseObject(payload);
                Object id = reply.get("id");
                if (!(id instanceof Long)) continue;
                long at = (Long) id;
                if (reply.containsKey("error")) throw new IOException(errorKind(reply, at));
                if (at == 1) {
                    if (listed) throw new IOException("repeated hello");
                    listed = true;
                    send(transport, listRequest());
                    continue;
                }
                if (at == 2) {
                    try {
                        return rows(reply.get("result"), coreId, hostLabel);
                    } catch (IllegalArgumentException e) {
                        throw new IOException("protocol", e);
                    }
                }
            }
        } catch (EOFException e) {
            throw new IOException("closed", e);
        } finally {
            transport.close();
        }
    }

    /** Rows of a `sessions.list` result, deduplicated and bounded. */
    static List<SessionCatalog.Entry> rows(Object resultValue, String coreId, String hostLabel) {
        if (!(resultValue instanceof Map)) throw new IllegalArgumentException("expected result");
        Map<?, ?> result = (Map<?, ?>) resultValue;
        Object rowsValue = result.get("rows");
        if (!(rowsValue instanceof List)) throw new IllegalArgumentException("expected rows");
        Map<String, SessionCatalog.Entry> byReference = new LinkedHashMap<>();
        for (Object rowValue : (List<?>) rowsValue) {
            if (byReference.size() >= MAX_ROWS) break;
            if (!(rowValue instanceof Map)) continue;
            Map<?, ?> row = (Map<?, ?>) rowValue;
            Object addressValue = row.get("address");
            if (!(addressValue instanceof String)) continue;
            String reference = "hc://" + coreId + "/" + addressValue;
            // The same validator the figure's binding uses: an address the pad
            // would refuse to dial never becomes a row it offers.
            if (GatewayTarget.Reference.parse(reference) == null) continue;
            if (byReference.containsKey(reference)) continue;
            Object label = row.get("label");
            String caption = label instanceof String && !((String) label).isEmpty()
                    ? (String) label : (String) addressValue;
            Object lifecycle = row.get("lifecycle");
            byReference.put(reference, new SessionCatalog.Entry(reference, caption,
                    hostLabel, state(lifecycle), false));
        }
        return new ArrayList<>(byReference.values());
    }

    /** `live`, `starting`, `finished`, `lost`; anything else is not offerable. */
    static SessionCatalog.State state(Object lifecycle) {
        String word = lifecycle instanceof String ? (String) lifecycle : "";
        switch (word) {
            case "live": return SessionCatalog.State.LIVE;
            case "starting": return SessionCatalog.State.STARTING;
            case "finished": return SessionCatalog.State.FINISHED;
            case "lost": return SessionCatalog.State.OFFLINE;
            default: return SessionCatalog.State.UNAVAILABLE;
        }
    }

    private static String errorKind(Map<String, Object> reply, long id) {
        Object error = reply.get("error");
        if (error instanceof Map) {
            Object data = ((Map<?, ?>) error).get("data");
            if (data instanceof Map) {
                Object kind = ((Map<?, ?>) data).get("kind");
                if (kind instanceof String) return (String) kind;
            }
        }
        return "request " + id;
    }

    private static Map<String, Object> parseObject(byte[] payload) throws IOException {
        try {
            return GatewayJson.parseObject(new String(payload, StandardCharsets.UTF_8), MAX_BODY);
        } catch (IllegalArgumentException e) {
            throw new IOException("protocol", e);
        }
    }

    private static byte[] hello(String coreId) {
        String request = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"hello\",\"params\":"
                + "{\"version\":1,\"core_id\":" + GatewayJson.quote(coreId)
                + ",\"features\":[\"resize_notifications\",\"automatic_input\"]}}";
        return request.getBytes(StandardCharsets.UTF_8);
    }

    private static byte[] listRequest() {
        String request = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"sessions.list\","
                + "\"params\":{\"limit\":" + MAX_ROWS + "}}";
        return request.getBytes(StandardCharsets.UTF_8);
    }

    private static void send(GatewayTransport transport, byte[] payload) throws IOException {
        ByteArrayOutputStream packet = new ByteArrayOutputStream(payload.length + 9);
        byte[] header = new byte[9];
        putLe32(header, 0, 0);
        header[4] = (byte) DATA;
        putLe32(header, 5, payload.length);
        packet.write(header, 0, header.length);
        packet.write(payload, 0, payload.length);
        transport.send(packet.toByteArray());
    }

    private static int[] readHeader(InputStream input) throws IOException {
        byte[] header = readExact(input, 9);
        int channel = (header[0] & 0xff) | ((header[1] & 0xff) << 8)
                | ((header[2] & 0xff) << 16) | ((header[3] & 0xff) << 24);
        int kind = header[4] & 0xff;
        int length = (header[5] & 0xff) | ((header[6] & 0xff) << 8)
                | ((header[7] & 0xff) << 16) | ((header[8] & 0xff) << 24);
        if (length < 0 || length > MAX_FRAME) throw new IOException("frame too large");
        return new int[] {channel, kind, length};
    }

    private static byte[] readExact(InputStream input, int length) throws IOException {
        if (length == 0) return new byte[0];
        byte[] out = new byte[length];
        int offset = 0;
        while (offset < length) {
            int read = input.read(out, offset, length - offset);
            if (read < 0) throw new EOFException();
            offset += read;
        }
        return out;
    }

    private static void putLe32(byte[] out, int offset, int value) {
        out[offset] = (byte) (value & 0xff);
        out[offset + 1] = (byte) ((value >>> 8) & 0xff);
        out[offset + 2] = (byte) ((value >>> 16) & 0xff);
        out[offset + 3] = (byte) ((value >>> 24) & 0xff);
    }
}
