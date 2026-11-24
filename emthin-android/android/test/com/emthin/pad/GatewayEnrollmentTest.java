package com.emthin.pad;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * T16's inventory half: what a host proposes, and what the pad sends back when
 * one is enrolled. The refusals are the host's, so they are read here as the
 * error kinds the picker reports rather than as guesses about a session.
 */
public final class GatewayEnrollmentTest {
    private static final String CORE = "6ab09a20fd1288983dfbbac2bb349231";
    private static final String INVENTORY = "{\"revision\":\"inv-vm-1\",\"vm_id\":\"vm-fixture-1\","
            + "\"rows\":["
            + "{\"address\":\"fb-7\",\"name\":\"fb-7\",\"profile\":\"fleet\","
            + "\"sandbox\":\"vm-fixture-1\",\"label\":\"Fleet build\",\"lifecycle\":\"live\"},"
            + "{\"address\":\"inf-3\",\"name\":\"inf-3\",\"profile\":\"infinity\","
            + "\"sandbox\":\"vm-fixture-1\",\"lifecycle\":\"live\"},"
            + "{\"address\":\"no-profile\",\"name\":\"x\"}]}";

    public static void main(String[] args) throws Exception {
        inventoryFromARealHost();
        enrolSendsWhatTheOperatorSaw();
        refusalsArriveAsTheirKind();
        anInventoryWithoutItsVmIsProtocolFailure();
        System.out.println("GatewayEnrollmentTest: ok");
    }

    private static void inventoryFromARealHost() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int port = startHost("enrollment.list", "\"result\":" + INVENTORY, failures);
        GatewaySessions.Inventory inventory = GatewaySessions.inventory(tcp(port), CORE);
        check(failures.isEmpty(), "the host thread stayed healthy: " + failures);
        check("vm-fixture-1".equals(inventory.vmId), "the inventory names the VM it was read against");
        check(inventory.proposals.size() == 2, "two proposals; the row without a profile is dropped");
        GatewaySessions.Proposal first = inventory.proposals.get(0);
        check("fb-7".equals(first.address), "the address is the host's own name for the session");
        check("fleet".equals(first.profile), "the profile rides along");
        check("vm-fixture-1".equals(first.sandbox), "and the sandbox it runs in");
        check(first.describe().equals("fb-7 \u00b7 fleet \u00b7 vm-fixture-1"),
                "what the operator reads: name, profile and VM");
        check("inf-3".equals(inventory.proposals.get(1).name),
                "a row without a label falls back to its address");
    }

    private static void enrolSendsWhatTheOperatorSaw() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        List<String> requests = new ArrayList<>();
        int port = startHost("enrollment.enroll",
                "\"result\":{\"address\":\"fb-7\",\"profile\":\"fleet\",\"lifecycle\":\"live\"}",
                failures, requests);
        GatewaySessions.Proposal proposal =
                new GatewaySessions.Proposal("fb-7", "fb-7", "fleet", "vm-fixture-1");
        GatewaySessions.enroll(tcp(port), CORE, proposal, "vm-fixture-1");
        check(failures.isEmpty(), "the host thread stayed healthy: " + failures);
        String body = requests.get(0);
        check(body.contains("\"method\":\"enrollment.enroll\""), "the enrol method is the one sent");
        check(body.contains("\"address\":\"fb-7\""), "the proposal's address is named");
        check(body.contains("\"vm_id\":\"vm-fixture-1\""), "and the VM the list was read against");
        check(body.contains("\"profile\":\"fleet\""), "and the profile it belongs to");
        check(!body.contains("sandbox"), "the sandbox is the host's field, not the pad's to send");
    }

    private static void refusalsArriveAsTheirKind() throws Exception {
        // A stale VM: the proposal named a VM the host no longer runs.
        List<Throwable> failures = new ArrayList<>();
        int stale = startHost("enrollment.enroll",
                "\"error\":{\"code\":-32000,\"message\":\"stale vm\",\"data\":{\"kind\":\"stale_vm\"}}",
                failures);
        check(kindOf(stale).equals("stale_vm"), "a stale VM comes back as stale_vm");

        // A spoofed profile: a name the host proposed, claimed under another one.
        int spoofed = startHost("enrollment.enroll",
                "\"error\":{\"code\":-32000,\"message\":\"profile mismatch\",\"data\":{\"kind\":\"spoofed\"}}",
                failures);
        check(kindOf(spoofed).equals("spoofed"), "a profile that is not the proposal's is spoofed");

        // A name the host never proposed.
        int unknown = startHost("enrollment.enroll",
                "\"error\":{\"code\":-32000,\"message\":\"no such session\",\"data\":{\"kind\":\"not_found\"}}",
                failures);
        check(kindOf(unknown).equals("not_found"), "an unproposed name is not_found");
        check(failures.isEmpty(), "every refusal was a host answer, not a harness fault: " + failures);
    }

    private static void anInventoryWithoutItsVmIsProtocolFailure() throws Exception {
        List<Throwable> failures = new ArrayList<>();
        int port = startHost("enrollment.list",
                "\"result\":{\"revision\":\"inv-1\",\"rows\":[]}", failures);
        String failure = null;
        try {
            GatewaySessions.inventory(tcp(port), CORE);
        } catch (IOException e) {
            failure = e.getMessage();
        }
        check("protocol".equals(failure), "an inventory that does not say which VM it is is refused");
    }

    /** The refusal kind from a host that answers `enrollment.enroll` with it. */
    private static String kindOf(int port) {
        try {
            GatewaySessions.enroll(tcp(port), CORE,
                    new GatewaySessions.Proposal("fb-7", "fb-7", "fleet", "vm-fixture-1"),
                    "vm-fixture-1");
            return "enrolled";
        } catch (IOException e) {
            return String.valueOf(e.getMessage());
        }
    }

    private static GatewayTransport.Connector tcp(final int port) {
        return new GatewayTransport.Connector() {
            @Override public GatewayTransport open() throws IOException {
                return GatewayTransport.tcp("127.0.0.1", port, 2000);
            }
        };
    }

    private static int startHost(final String method, final String reply,
            final List<Throwable> failures) throws IOException {
        return startHost(method, reply, failures, new ArrayList<String>());
    }

    /** A one-connection Core: hello, then {@code method} answered by {@code reply}. */
    private static int startHost(final String method, final String reply,
            final List<Throwable> failures, final List<String> requests) throws IOException {
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
                        String request = new String(payload(in), StandardCharsets.UTF_8);
                        if (!request.contains("\"" + method + "\"")) {
                            throw new IllegalStateException("expected " + method + ", got " + request);
                        }
                        requests.add(request);
                        send(out, "{\"jsonrpc\":\"2.0\",\"id\":2," + reply + "}");
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
        }, "enrollment-fixture");
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

    private static byte[] readExact(InputStream in, int length) throws IOException {
        byte[] out = new byte[length];
        int offset = 0;
        while (offset < length) {
            int read = in.read(out, offset, length - offset);
            if (read < 0) throw new IOException("closed");
            offset += read;
        }
        return out;
    }

    private static void send(OutputStream out, String json) throws IOException {
        byte[] payload = json.getBytes(StandardCharsets.UTF_8);
        byte[] header = new byte[9];
        header[5] = (byte) (payload.length & 0xff);
        header[6] = (byte) ((payload.length >>> 8) & 0xff);
        header[7] = (byte) ((payload.length >>> 16) & 0xff);
        header[8] = (byte) ((payload.length >>> 24) & 0xff);
        out.write(header);
        out.write(payload);
        out.flush();
    }

    private static int checks;

    private static void check(boolean condition, String what) {
        checks++;
        if (!condition) throw new AssertionError("failed: " + what);
    }
}
