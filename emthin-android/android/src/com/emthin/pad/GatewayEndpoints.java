package com.emthin.pad;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The endpoints this pad has paired with, keyed by tailnet host.
 *
 * <p>Only an approved pairing link creates an entry, and the stored name is the
 * host the link named — never a value the gateway sent, so a compromised host
 * cannot rewrite another row's label. Entries are revalidated on load: a
 * registry that was hand-edited or written by an older build cannot smuggle a
 * non-tailnet origin into the connection path.
 */
final class GatewayEndpoints {
    static final int MAX_STORED = 16;

    interface Store {
        String read();

        void write(String value);
    }

    static final class Endpoint {
        final String host;
        final String coreId;
        final String name;

        Endpoint(String host, String coreId, String name) {
            this.host = host;
            this.coreId = coreId;
            this.name = name;
        }
    }

    private final Store store;
    private final Map<String, Endpoint> endpoints = new LinkedHashMap<String, Endpoint>();

    GatewayEndpoints(Store store) {
        this.store = store;
        load();
    }

    List<Endpoint> list() {
        return new ArrayList<Endpoint>(endpoints.values());
    }

    Endpoint find(String host) {
        return endpoints.get(host);
    }

    /**
     * The row paired with a Core ID. Hosts key the registry (that is what the
     * list shows and Forget removes), but a figure names a Core, so the
     * connection path looks rows up by it.
     */
    Endpoint findByCore(String coreId) {
        if (coreId == null) return null;
        for (Endpoint endpoint : endpoints.values()) {
            if (endpoint.coreId.equals(coreId)) return endpoint;
        }
        return null;
    }

    /** Adds or refreshes a row. Returns false when the pair would be invalid. */
    boolean add(String host, String coreId, String name) {
        if (!validHost(host) || !PairingLink.isCoreId(coreId)) return false;
        if (!endpoints.containsKey(host) && endpoints.size() >= MAX_STORED) {
            String oldest = endpoints.keySet().iterator().next();
            endpoints.remove(oldest);
        }
        endpoints.put(host, new Endpoint(host, coreId, name == null || name.isEmpty() ? host : name));
        save();
        return true;
    }

    void remove(String host) {
        if (endpoints.remove(host) != null) save();
    }

    private void load() {
        String stored = store.read();
        if (stored == null || stored.isEmpty()) return;
        try {
            Map<String, Object> root = GatewayJson.parseObject(stored, 8 * 1024);
            Object rows = root.get("endpoints");
            if (!(rows instanceof List)) return;
            for (Object row : (List<?>) rows) {
                if (!(row instanceof Map)) continue;
                Map<?, ?> entry = (Map<?, ?>) row;
                Object host = entry.get("host");
                Object core = entry.get("core_id");
                Object name = entry.get("name");
                if (!(host instanceof String) || !(core instanceof String)) continue;
                if (!validHost((String) host) || !PairingLink.isCoreId((String) core)) continue;
                String label = name instanceof String && !((String) name).isEmpty()
                        ? (String) name : (String) host;
                endpoints.put((String) host, new Endpoint((String) host, (String) core, label));
            }
        } catch (IllegalArgumentException ignored) {
            // A corrupt registry is empty, not a crash: pairing again is cheap.
            endpoints.clear();
        }
    }

    private void save() {
        StringBuilder out = new StringBuilder();
        out.append("{\"version\":1,\"endpoints\":[");
        boolean first = true;
        for (Endpoint endpoint : endpoints.values()) {
            if (!first) out.append(',');
            first = false;
            out.append("{\"host\":").append(GatewayJson.quote(endpoint.host))
                    .append(",\"core_id\":").append(GatewayJson.quote(endpoint.coreId))
                    .append(",\"name\":").append(GatewayJson.quote(endpoint.name))
                    .append('}');
        }
        out.append("]}");
        store.write(out.toString());
    }

    private static boolean validHost(String host) {
        if (host == null) return false;
        try {
            PairingLink.parse("https://" + host + "/pair?v=1&core=00000000000000000000000000000000");
            return true;
        } catch (IllegalArgumentException e) {
            return false;
        }
    }
}
