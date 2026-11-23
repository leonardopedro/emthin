package com.emthin.pad;

import java.net.IDN;

/**
 * The pairing link a host shows as a QR code and as copyable text.
 *
 * <p>Contract (docs/HERDR_GATEWAY_V1.md §Pairing and listeners): only an HTTPS
 * hostname of DNS labels below {@code ts.net}, the default port or 443, the
 * exact {@code /pair} path, one occurrence of each expected query parameter,
 * no credentials, no fragment, no IP literal and at most 1 KiB. Scan and Paste
 * use this same parser. The URL is data: requests are built on its origin and
 * no redirect is ever followed.
 */
final class PairingLink {
    static final int MAX_LENGTH = 1024;
    private static final String SUFFIX = ".ts.net";

    final String host;
    final String coreId;

    private PairingLink(String host, String coreId) {
        this.host = host;
        this.coreId = coreId;
    }

    /** Origin used for both pairing calls, without a trailing slash. */
    String origin() {
        return "https://" + host;
    }

    String pairUrl() {
        return origin() + "/v1/pair";
    }

    String streamUrl() {
        return "wss://" + host + "/v1/stream";
    }

    String displayName() {
        int end = host.length() - SUFFIX.length();
        return end > 0 ? host.substring(0, end) : host;
    }

    static PairingLink parse(String text) throws IllegalArgumentException {
        if (text == null) throw new IllegalArgumentException("empty link");
        String link = text.trim();
        if (link.isEmpty()) throw new IllegalArgumentException("empty link");
        if (link.length() > MAX_LENGTH) throw new IllegalArgumentException("link too long");
        for (int i = 0; i < link.length(); i++) {
            char c = link.charAt(i);
            if (c <= 0x20 || c == 0x7f) throw new IllegalArgumentException("bad character");
        }
        if (!link.startsWith("https://")) {
            throw new IllegalArgumentException("link must be https");
        }
        String rest = link.substring("https://".length());

        int hash = rest.indexOf('#');
        if (hash >= 0) throw new IllegalArgumentException("fragments are not allowed");
        int slash = rest.indexOf('/');
        if (slash < 0) throw new IllegalArgumentException("missing /pair path");
        String authority = rest.substring(0, slash);
        String pathAndQuery = rest.substring(slash);

        if (authority.indexOf('@') >= 0) throw new IllegalArgumentException("credentials are not allowed");
        String host;
        String port = null;
        if (authority.startsWith("[")) throw new IllegalArgumentException("IP literals are not allowed");
        int colon = authority.indexOf(':');
        if (colon >= 0) {
            host = authority.substring(0, colon);
            port = authority.substring(colon + 1);
            if (port.isEmpty()) throw new IllegalArgumentException("empty port");
            for (int i = 0; i < port.length(); i++) {
                if (!Character.isDigit(port.charAt(i))) throw new IllegalArgumentException("bad port");
            }
            if (!"443".equals(port)) throw new IllegalArgumentException("only port 443 is allowed");
        } else {
            host = authority;
        }
        host = validateHost(host);

        int queryAt = pathAndQuery.indexOf('?');
        String path = queryAt < 0 ? pathAndQuery : pathAndQuery.substring(0, queryAt);
        String query = queryAt < 0 ? "" : pathAndQuery.substring(queryAt + 1);
        if (!"/pair".equals(path)) throw new IllegalArgumentException("path must be /pair");

        String version = null;
        String core = null;
        if (!query.isEmpty()) {
            String[] fields = query.split("&", -1);
            for (String field : fields) {
                if (field.isEmpty()) throw new IllegalArgumentException("empty query field");
                int equals = field.indexOf('=');
                if (equals <= 0) throw new IllegalArgumentException("bad query field");
                String name = decode(field.substring(0, equals));
                String value = decode(field.substring(equals + 1));
                if ("v".equals(name)) {
                    if (version != null) throw new IllegalArgumentException("repeated v");
                    version = value;
                } else if ("core".equals(name)) {
                    if (core != null) throw new IllegalArgumentException("repeated core");
                    core = value;
                } else {
                    throw new IllegalArgumentException("unknown query parameter");
                }
            }
        }
        if (!"1".equals(version)) throw new IllegalArgumentException("unsupported pairing version");
        if (core == null || !isCoreId(core)) throw new IllegalArgumentException("bad core id");
        return new PairingLink(host, core);
    }

    static boolean isCoreId(String value) {
        if (value == null || value.length() != 32) return false;
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            boolean hex = (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f');
            if (!hex) return false;
        }
        return true;
    }

    private static String validateHost(String host) {
        if (host.isEmpty()) throw new IllegalArgumentException("missing host");
        if (host.length() > 253) throw new IllegalArgumentException("host too long");
        if (host.indexOf('.') < 0) throw new IllegalArgumentException("host must be a tailnet name");
        String lower = host.toLowerCase(java.util.Locale.ROOT);
        if (!lower.endsWith(SUFFIX)) throw new IllegalArgumentException("host must be below ts.net");
        String[] labels = lower.split("\\.", -1);
        // <host>.<tailnet>.ts.net: a bare `ts.net` or `x.ts.net` is not a node.
        if (labels.length < 4) throw new IllegalArgumentException("host must name a tailnet node");
        for (String label : labels) {
            if (label.isEmpty() || label.length() > 63) {
                throw new IllegalArgumentException("bad host label");
            }
            if (label.charAt(0) == '-' || label.charAt(label.length() - 1) == '-') {
                throw new IllegalArgumentException("bad host label");
            }
            for (int i = 0; i < label.length(); i++) {
                char c = label.charAt(i);
                boolean ok = (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '-';
                if (!ok) throw new IllegalArgumentException("bad host label");
            }
        }
        // The tailnet name is what the host operator sees; keep the IDN form
        // so a punycode label cannot smuggle a different origin past this check.
        try {
            return IDN.toASCII(lower, IDN.ALLOW_UNASSIGNED);
        } catch (IllegalArgumentException e) {
            throw new IllegalArgumentException("bad host name");
        }
    }

    private static String decode(String value) {
        if (value.indexOf('%') < 0) return value;
        StringBuilder out = new StringBuilder(value.length());
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            if (c != '%') {
                out.append(c);
                continue;
            }
            if (i + 2 >= value.length()) throw new IllegalArgumentException("bad escape");
            int high = Character.digit(value.charAt(i + 1), 16);
            int low = Character.digit(value.charAt(i + 2), 16);
            if (high < 0 || low < 0) throw new IllegalArgumentException("bad escape");
            out.append((char) ((high << 4) | low));
            i += 2;
        }
        return out.toString();
    }
}
