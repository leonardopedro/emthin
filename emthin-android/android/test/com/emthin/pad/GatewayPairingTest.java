package com.emthin.pad;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/** Host tests for the pairing link, JSON reader, pairing protocol and registry. */
public final class GatewayPairingTest {
    private static int checks;

    private static final String CORE = "6ab09a20fd1288983dfbbac2bb349231";
    private static final String LINK = "https://fleet.leo.ts.net/pair?v=1&core=" + CORE;

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static void rejects(String text, String why) {
        try {
            PairingLink.parse(text);
        } catch (IllegalArgumentException expected) {
            checks++;
            return;
        }
        throw new AssertionError("accepted " + why + ": " + text);
    }

    public static void main(String[] args) throws Exception {
        json();
        links();
        pairing();
        endpoints();
        System.out.println("GatewayPairingTest: " + checks + " checks");
    }

    private static void json() {
        Map<String, Object> object = GatewayJson.parseObject("{\"a\":1,\"b\":[\"x\",true,null]}", 128);
        check(object.get("a") instanceof Long, "integer reads as Long");
        check(((List<?>) object.get("b")).size() == 3, "array length");
        check(GatewayJson.parseObject("{}", 8).isEmpty(), "empty object");
        check(GatewayJson.parseObject("{\"a\":1} ", 8).size() == 1, "trailing space is fine");
        expectJsonFailure("{\"a\":1,\"a\":2}", "duplicate key");
        expectJsonFailure("{\"a\":1}x", "trailing data");
        expectJsonFailure("{\"a\":", "unexpected end");
        expectJsonFailure("[1,2", "unterminated array");
        expectJsonFailure("{\"a\":\"\\q\"}", "bad escape");
        expectJsonFailure("nul", "bad literal");
        expectJsonFailure("[[[[[[[[[[1]]]]]]]]]]", "too deep");
        expectJsonFailure("{\"a\":1}", "over the cap", 4);
        Map<String, Object> escapes = GatewayJson.parseObject("{\"a\":\"\\u0041\\n\"}", 64);
        check("A\n".equals(escapes.get("a")), "unicode escape");
        check("\"a\\\"b\"".equals(GatewayJson.quote("a\"b")), "quote escapes");
    }

    private static void expectJsonFailure(String text, String why) {
        expectJsonFailure(text, why, 64);
    }

    private static void expectJsonFailure(String text, String why, int cap) {
        try {
            GatewayJson.parseObject(text, cap);
        } catch (IllegalArgumentException expected) {
            checks++;
            return;
        }
        throw new AssertionError("parsed " + why + ": " + text);
    }

    private static void links() {
        PairingLink link = PairingLink.parse(LINK);
        check("fleet.leo.ts.net".equals(link.host), "host");
        check(CORE.equals(link.coreId), "core id");
        check("https://fleet.leo.ts.net/v1/pair".equals(link.pairUrl()), "pair url");
        check("wss://fleet.leo.ts.net/v1/stream".equals(link.streamUrl()), "stream url");
        check("fleet.leo".equals(link.displayName()), "display name");
        check("fleet.leo.ts.net".equals(
                PairingLink.parse(" https://Fleet.Leo.TS.NET:443/pair?v=1&core=" + CORE + " ").host),
                "case and port 443 normalise");

        rejects("http://fleet.leo.ts.net/pair?v=1&core=" + CORE, "plain http");
        rejects("wss://fleet.leo.ts.net/pair?v=1&core=" + CORE, "wss scheme");
        rejects("https://user@fleet.leo.ts.net/pair?v=1&core=" + CORE, "credentials");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE + "#x", "fragment");
        rejects("https://fleet.leo.ts.net:8443/pair?v=1&core=" + CORE, "other port");
        rejects("https://192.0.2.10/pair?v=1&core=" + CORE, "ip literal");
        rejects("https://[fd00::1]/pair?v=1&core=" + CORE, "bracketed ip literal");
        rejects("https://evil.example.com/pair?v=1&core=" + CORE, "foreign domain");
        rejects("https://ts.net/pair?v=1&core=" + CORE, "bare ts.net");
        rejects("https://a.ts.net/pair?v=1&core=" + CORE, "two-label ts.net name");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE + "x", "long core id");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE.toUpperCase(java.util.Locale.ROOT),
                "uppercase core id");
        rejects("https://fleet.leo.ts.net/pair?v=2&core=" + CORE, "version 2");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE + "&v=1", "repeated v");
        rejects("https://fleet.leo.ts.net/pair?v=1", "missing core");
        rejects("https://fleet.leo.ts.net/pair?core=" + CORE, "missing version");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE + "&target=x", "unknown parameter");
        rejects("https://fleet.leo.ts.net/pair/?v=1&core=" + CORE, "trailing slash path");
        rejects("https://fleet.leo.ts.net/pairx?v=1&core=" + CORE, "wrong path");
        rejects("https://fleet.leo.ts.net/pair", "missing query");
        rejects("https://fleet..ts.net/pair?v=1&core=" + CORE, "empty label");
        rejects("https://-fleet.leo.ts.net/pair?v=1&core=" + CORE, "leading hyphen");
        rejects("https://fleet.leo.ts.net/pair?v=1&core=" + CORE.substring(0, 8) + '\u0000'
                + CORE.substring(8), "control character");
        check(CORE.equals(PairingLink.parse(LINK + "\n").coreId), "surrounding whitespace is trimmed");
        StringBuilder longLink = new StringBuilder("https://fleet.leo.ts.net/pair?v=1&core=");
        while (longLink.length() <= PairingLink.MAX_LENGTH) longLink.append(CORE.charAt(0));
        rejects(longLink.toString(), "over 1 KiB");
    }

    private static void pairing() throws java.io.IOException {
        final List<String> calls = new ArrayList<String>();
        PairingProtocol.Http http = new PairingProtocol.Http() {
            @Override public PairingProtocol.Reply request(String method, String url, String body) {
                calls.add(method + " " + url + " " + body);
                return new PairingProtocol.Reply(202,
                        "{\"request_id\":\"r1\",\"state\":\"pending\",\"expires_in_s\":300}");
            }
        };
        PairingLink link = PairingLink.parse(LINK);
        PairingProtocol.Result pending = PairingProtocol.start(http, link);
        check(pending.state == PairingProtocol.State.PENDING, "pending start");
        check("r1".equals(pending.requestId), "request id");
        check(pending.expiresInS == 300, "expiry");
        check(calls.get(0).startsWith("POST https://fleet.leo.ts.net/v1/pair "), "post target");
        check(calls.get(0).contains("\"version\":1"), "post version");
        check(calls.get(0).contains(CORE), "post core id");

        check(poll(200, "{\"request_id\":\"r1\",\"state\":\"approved\",\"core_id\":\"" + CORE + "\"}",
                link).state == PairingProtocol.State.APPROVED, "approved");
        check(poll(200, "{\"request_id\":\"r1\",\"state\":\"approved\",\"core_id\":\""
                + "0123456789abcdef0123456789abcdef" + "\"}",
                link).state == PairingProtocol.State.CORE_MISMATCH, "approved for another core");
        check(poll(200, "{\"request_id\":\"r1\",\"state\":\"denied\"}",
                link).state == PairingProtocol.State.DENIED, "denied");
        check(poll(404, "", link).state == PairingProtocol.State.EXPIRED, "404 expires");
        check(poll(403, "", link).state == PairingProtocol.State.FORBIDDEN, "403 forbids");
        check(poll(409, "", link).state == PairingProtocol.State.CORE_MISMATCH, "409 mismatches");
        check(poll(429, "", link).state == PairingProtocol.State.UNAVAILABLE, "429 throttles");
        check(poll(503, "", link).state == PairingProtocol.State.UNAVAILABLE, "503 is unavailable");
        check(poll(302, "", link).state == PairingProtocol.State.INVALID, "redirect is invalid");
        check(poll(200, "{}", link).state == PairingProtocol.State.INVALID, "empty body");
        check(poll(200, "{\"state\":\"pending\"}", link).state
                == PairingProtocol.State.INVALID, "pending without id");
        check(poll(202, "{\"request_id\":\"r1\",\"state\":\"pending\",\"expires_in_s\":1}",
                link).state == PairingProtocol.State.INVALID, "poll must be 200");
        check(poll(200, "{\"request_id\":\"r1\",\"state\":\"approved\",\"core_id\":"
                + GatewayJson.quote(CORE) + "}", link).state
                == PairingProtocol.State.APPROVED, "quoted core id");
        check(PairingProtocol.poll(new PairingProtocol.Http() {
            @Override public PairingProtocol.Reply request(String method, String url, String body) {
                throw new AssertionError("no request for an invalid id");
            }
        }, link, "").state == PairingProtocol.State.INVALID, "empty request id never hits the network");
        check(PairingProtocol.State.APPROVED.terminal(), "approved is terminal");
        check(!PairingProtocol.State.PENDING.terminal(), "pending is not terminal");
    }

    private static PairingProtocol.Result poll(int status, String body, PairingLink link)
            throws java.io.IOException {
        final PairingProtocol.Reply reply = new PairingProtocol.Reply(status, body);
        return PairingProtocol.poll(new PairingProtocol.Http() {
            @Override public PairingProtocol.Reply request(String method, String url, String body) {
                check("GET".equals(method), "poll uses GET");
                check(url.startsWith("https://fleet.leo.ts.net/v1/pair/"), "poll target");
                return reply;
            }
        }, link, "r1");
    }

    private static void endpoints() {
        final StringBuilder stored = new StringBuilder();
        GatewayEndpoints.Store store = new GatewayEndpoints.Store() {
            @Override public String read() {
                return stored.length() == 0 ? null : stored.toString();
            }

            @Override public void write(String value) {
                stored.setLength(0);
                stored.append(value);
            }
        };
        GatewayEndpoints registry = new GatewayEndpoints(store);
        check(registry.list().isEmpty(), "starts empty");
        check(registry.add("fleet.leo.ts.net", CORE, "fleet"), "adds a tailnet host");
        check(registry.list().size() == 1, "one row");
        check("fleet".equals(registry.find("fleet.leo.ts.net").name), "label");
        check(!registry.add("evil.example.com", CORE, "x"), "rejects a foreign host");
        check(!registry.add("fleet.leo.ts.net", "not-a-core", "x"), "rejects a bad core id");
        check(registry.add("fleet.leo.ts.net", CORE, "renamed"), "refreshes in place");
        check(registry.list().size() == 1, "no duplicate row");
        check("renamed".equals(registry.find("fleet.leo.ts.net").name), "label refreshed");

        GatewayEndpoints reloaded = new GatewayEndpoints(store);
        check(reloaded.list().size() == 1, "round-trips through the store");
        check(CORE.equals(reloaded.find("fleet.leo.ts.net").coreId), "core id survives");

        reloaded.remove("fleet.leo.ts.net");
        check(new GatewayEndpoints(store).list().isEmpty(), "removal persists");

        GatewayEndpoints corrupt = new GatewayEndpoints(new GatewayEndpoints.Store() {
            @Override public String read() {
                return "{\"endpoints\":[{\"host\":\"evil.example.com\",\"core_id\":\"" + CORE + "\"}]}";
            }

            @Override public void write(String value) {
                throw new AssertionError("corrupt load must not rewrite");
            }
        });
        check(corrupt.list().isEmpty(), "invalid rows are dropped on load");

        GatewayEndpoints full = new GatewayEndpoints(new GatewayEndpoints.Store() {
            @Override public String read() {
                return null;
            }

            @Override public void write(String value) {
            }
        });
        for (int i = 0; i < GatewayEndpoints.MAX_STORED + 2; i++) {
            check(full.add("h" + i + ".leo.ts.net", CORE, null), "fill " + i);
        }
        check(full.list().size() == GatewayEndpoints.MAX_STORED, "registry is capped");
        check(full.find("h0.leo.ts.net") == null, "oldest row is evicted");
    }
}
