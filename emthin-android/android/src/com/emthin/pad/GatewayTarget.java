package com.emthin.pad;

/**
 * Where one figure's gateway lease connects, and under which core identity.
 *
 * <p>Two sources, in a deliberate order. The device lane's
 * {@code emthin.fixtureGateway} extra points the same build at a loopback
 * scripted core, so while it is set it wins. Otherwise the pad uses the
 * endpoint it paired for the figure's Core ID: a {@code hc://<core>/<address>}
 * reference carries that ID and {@link GatewayEndpoints} is keyed by it. A
 * reference whose Core ID has no row is unpaired and does not connect at all —
 * inventing an origin is worse than leaving the figure dormant.
 *
 * <p>A paired target leaves {@link #generation} null: the pad has no local
 * knowledge of which program generation an address is on, so
 * {@link GatewayLeaseClient} asks the core before attaching.
 */
final class GatewayTarget {
    /** Paired gateways are reached on the tailnet host's HTTPS port. */
    static final int PAIRED_PORT = 443;
    /** The scripted peer's Core ID, which the fixture lane pairs with. */
    static final String FIXTURE_CORE_ID = "6ab09a20fd1288983dfbbac2bb349231";

    final String host;
    final int port;
    final String coreId;
    final String address;
    /** null means "discover through `sessions.list` before attaching". */
    final String generation;
    final boolean secure;
    /** True only for the fixture CA the device lane installs. */
    final boolean fixtureCertificate;

    private GatewayTarget(String host, int port, String coreId, String address,
            String generation, boolean secure, boolean fixtureCertificate) {
        this.host = host;
        this.port = port;
        this.coreId = coreId;
        this.address = address;
        this.generation = generation;
        this.secure = secure;
        this.fixtureCertificate = fixtureCertificate;
    }

    /**
     * Resolves the target for {@code reference}, or null when there is none.
     *
     * @throws IllegalArgumentException with {@code endpoint} or {@code port} as
     *     its message when a fixture extra is set but malformed
     */
    static GatewayTarget resolve(String fixture, String fixtureGeneration,
            String reference, GatewayEndpoints endpoints) {
        Reference parsed = Reference.parse(reference);
        String extra = fixture == null ? "" : fixture.trim();
        if (!extra.isEmpty()) return fromFixture(extra, fixtureGeneration, parsed, reference);
        if (parsed == null || endpoints == null) return null;
        GatewayEndpoints.Endpoint paired = endpoints.findByCore(parsed.coreId);
        if (paired == null) return null;
        return new GatewayTarget(paired.host, PAIRED_PORT, parsed.coreId,
                parsed.address, null, true, false);
    }

    private static GatewayTarget fromFixture(String extra, String fixtureGeneration,
            Reference parsed, String reference) {
        // The fixture lane offers either `host:port` (plain TCP) or
        // `wss://host:port` with `emthin.fixtureGatewayCa` naming a PEM the app
        // can read.
        boolean secure = extra.startsWith("wss://");
        String endpoint = secure ? extra.substring("wss://".length()) : extra;
        int pathAt = endpoint.indexOf('/');
        if (pathAt >= 0) endpoint = endpoint.substring(0, pathAt);
        int colon = endpoint.lastIndexOf(':');
        if (colon <= 0 || colon == endpoint.length() - 1) {
            throw new IllegalArgumentException("endpoint");
        }
        String host = endpoint.substring(0, colon);
        int port;
        try {
            port = Integer.parseInt(endpoint.substring(colon + 1));
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("port");
        }
        if (port <= 0 || port > 65535) throw new IllegalArgumentException("port");
        // A figure's reference is normally `hc://<core>/<address>`; the lane also
        // accepts a bare reference, which is then the address as-is.
        String address = parsed != null ? parsed.address
                : reference == null ? extra : reference.trim();
        // A lane that names a generation keeps the old single-attach behaviour;
        // one that omits it exercises the paired path's discovery step.
        String generation = fixtureGeneration == null || fixtureGeneration.isEmpty()
                ? null : fixtureGeneration;
        return new GatewayTarget(host, port, FIXTURE_CORE_ID,
                address, generation, secure, secure);
    }

    /**
     * Rewrites only the socket origin, keeping who is dialled and how.
     *
     * <p>The device lane cannot resolve a tailnet name, so a row paired through
     * the pairing screen is reached at loopback instead. The Core ID, the
     * address and the "discover the generation" path stay the paired ones:
     * only the origin moves, so the step still proves the registry resolved the
     * target.
     *
     * @param override {@code host:port}, or {@code https://host:port} /
     *     {@code wss://host:port} to trust the fixture certificate
     * @return this target when {@code override} is empty
     * @throws IllegalArgumentException with {@code endpoint} or {@code port} as
     *     its message when the override is malformed
     */
    GatewayTarget remap(String override) {
        String extra = override == null ? "" : override.trim();
        if (extra.isEmpty()) return this;
        boolean tls = extra.startsWith("https://") || extra.startsWith("wss://");
        String endpoint = tls ? extra.substring(extra.indexOf("://") + 3) : extra;
        int pathAt = endpoint.indexOf('/');
        if (pathAt >= 0) endpoint = endpoint.substring(0, pathAt);
        int colon = endpoint.lastIndexOf(':');
        if (colon <= 0 || colon == endpoint.length() - 1) {
            throw new IllegalArgumentException("endpoint");
        }
        int port;
        try {
            port = Integer.parseInt(endpoint.substring(colon + 1));
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("port");
        }
        if (port <= 0 || port > 65535) throw new IllegalArgumentException("port");
        String host = endpoint.substring(0, colon);
        return new GatewayTarget(host, port, coreId, address, generation, tls, tls);
    }

    /** A public terminal reference: `hc://<core-id>/<address>`. */
    static final class Reference {
        final String coreId;
        final String address;

        private Reference(String coreId, String address) {
            this.coreId = coreId;
            this.address = address;
        }

        static Reference parse(String reference) {
            if (reference == null) return null;
            String text = reference.trim();
            if (!text.startsWith("hc://")) return null;
            int slash = text.indexOf('/', "hc://".length());
            if (slash <= "hc://".length() || slash + 1 >= text.length()) return null;
            String coreId = text.substring("hc://".length(), slash);
            String address = text.substring(slash + 1);
            if (!PairingLink.isCoreId(coreId) || !validAddress(address)) return null;
            return new Reference(coreId, address);
        }

        /** The contract's ceiling, and no whitespace to smuggle into a request. */
        private static boolean validAddress(String address) {
            if (address.isEmpty() || address.length() > 256) return false;
            for (int i = 0; i < address.length(); i++) {
                if (Character.isWhitespace(address.charAt(i))
                        || Character.isISOControl(address.charAt(i))) {
                    return false;
                }
            }
            return true;
        }
    }
}
