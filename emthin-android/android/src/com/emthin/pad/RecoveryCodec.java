package com.emthin.pad;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Pure-Java codec for the A3 recovery envelope.
 *
 * <pre>
 * 8 bytes  "EMPADRC1"
 * u32 LE   metadata length (max 1 MiB)
 * bytes    UTF-8 JSON metadata
 * bytes    raw Loro snapshot (max 32 MiB)
 * 32 bytes SHA-256 over every preceding byte
 * </pre>
 *
 * <p>Decode retains the original bytes on every failure so callers can
 * quarantine or export evidence instead of overwriting it.
 */
final class RecoveryCodec {
    static final byte[] MAGIC = "EMPADRC1".getBytes(StandardCharsets.US_ASCII);
    static final int VERSION = 1;
    static final int MAX_METADATA = 1 << 20;
    static final int MAX_SNAPSHOT = 32 << 20;
    static final int CHECKSUM = 32;

    private RecoveryCodec() {}

    static final class Metadata {
        int version = VERSION;
        long generation;
        String documentId = "";
        String logicalFingerprint = "";
        int snapshotLength;
        String displayName = "";
        String uri = "";
        boolean fileSnapshot = true;
        int caret;
        int selectionEnd;
        int readingAnchor = -1;
        String readingAnchorEncoded = "";
        int readingAnchorSide = -1;
        int readingFallback = -1;
        int scrollY;
        int scrollWidth;
        double zoom = 1.0;
        double panX;
        double panY;
        boolean showMarkers;
        boolean inspectorOpen;
        String query = "";
        String savedFingerprint = "";
        final List<List<String>> folds = new ArrayList<>();
        final List<String> citationChoices = new ArrayList<>();

        Map<String, Object> toMap() {
            Map<String, Object> map = new LinkedHashMap<>();
            map.put("version", version);
            map.put("generation", generation);
            map.put("documentId", documentId);
            map.put("logicalFingerprint", logicalFingerprint);
            map.put("snapshotLength", snapshotLength);
            map.put("displayName", displayName);
            map.put("uri", uri);
            map.put("fileSnapshot", fileSnapshot);
            map.put("caret", caret);
            map.put("selectionEnd", selectionEnd);
            map.put("readingAnchor", readingAnchor);
            map.put("readingAnchorEncoded", readingAnchorEncoded);
            map.put("readingAnchorSide", readingAnchorSide);
            map.put("readingFallback", readingFallback);
            map.put("scrollY", scrollY);
            map.put("scrollWidth", scrollWidth);
            map.put("zoom", zoom);
            map.put("panX", panX);
            map.put("panY", panY);
            map.put("showMarkers", showMarkers);
            map.put("inspectorOpen", inspectorOpen);
            map.put("query", query);
            map.put("savedFingerprint", savedFingerprint);
            List<Object> foldList = new ArrayList<>();
            for (List<String> pair : folds) {
                foldList.add(new ArrayList<Object>(pair));
            }
            map.put("folds", foldList);
            map.put("citationChoices", new ArrayList<Object>(citationChoices));
            return map;
        }

        static Metadata fromMap(Map<String, Object> map) {
            Metadata m = new Metadata();
            m.version = intValue(map, "version", 0);
            m.generation = longValue(map, "generation", -1);
            m.documentId = stringValue(map, "documentId");
            m.logicalFingerprint = stringValue(map, "logicalFingerprint");
            m.snapshotLength = intValue(map, "snapshotLength", -1);
            m.displayName = stringValue(map, "displayName");
            m.uri = stringValue(map, "uri");
            m.fileSnapshot = boolValue(map, "fileSnapshot", true);
            m.caret = intValue(map, "caret", 0);
            m.selectionEnd = intValue(map, "selectionEnd", m.caret);
            m.readingAnchor = intValue(map, "readingAnchor", -1);
            m.readingAnchorEncoded = stringValue(map, "readingAnchorEncoded");
            m.readingAnchorSide = intValue(map, "readingAnchorSide", -1);
            m.readingFallback = intValue(map, "readingFallback", -1);
            m.scrollY = intValue(map, "scrollY", 0);
            m.scrollWidth = intValue(map, "scrollWidth", 0);
            m.zoom = doubleValue(map, "zoom", 1.0);
            m.panX = doubleValue(map, "panX", 0.0);
            m.panY = doubleValue(map, "panY", 0.0);
            m.showMarkers = boolValue(map, "showMarkers", false);
            m.inspectorOpen = boolValue(map, "inspectorOpen", false);
            m.query = stringValue(map, "query");
            m.savedFingerprint = stringValue(map, "savedFingerprint");
            Object folds = map.get("folds");
            if (folds instanceof List) {
                for (Object item : (List<?>) folds) {
                    if (!(item instanceof List)) continue;
                    List<?> pair = (List<?>) item;
                    if (pair.size() != 2) continue;
                    m.folds.add(Arrays.asList(String.valueOf(pair.get(0)), String.valueOf(pair.get(1))));
                }
            }
            Object choices = map.get("citationChoices");
            if (choices instanceof List) {
                for (Object item : (List<?>) choices) {
                    m.citationChoices.add(String.valueOf(item));
                }
            }
            return m;
        }
    }

    static final class Record {
        final Metadata metadata;
        final byte[] snapshot;
        final byte[] raw;

        Record(Metadata metadata, byte[] snapshot, byte[] raw) {
            this.metadata = metadata;
            this.snapshot = snapshot;
            this.raw = raw;
        }
    }

    static final class DecodeException extends Exception {
        private static final long serialVersionUID = 1L;
        final byte[] raw;
        final boolean futureVersion;

        DecodeException(String reason, byte[] raw, boolean futureVersion) {
            super(reason);
            this.raw = raw;
            this.futureVersion = futureVersion;
        }
    }

    static byte[] encode(Metadata metadata, byte[] snapshot) {
        if (metadata == null || snapshot == null) {
            throw new IllegalArgumentException("metadata and snapshot are required");
        }
        if (snapshot.length > MAX_SNAPSHOT) {
            throw new IllegalArgumentException("snapshot exceeds 32 MiB");
        }
        metadata.version = VERSION;
        metadata.snapshotLength = snapshot.length;
        byte[] json = Json.write(metadata.toMap()).getBytes(StandardCharsets.UTF_8);
        if (json.length > MAX_METADATA) {
            throw new IllegalArgumentException("metadata exceeds 1 MiB");
        }
        int total = MAGIC.length + 4 + json.length + snapshot.length + CHECKSUM;
        byte[] out = new byte[total];
        System.arraycopy(MAGIC, 0, out, 0, MAGIC.length);
        putU32(out, MAGIC.length, json.length);
        System.arraycopy(json, 0, out, MAGIC.length + 4, json.length);
        System.arraycopy(snapshot, 0, out, MAGIC.length + 4 + json.length, snapshot.length);
        byte[] digest = sha256(out, 0, total - CHECKSUM);
        System.arraycopy(digest, 0, out, total - CHECKSUM, CHECKSUM);
        return out;
    }

    static Record decode(byte[] bytes) throws DecodeException {
        if (bytes == null) {
            throw new DecodeException("missing recovery data", new byte[0], false);
        }
        int minimum = MAGIC.length + 4 + CHECKSUM;
        if (bytes.length < minimum) {
            throw new DecodeException("truncated envelope", bytes, false);
        }
        for (int i = 0; i < MAGIC.length; i++) {
            if (bytes[i] != MAGIC[i]) {
                throw new DecodeException("bad recovery magic", bytes, false);
            }
        }
        long metadataLength = readU32(bytes, MAGIC.length);
        if (metadataLength > MAX_METADATA) {
            throw new DecodeException("metadata length exceeds 1 MiB", bytes, false);
        }
        long bodyStart = MAGIC.length + 4L + metadataLength;
        long snapshotLength = bytes.length - CHECKSUM - bodyStart;
        if (snapshotLength < 0 || snapshotLength > MAX_SNAPSHOT) {
            throw new DecodeException("snapshot length is out of bounds", bytes, false);
        }
        byte[] expected = sha256(bytes, 0, bytes.length - CHECKSUM);
        if (!MessageDigest.isEqual(expected,
                Arrays.copyOfRange(bytes, bytes.length - CHECKSUM, bytes.length))) {
            throw new DecodeException("checksum mismatch", bytes, false);
        }

        String json = new String(bytes, MAGIC.length + 4, (int) metadataLength,
                StandardCharsets.UTF_8);
        Map<String, Object> map;
        try {
            Object parsed = Json.parse(json);
            if (!(parsed instanceof Map)) throw new IllegalArgumentException("not an object");
            @SuppressWarnings("unchecked")
            Map<String, Object> object = (Map<String, Object>) parsed;
            map = object;
        } catch (RuntimeException e) {
            throw new DecodeException("metadata is not valid JSON", bytes, false);
        }
        Metadata metadata = Metadata.fromMap(map);
        if (metadata.version != VERSION) {
            throw new DecodeException("unsupported recovery version " + metadata.version,
                    bytes, metadata.version > VERSION);
        }
        if (metadata.generation < 0) {
            throw new DecodeException("missing recovery generation", bytes, false);
        }
        if (metadata.documentId.isEmpty()) {
            throw new DecodeException("missing document identity", bytes, false);
        }
        if (metadata.snapshotLength != snapshotLength) {
            throw new DecodeException("snapshot length does not match metadata", bytes, false);
        }
        if (!metadata.logicalFingerprint.isEmpty()
                && !metadata.logicalFingerprint.matches("[0-9a-f]{64}")) {
            throw new DecodeException("logical fingerprint is invalid", bytes, false);
        }
        byte[] snapshot = Arrays.copyOfRange(bytes, (int) bodyStart, bytes.length - CHECKSUM);
        return new Record(metadata, snapshot, bytes);
    }

    /** Stable quarantine suffix for rejected bytes. */
    static String checksumName(byte[] bytes) {
        return toHex(sha256(bytes, 0, bytes.length));
    }

    private static byte[] sha256(byte[] bytes, int offset, int length) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            digest.update(bytes, offset, length);
            return digest.digest();
        } catch (NoSuchAlgorithmException e) {
            throw new AssertionError("SHA-256 is required", e);
        }
    }

    private static String toHex(byte[] bytes) {
        StringBuilder out = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) {
            out.append(Character.forDigit((b >>> 4) & 0xf, 16));
            out.append(Character.forDigit(b & 0xf, 16));
        }
        return out.toString();
    }

    private static long readU32(byte[] bytes, int offset) {
        return (bytes[offset] & 0xffL)
                | ((bytes[offset + 1] & 0xffL) << 8)
                | ((bytes[offset + 2] & 0xffL) << 16)
                | ((bytes[offset + 3] & 0xffL) << 24);
    }

    private static void putU32(byte[] bytes, int offset, int value) {
        bytes[offset] = (byte) value;
        bytes[offset + 1] = (byte) (value >>> 8);
        bytes[offset + 2] = (byte) (value >>> 16);
        bytes[offset + 3] = (byte) (value >>> 24);
    }

    private static String stringValue(Map<String, Object> map, String key) {
        Object value = map.get(key);
        return value == null ? "" : String.valueOf(value);
    }

    private static int intValue(Map<String, Object> map, String key, int fallback) {
        Object value = map.get(key);
        return value instanceof Number ? ((Number) value).intValue() : fallback;
    }

    private static long longValue(Map<String, Object> map, String key, long fallback) {
        Object value = map.get(key);
        return value instanceof Number ? ((Number) value).longValue() : fallback;
    }

    private static double doubleValue(Map<String, Object> map, String key, double fallback) {
        Object value = map.get(key);
        return value instanceof Number ? ((Number) value).doubleValue() : fallback;
    }

    private static boolean boolValue(Map<String, Object> map, String key, boolean fallback) {
        Object value = map.get(key);
        return value instanceof Boolean ? (Boolean) value : fallback;
    }

    /**
     * Minimal deterministic JSON support for the bounded metadata object.
     * Keeping it here lets the codec and its corruption tests run on a host
     * JVM without Android's org.json stubs.
     */
    static final class Json {
        private Json() {}

        static String write(Object value) {
            StringBuilder out = new StringBuilder();
            writeValue(out, value);
            return out.toString();
        }

        private static void writeValue(StringBuilder out, Object value) {
            if (value == null) {
                out.append("null");
            } else if (value instanceof String) {
                writeString(out, (String) value);
            } else if (value instanceof Boolean || value instanceof Integer
                    || value instanceof Long) {
                out.append(value);
            } else if (value instanceof Number) {
                double d = ((Number) value).doubleValue();
                if (Double.isFinite(d)) out.append(d);
                else out.append("null");
            } else if (value instanceof Map) {
                @SuppressWarnings("unchecked")
                Map<String, Object> map = (Map<String, Object>) value;
                out.append('{');
                boolean first = true;
                for (Map.Entry<String, Object> entry : new TreeMap<>(map).entrySet()) {
                    if (!first) out.append(',');
                    first = false;
                    writeString(out, entry.getKey());
                    out.append(':');
                    writeValue(out, entry.getValue());
                }
                out.append('}');
            } else if (value instanceof List) {
                out.append('[');
                boolean first = true;
                for (Object item : (List<?>) value) {
                    if (!first) out.append(',');
                    first = false;
                    writeValue(out, item);
                }
                out.append(']');
            } else {
                writeString(out, String.valueOf(value));
            }
        }

        private static void writeString(StringBuilder out, String value) {
            out.append('"');
            for (int i = 0; i < value.length(); i++) {
                char c = value.charAt(i);
                switch (c) {
                    case '"': out.append("\\\""); break;
                    case '\\': out.append("\\\\"); break;
                    case '\b': out.append("\\b"); break;
                    case '\f': out.append("\\f"); break;
                    case '\n': out.append("\\n"); break;
                    case '\r': out.append("\\r"); break;
                    case '\t': out.append("\\t"); break;
                    default:
                        if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
                        else out.append(c);
                }
            }
            out.append('"');
        }

        static Object parse(String json) {
            Parser parser = new Parser(json);
            Object value = parser.value();
            parser.skipSpace();
            if (!parser.atEnd()) throw new IllegalArgumentException("trailing data");
            return value;
        }

        private static final class Parser {
            private final String text;
            private int at;

            Parser(String text) { this.text = text; }

            boolean atEnd() { return at == text.length(); }

            void skipSpace() {
                while (at < text.length() && Character.isWhitespace(text.charAt(at))) at++;
            }

            Object value() {
                skipSpace();
                if (atEnd()) throw new IllegalArgumentException("unexpected end");
                char c = text.charAt(at);
                if (c == '{') return object();
                if (c == '[') return array();
                if (c == '"') return string();
                if (c == 't') return literal("true", Boolean.TRUE);
                if (c == 'f') return literal("false", Boolean.FALSE);
                if (c == 'n') return literal("null", null);
                return number();
            }

            Map<String, Object> object() {
                Map<String, Object> map = new LinkedHashMap<>();
                at++;
                skipSpace();
                if (peek('}')) { at++; return map; }
                while (true) {
                    skipSpace();
                    String key = string();
                    skipSpace();
                    expect(':');
                    map.put(key, value());
                    skipSpace();
                    if (peek('}')) { at++; return map; }
                    expect(',');
                }
            }

            List<Object> array() {
                List<Object> list = new ArrayList<>();
                at++;
                skipSpace();
                if (peek(']')) { at++; return list; }
                while (true) {
                    list.add(value());
                    skipSpace();
                    if (peek(']')) { at++; return list; }
                    expect(',');
                }
            }

            String string() {
                skipSpace();
                expect('"');
                StringBuilder out = new StringBuilder();
                while (at < text.length()) {
                    char c = text.charAt(at++);
                    if (c == '"') return out.toString();
                    if (c != '\\') { out.append(c); continue; }
                    if (atEnd()) break;
                    char escaped = text.charAt(at++);
                    switch (escaped) {
                        case '"': out.append('"'); break;
                        case '\\': out.append('\\'); break;
                        case '/': out.append('/'); break;
                        case 'b': out.append('\b'); break;
                        case 'f': out.append('\f'); break;
                        case 'n': out.append('\n'); break;
                        case 'r': out.append('\r'); break;
                        case 't': out.append('\t'); break;
                        case 'u':
                            if (at + 4 > text.length()) throw new IllegalArgumentException("short escape");
                            out.append((char) Integer.parseInt(text.substring(at, at + 4), 16));
                            at += 4;
                            break;
                        default: throw new IllegalArgumentException("bad escape");
                    }
                }
                throw new IllegalArgumentException("unterminated string");
            }

            Object number() {
                int start = at;
                if (peek('-')) at++;
                while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
                boolean fractional = false;
                if (peek('.')) {
                    fractional = true;
                    at++;
                    while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
                }
                if (peek('e') || peek('E')) {
                    fractional = true;
                    at++;
                    if (peek('+') || peek('-')) at++;
                    while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
                }
                if (start == at) throw new IllegalArgumentException("expected value");
                String number = text.substring(start, at);
                return fractional ? (Object) Double.parseDouble(number) : (Object) Long.parseLong(number);
            }

            Object literal(String expected, Object value) {
                if (!text.startsWith(expected, at)) throw new IllegalArgumentException("bad literal");
                at += expected.length();
                return value;
            }

            void expect(char c) {
                skipSpace();
                if (at >= text.length() || text.charAt(at) != c) {
                    throw new IllegalArgumentException("expected " + c);
                }
                at++;
            }

            boolean peek(char c) {
                return at < text.length() && text.charAt(at) == c;
            }
        }
    }
}
