package com.emthin.pad;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * The live catalogue: a real socket answering `hello` + `sessions.list`, and
 * what a host that refuses, times out or answers nonsense becomes.
 */
public final class GatewayCatalogTest {
    private static final String CORE = "6ab09a20fd1288983dfbbac2bb349231";
    private static final String LIST_RESULT = "{\"revision\":\"r1\",\"rows\":["
            + "{\"address\":\"t7\",\"generation\":\"g1\",\"label\":\"Lease fixture\","
            + "\"grid\":{\"cols\":80,\"rows\":24},\"lifecycle\":\"live\",\"status\":null,"
            + "\"actions\":[\"watch\",\"control\"]},"
            + "{\"address\":\"t8\",\"label\":\"\",\"lifecycle\":\"lost\"},"
            + "{\"address\":\"has space\",\"label\":\"bad\",\"lifecycle\":\"live\"},"
            + "{\"label\":\"no address\",\"lifecycle\":\"live\"}],\"next_cursor\":null}";

    public static void main(String[] args) throws Exception {
        rowsFromARealHost();
        aRefreshReplacesOnlyItsOwnHost();
        unreachableHostBecomesARow();
        malformedReplyBecomesARow();
        pinnedRowsKeepTheirMark();
        lifecycleWords();
        System.out.println("GatewayCatalogTest: ok");
    }

    private static void rowsFromARealHost() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int port = startHost(LIST_RESULT, failures);
        List<SessionCatalog.Entry> rows =
                GatewaySessions.list(tcp(port), CORE, "fleet.leo.ts.net", "fleet");
        check(failures.isEmpty(), "the host thread stayed healthy: " + failures);
        check(rows.size() == 2, "two valid rows out of four (bad address and missing one dropped)");
        SessionCatalog.Entry live = rows.get(0);
        check(live.reference.equals("hc://" + CORE + "/t7"), "reference is the public one");
        check(live.caption.equals("Lease fixture"), "label is the caption");
        check(live.hostLabel.equals("fleet"), "host label comes from the registry row");
        check(live.host.equals("fleet.leo.ts.net"), "and the row keeps its host key");
        check(live.state == SessionCatalog.State.LIVE, "live lifecycle");
        check(!live.pinned, "a fresh row is not pinned");
        SessionCatalog.Entry lost = rows.get(1);
        check(lost.caption.equals("t8"), "an empty label falls back to the address");
        check(lost.state == SessionCatalog.State.OFFLINE, "lost lifecycle is offline");
    }

    /**
     * Reconnect is per host: one host's fresh answer replaces its rows and its
     * outcome, and the host that answered before keeps both.
     */
    private static void aRefreshReplacesOnlyItsOwnHost() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int first = startHost(LIST_RESULT, failures);
        int second = startHost(LIST_RESULT, failures);
        final int firstPort = first;
        final int secondPort = second;
        List<GatewayEndpoints.Endpoint> endpoints = new ArrayList<>();
        endpoints.add(new GatewayEndpoints.Endpoint("one.leo.ts.net", CORE, "one"));
        endpoints.add(new GatewayEndpoints.Endpoint("two.leo.ts.net", CORE, "two"));
        SessionCatalog catalog = GatewayCatalog.build(endpoints,
                new GatewayCatalog.Fetcher() {
                    @Override public List<SessionCatalog.Entry> fetch(
                            GatewayEndpoints.Endpoint endpoint) throws IOException {
                        int port = "one.leo.ts.net".equals(endpoint.host) ? firstPort : secondPort;
                        return GatewaySessions.list(tcp(port), endpoint.coreId,
                                endpoint.host, endpoint.name);
                    }
                }, Collections.<String>emptySet());
        check(catalog.hosts().size() == 2, "two hosts, two rows");
        check(catalog.entries().size() == 4, "and both catalogues in one list");

        // The second host goes away; only its rows and outcome move.
        SessionCatalog afterFailure = catalog.withHost(
                new SessionCatalog.Host("two.leo.ts.net", "two", "unreachable"),
                new ArrayList<SessionCatalog.Entry>(), Collections.<String>emptySet());
        check(afterFailure.hosts().size() == 2, "the host rows are still both there");
        check(!afterFailure.hosts().get(1).reachable(), "the tapped host is the one marked");
        check(afterFailure.hosts().get(0).reachable(), "the untouched host keeps its answer");
        check(afterFailure.entries().size() == 2, "only the failed host's rows are gone");
        for (SessionCatalog.Entry entry : afterFailure.entries()) {
            check(entry.host.equals("one.leo.ts.net"), "the survivor belongs to the other host");
        }

        // And it comes back without disturbing the order of the picker. The
        // fixture Core serves one connection, so the retry dials a fresh one
        // on the same registry row.
        int third = startHost(LIST_RESULT, failures);
        List<SessionCatalog.Entry> refreshed = GatewaySessions.list(
                tcp(third), CORE, "two.leo.ts.net", "two");
        SessionCatalog afterReturn = afterFailure.withHost(
                new SessionCatalog.Host("two.leo.ts.net", "two", null), refreshed,
                Collections.<String>emptySet());
        check(afterReturn.entries().size() == 4, "the returning host's rows are back");
        check(afterReturn.entries().get(0).host.equals("one.leo.ts.net"),
                "the first host stays first");
        check(afterReturn.entries().get(3).host.equals("two.leo.ts.net"),
                "and the refreshed host's rows follow it");
    }

    private static void unreachableHostBecomesARow() throws Exception {
        ServerSocket probe = new ServerSocket(0);
        int closed = probe.getLocalPort();
        probe.close();
        final int port = closed;
        SessionCatalog catalog = GatewayCatalog.build(endpoints("fleet.leo.ts.net"),
                new GatewayCatalog.Fetcher() {
                    @Override public List<SessionCatalog.Entry> fetch(
                            GatewayEndpoints.Endpoint endpoint) throws IOException {
                        return GatewaySessions.list(tcp(port), endpoint.coreId,
                                endpoint.host, endpoint.name);
                    }
                }, Collections.<String>emptySet());
        check(catalog.hosts().size() == 1, "the refused host still has a row");
        SessionCatalog.Host host = catalog.hosts().get(0);
        check(!host.reachable(), "the host is reported unreachable");
        check("unreachable".equals(host.error), "with a reason, not a stack trace");
        check(catalog.entries().isEmpty(), "and contributes no sessions");
    }

    private static void malformedReplyBecomesARow() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int port = startHost("{\"rows\":\"not a list\"}", failures);
        SessionCatalog catalog = GatewayCatalog.build(endpoints("fleet.leo.ts.net"),
                new GatewayCatalog.Fetcher() {
                    @Override public List<SessionCatalog.Entry> fetch(
                            GatewayEndpoints.Endpoint endpoint) throws IOException {
                        return GatewaySessions.list(tcp(port), endpoint.coreId,
                                endpoint.host, endpoint.name);
                    }
                }, Collections.<String>emptySet());
        SessionCatalog.Host host = catalog.hosts().get(0);
        check(!host.reachable(), "an unreadable answer is not a reachable host");
        check("unreadable".equals(host.error), "and says so");
        check(catalog.entries().isEmpty(), "with no rows invented");
    }

    private static void pinnedRowsKeepTheirMark() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int port = startHost(LIST_RESULT, failures);
        Set<String> pinned = new LinkedHashSet<>();
        pinned.add("hc://" + CORE + "/t7");
        SessionCatalog catalog = GatewayCatalog.build(endpoints("fleet.leo.ts.net"),
                new GatewayCatalog.Fetcher() {
                    @Override public List<SessionCatalog.Entry> fetch(
                            GatewayEndpoints.Endpoint endpoint) throws IOException {
                        return GatewaySessions.list(tcp(port), endpoint.coreId,
                                endpoint.host, endpoint.name);
                    }
                }, pinned);
        check(catalog.byReference("hc://" + CORE + "/t7").pinned, "a reference the document holds is pinned");
        check(!catalog.byReference("hc://" + CORE + "/t8").pinned, "and only that one");
        check(catalog.hosts().get(0).reachable(), "the answering host is reachable");
    }

    private static void lifecycleWords() {
        check(GatewaySessions.state("live") == SessionCatalog.State.LIVE, "live");
        check(GatewaySessions.state("starting") == SessionCatalog.State.STARTING, "starting");
        check(GatewaySessions.state("finished") == SessionCatalog.State.FINISHED, "finished");
        check(GatewaySessions.state("lost") == SessionCatalog.State.OFFLINE, "lost");
        check(GatewaySessions.state("somewhere-else") == SessionCatalog.State.UNAVAILABLE,
                "an unknown word is not offered as live");
        check(GatewaySessions.state(null) == SessionCatalog.State.UNAVAILABLE, "a missing lifecycle too");
    }

    private static List<GatewayEndpoints.Endpoint> endpoints(String host) {
        List<GatewayEndpoints.Endpoint> endpoints = new ArrayList<>();
        endpoints.add(new GatewayEndpoints.Endpoint(host, CORE, "fleet"));
        return endpoints;
    }

    private static GatewayTransport.Connector tcp(final int port) {
        return new GatewayTransport.Connector() {
            @Override public GatewayTransport open() throws IOException {
                return GatewayTransport.tcp("127.0.0.1", port, 2000);
            }
        };
    }

    /** A one-connection Core: hello, then `sessions.list` with {@code result}. */
    private static int startHost(final String listResult, final List<Throwable> failures)
            throws IOException {
        final ServerSocket server = new ServerSocket(0);
        Thread thread = new Thread(new Runnable() {
            @Override public void run() {
                try {
                    Socket socket = server.accept();
                    try {
                        InputStream in = socket.getInputStream();
                        OutputStream out = socket.getOutputStream();
                        String hello = new String(payload(in), StandardCharsets.UTF_8);
                        if (!hello.contains("\"hello\"")) {
                            throw new IllegalStateException("expected hello, got " + hello);
                        }
                        send(out, "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"version\":1}}");
                        String list = new String(payload(in), StandardCharsets.UTF_8);
                        if (!list.contains("\"sessions.list\"")) {
                            throw new IllegalStateException("expected sessions.list, got " + list);
                        }
                        send(out, "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":" + listResult + "}");
                        socket.getInputStream().read();
                    } finally {
                        socket.close();
                    }
                } catch (Throwable t) {
                    failures.add(t);
                } finally {
                    try {
                        server.close();
                    } catch (IOException ignored) {
                        // The test owns the port; closing twice is fine.
                    }
                }
            }
        }, "catalogue-fixture");
        thread.setDaemon(true);
        thread.start();
        return server.getLocalPort();
    }

    private static byte[] payload(InputStream in) throws IOException {
        byte[] header = readExact(in, 9);
        int length = (header[5] & 0xff) | ((header[6] & 0xff) << 8)
                | ((header[7] & 0xff) << 16) | ((header[8] & 0xff) << 24);
        return readExact(in, length);
    }

    private static void send(OutputStream out, String json) throws IOException {
        byte[] payload = json.getBytes(StandardCharsets.UTF_8);
        byte[] header = new byte[9];
        header[4] = 0;
        header[5] = (byte) (payload.length & 0xff);
        header[6] = (byte) ((payload.length >>> 8) & 0xff);
        header[7] = (byte) ((payload.length >>> 16) & 0xff);
        header[8] = (byte) ((payload.length >>> 24) & 0xff);
        out.write(header);
        out.write(payload);
        out.flush();
    }

    private static byte[] readExact(InputStream in, int length) throws IOException {
        byte[] out = new byte[length];
        int at = 0;
        while (at < length) {
            int read = in.read(out, at, length - at);
            if (read < 0) throw new IOException("closed");
            at += read;
        }
        return out;
    }

    private static void check(boolean condition, String what) {
        if (!condition) throw new IllegalStateException("failed: " + what);
    }
}
