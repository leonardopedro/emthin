package com.emthin.pad;

/** Host tests for the lease target: registry lookup, fixture override, parsing. */
public final class GatewayTargetTest {
    private static int checks;

    private static final String CORE = "0123456789abcdef0123456789abcdef";
    private static final String OTHER_CORE = "6ab09a20fd1288983dfbbac2bb349231";
    private static final String REFERENCE = "hc://" + CORE + "/t7";

    private static void check(boolean condition, String message) {
        checks++;
        if (!condition) throw new AssertionError(message);
    }

    private static final GatewayEndpoints.Store MEMORY = new GatewayEndpoints.Store() {
        @Override public String read() {
            return null;
        }

        @Override public void write(String value) {
        }
    };

    private static GatewayEndpoints paired() {
        GatewayEndpoints endpoints = new GatewayEndpoints(MEMORY);
        check(endpoints.add("fleet.leo.ts.net", CORE, "Fleet"), "pairing a tailnet host");
        return endpoints;
    }

    private static void expectInvalid(String fixture, String kind) {
        try {
            GatewayTarget.resolve(fixture, null, REFERENCE, null);
        } catch (IllegalArgumentException expected) {
            checks++;
            check(kind.equals(expected.getMessage()),
                    "expected reason " + kind + ", got " + expected.getMessage());
            return;
        }
        throw new AssertionError("accepted fixture target " + fixture);
    }

    public static void main(String[] args) {
        pairedLookup();
        unpaired();
        references();
        fixture();
        fixtureWins();
        remap();
        System.out.println("GatewayTargetTest: " + checks + " checks");
    }

    private static void pairedLookup() {
        GatewayTarget target = GatewayTarget.resolve(null, null, REFERENCE, paired());
        check(target != null, "a paired core resolves");
        check("fleet.leo.ts.net".equals(target.host), "host comes from the registry row");
        check(target.port == GatewayTarget.PAIRED_PORT, "paired gateways use 443");
        check(target.secure, "paired gateways are TLS");
        check(!target.fixtureCertificate, "a paired gateway is a public CA, not the fixture");
        check(CORE.equals(target.coreId), "hello carries the reference's core");
        check("t7".equals(target.address), "address comes from the reference");
        check(target.generation == null, "a paired target discovers its generation");

        check(GatewayTarget.resolve(null, null, "hc://" + CORE + "/a/b",
                paired()) != null, "an address may contain a slash");
        check(GatewayTarget.resolve(null, null, "hc://" + CORE + "/é",
                paired()) != null, "a unicode address is allowed");
    }

    private static void unpaired() {
        check(GatewayTarget.resolve(null, null, "hc://" + OTHER_CORE + "/t3",
                paired()) == null, "an unpaired core does not connect");
        check(GatewayTarget.resolve(null, null, REFERENCE, null) == null,
                "no registry means no target");
        check(GatewayTarget.resolve("  ", null, REFERENCE,
                new GatewayEndpoints(MEMORY)) == null, "a blank fixture extra is ignored");
    }

    private static void references() {
        GatewayEndpoints endpoints = paired();
        check(GatewayTarget.resolve(null, null, null, endpoints) == null, "null reference");
        check(GatewayTarget.resolve(null, null, "", endpoints) == null, "empty reference");
        check(GatewayTarget.resolve(null, null, "hc://" + CORE, endpoints) == null,
                "reference without an address");
        check(GatewayTarget.resolve(null, null, "hc://" + CORE + "/", endpoints) == null,
                "reference with an empty address");
        check(GatewayTarget.resolve(null, null, "hc://not-a-core/t7", endpoints) == null,
                "reference with a non-hex core");
        check(GatewayTarget.resolve(null, null, "hc://" + CORE + "/a b", endpoints) == null,
                "reference with whitespace in the address");
        check(GatewayTarget.resolve(null, null, "hc:/" + CORE + "/t7", endpoints) == null,
                "reference with the wrong scheme");
    }

    private static void fixture() {
        GatewayTarget plain = GatewayTarget.resolve("127.0.0.1:9000", null, REFERENCE, null);
        check(plain != null, "a fixture target needs no registry");
        check("127.0.0.1".equals(plain.host), "fixture host");
        check(plain.port == 9000, "fixture port");
        check(!plain.secure, "a bare host:port is plain TCP");
        check(!plain.fixtureCertificate, "plain TCP uses no certificate");
        check(GatewayTarget.FIXTURE_CORE_ID.equals(plain.coreId),
                "the fixture lane keeps the scripted core's identity");
        check("t7".equals(plain.address), "fixture address from the reference");
        check(plain.generation == null, "no generation extra means discover");

        GatewayTarget named = GatewayTarget.resolve("127.0.0.1:9000", "g-fixture-9",
                REFERENCE, null);
        check("g-fixture-9".equals(named.generation), "an explicit generation is kept");

        GatewayTarget secure = GatewayTarget.resolve("wss://127.0.0.1:8443/v1/stream",
                "g-fixture-1", REFERENCE, null);
        check(secure.secure, "wss:// selects TLS");
        check(secure.fixtureCertificate, "the secure fixture trusts its own CA");
        check(secure.port == 8443, "the port is taken before the path");

        GatewayTarget bare = GatewayTarget.resolve("127.0.0.1:9000", null,
                "not-a-reference", null);
        check("not-a-reference".equals(bare.address),
                "an unparseable reference is all the fixture has for an address");

        expectInvalid("127.0.0.1", "endpoint");
        expectInvalid("127.0.0.1:", "endpoint");
        expectInvalid("127.0.0.1:nope", "port");
        expectInvalid("127.0.0.1:0", "port");
        expectInvalid("127.0.0.1:70000", "port");
    }

    private static void remap() {
        GatewayTarget target = GatewayTarget.resolve(null, null, REFERENCE, paired());
        GatewayTarget loopback = target.remap("wss://127.0.0.1:47658");
        check("127.0.0.1".equals(loopback.host), "the remap moves the origin");
        check(loopback.port == 47658, "the remap takes the override's port");
        check(loopback.secure, "wss:// stays TLS");
        check(loopback.fixtureCertificate, "the remapped origin trusts the fixture CA");
        check(CORE.equals(loopback.coreId), "the remap keeps the row's core identity");
        check("t7".equals(loopback.address), "the remap keeps the figure's address");
        check(loopback.generation == null, "a remapped row still discovers its generation");

        check(target.remap("") == target, "an absent override leaves the target alone");
        check(target.remap(null) == target, "a null override leaves the target alone");

        GatewayTarget plain = target.remap("127.0.0.1:9000");
        check(!plain.secure, "a bare host:port remaps to plain TCP");
        check(!plain.fixtureCertificate, "plain TCP needs no fixture CA");

        for (String bad : new String[] {"127.0.0.1", "127.0.0.1:", "https://127.0.0.1"}) {
            try {
                target.remap(bad);
            } catch (IllegalArgumentException expected) {
                checks++;
                check("endpoint".equals(expected.getMessage()), "bad remap " + bad);
                continue;
            }
            throw new AssertionError("accepted remap " + bad);
        }
        try {
            target.remap("127.0.0.1:nope");
        } catch (IllegalArgumentException expected) {
            checks++;
            check("port".equals(expected.getMessage()), "a non-numeric remap port");
            return;
        }
        throw new AssertionError("accepted a non-numeric remap port");
    }

    private static void fixtureWins() {
        GatewayTarget target = GatewayTarget.resolve("127.0.0.1:9000", "g-fixture-1",
                REFERENCE, paired());
        check(target != null && target.port == 9000,
                "the lane's fixture extra overrides the registry");
        check("g-fixture-1".equals(target.generation), "and keeps its generation");
    }
}
