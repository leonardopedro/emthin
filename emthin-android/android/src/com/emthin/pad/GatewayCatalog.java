package com.emthin.pad;

import java.io.IOException;
import java.net.SocketTimeoutException;
import java.net.UnknownHostException;
import java.util.ArrayList;
import java.util.List;
import java.util.Set;

/**
 * One catalogue from every paired host, merged.
 *
 * <p>The picker is per pad, not per host: the document may pin a session on
 * any gateway the user approved, so the list is the union of each host's
 * `sessions.list`. A host that does not answer contributes a row with the
 * reason and keeps its Reconnect action — one unreachable gateway must not
 * hide the sessions that are reachable, and it must not raise a dialog over
 * them either.
 *
 * <p>Fetching is the caller's thread: this class only decides what a result or
 * a failure becomes, so it is unit-testable without a socket.
 */
final class GatewayCatalog {
    /** `sessions.list` for one endpoint; throws when the host is unreachable. */
    interface Fetcher {
        List<SessionCatalog.Entry> fetch(GatewayEndpoints.Endpoint endpoint) throws IOException;
    }

    private GatewayCatalog() {
    }

    /**
     * @param pinned references the open document holds; matched by reference,
     *     so a row keeps its Pin affordance across a refresh
     */
    static SessionCatalog build(List<GatewayEndpoints.Endpoint> endpoints, Fetcher fetcher,
            Set<String> pinned) {
        List<SessionCatalog.Host> hosts = new ArrayList<>();
        List<SessionCatalog.Entry> entries = new ArrayList<>();
        if (endpoints != null) {
            for (GatewayEndpoints.Endpoint endpoint : endpoints) {
                if (endpoint == null) continue;
                List<SessionCatalog.Entry> rows;
                try {
                    rows = fetcher.fetch(endpoint);
                } catch (IOException e) {
                    hosts.add(new SessionCatalog.Host(endpoint.host, endpoint.name, reason(e)));
                    continue;
                } catch (RuntimeException e) {
                    // A malformed reply is the host's fault, not the pad's: it
                    // becomes the same unreachable row as a refused socket.
                    hosts.add(new SessionCatalog.Host(endpoint.host, endpoint.name, "unreadable"));
                    continue;
                }
                hosts.add(new SessionCatalog.Host(endpoint.host, endpoint.name, null));
                if (rows != null) entries.addAll(rows);
            }
        }
        return SessionCatalog.of(hosts, entries, pinned);
    }

    /** A short reason for the host row; never the peer's own words. */
    static String reason(Throwable error) {
        if (error instanceof SocketTimeoutException) return "timed out";
        if (error instanceof UnknownHostException) return "not found";
        // The transport reached the host but could not read the exchange.
        if ("protocol".equals(error.getMessage())) return "unreadable";
        return "unreachable";
    }
}
