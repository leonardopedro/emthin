package com.emthin.pad;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Bounded, strict JSON for the gateway control plane.
 *
 * <p>org.json is Android-only, so the host Java suite cannot see it, and the
 * gateway contract is stricter than a permissive reader: pairing replies are
 * bounded, and duplicate keys or trailing garbage mean the peer is not speaking
 * the protocol. This reader accepts exactly one value, rejects duplicate object
 * keys and anything past the length cap, and never recurses past a fixed depth.
 */
final class GatewayJson {
    static final int MAX_DEPTH = 8;

    private final String text;
    private int at;

    private GatewayJson(String text) {
        this.text = text;
    }

    static Object parse(String text, int maxLength) throws IllegalArgumentException {
        if (text == null) throw new IllegalArgumentException("empty body");
        if (text.length() > maxLength) throw new IllegalArgumentException("body too long");
        GatewayJson reader = new GatewayJson(text);
        Object value = reader.value(0);
        reader.skipSpace();
        if (reader.at != text.length()) throw new IllegalArgumentException("trailing data");
        return value;
    }

    static Map<String, Object> parseObject(String text, int maxLength) {
        Object value = parse(text, maxLength);
        if (!(value instanceof Map)) throw new IllegalArgumentException("expected object");
        @SuppressWarnings("unchecked")
        Map<String, Object> object = (Map<String, Object>) value;
        return object;
    }

    static String quote(String value) {
        StringBuilder out = new StringBuilder(value.length() + 2);
        out.append('"');
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            switch (c) {
                case '"': out.append("\\\""); break;
                case '\\': out.append("\\\\"); break;
                case '\n': out.append("\\n"); break;
                case '\r': out.append("\\r"); break;
                case '\t': out.append("\\t"); break;
                default:
                    if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
                    else out.append(c);
            }
        }
        return out.append('"').toString();
    }

    private Object value(int depth) {
        if (depth > MAX_DEPTH) throw new IllegalArgumentException("too deep");
        skipSpace();
        if (at >= text.length()) throw new IllegalArgumentException("unexpected end");
        char c = text.charAt(at);
        if (c == '{') return object(depth);
        if (c == '[') return array(depth);
        if (c == '"') return string();
        if (c == 't') return literal("true", Boolean.TRUE);
        if (c == 'f') return literal("false", Boolean.FALSE);
        if (c == 'n') return literal("null", null);
        return number();
    }

    private Map<String, Object> object(int depth) {
        Map<String, Object> out = new LinkedHashMap<String, Object>();
        at++;
        skipSpace();
        if (peek('}')) {
            at++;
            return out;
        }
        while (true) {
            skipSpace();
            if (!peek('"')) throw new IllegalArgumentException("expected key");
            String key = string();
            if (out.containsKey(key)) throw new IllegalArgumentException("duplicate key");
            skipSpace();
            expect(':');
            out.put(key, value(depth + 1));
            skipSpace();
            if (peek(',')) {
                at++;
                continue;
            }
            expect('}');
            return out;
        }
    }

    private List<Object> array(int depth) {
        List<Object> out = new ArrayList<Object>();
        at++;
        skipSpace();
        if (peek(']')) {
            at++;
            return out;
        }
        while (true) {
            out.add(value(depth + 1));
            skipSpace();
            if (peek(',')) {
                at++;
                continue;
            }
            expect(']');
            return out;
        }
    }

    private String string() {
        expect('"');
        StringBuilder out = new StringBuilder();
        while (true) {
            if (at >= text.length()) throw new IllegalArgumentException("unterminated string");
            char c = text.charAt(at++);
            if (c == '"') return out.toString();
            if (c < 0x20) throw new IllegalArgumentException("control character in string");
            if (c != '\\') {
                out.append(c);
                continue;
            }
            if (at >= text.length()) throw new IllegalArgumentException("unterminated escape");
            char escape = text.charAt(at++);
            switch (escape) {
                case '"': out.append('"'); break;
                case '\\': out.append('\\'); break;
                case '/': out.append('/'); break;
                case 'b': out.append('\b'); break;
                case 'f': out.append('\f'); break;
                case 'n': out.append('\n'); break;
                case 'r': out.append('\r'); break;
                case 't': out.append('\t'); break;
                case 'u': out.append(unicode()); break;
                default: throw new IllegalArgumentException("bad escape");
            }
        }
    }

    private char unicode() {
        if (at + 4 > text.length()) throw new IllegalArgumentException("short unicode escape");
        int value = 0;
        for (int i = 0; i < 4; i++) {
            int digit = Character.digit(text.charAt(at++), 16);
            if (digit < 0) throw new IllegalArgumentException("bad unicode escape");
            value = (value << 4) | digit;
        }
        return (char) value;
    }

    private Object number() {
        int start = at;
        if (peek('-')) at++;
        while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
        boolean fraction = false;
        if (peek('.')) {
            fraction = true;
            at++;
            while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
        }
        if (peek('e') || peek('E')) {
            fraction = true;
            at++;
            if (peek('+') || peek('-')) at++;
            while (at < text.length() && Character.isDigit(text.charAt(at))) at++;
        }
        if (start == at) throw new IllegalArgumentException("expected value");
        String token = text.substring(start, at);
        try {
            if (fraction) return Double.valueOf(token);
            return Long.valueOf(token);
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("bad number");
        }
    }

    private Object literal(String word, Object value) {
        if (!text.startsWith(word, at)) throw new IllegalArgumentException("bad literal");
        at += word.length();
        return value;
    }

    private boolean peek(char c) {
        return at < text.length() && text.charAt(at) == c;
    }

    private void expect(char c) {
        if (!peek(c)) throw new IllegalArgumentException("expected " + c);
        at++;
    }

    private void skipSpace() {
        while (at < text.length()) {
            char c = text.charAt(at);
            if (c != ' ' && c != '\t' && c != '\n' && c != '\r') return;
            at++;
        }
    }
}
