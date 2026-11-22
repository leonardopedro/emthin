package com.emthin.pad;

import org.json.JSONObject;

import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * One Android connection to the scripted gateway v1 lease peer.
 *
 * <p>This is deliberately a small transport, not a second terminal parser.
 * It speaks the documented frame envelope, owns the control-channel JSON-RPC
 * correlation, and turns a control attach into a lease. The scene remains the
 * only terminal state; DATA is handed back as bytes.
 */
final class GatewayLeaseClient {
    interface Listener {
        void onReady(String mode, String leaseId);
        void onClosed(String reason, boolean revoked);
        void onData(byte[] bytes);
        void onResize(int cols, int rows);
        void onError(String kind);
    }

    private static final int DATA = 0;
    private static final int CREDIT = 1;
    private static final int CLOSE = 2;
    private static final int RESIZE = 3;
    private static final int MAX_FRAME = 64 * 1024;

    private final String host;
    private final int port;
    private final String address;
    private final String generation;
    private final Listener listener;
    private final AtomicInteger nextId = new AtomicInteger(2);
    private final Object writeLock = new Object();
    private final ExecutorService writes = Executors.newSingleThreadExecutor();

    private volatile Socket socket;
    private volatile OutputStream output;
    private volatile boolean closed;
    private volatile int channel;
    private volatile int pendingAttachId;
    private volatile String mode;
    private volatile String leaseId;
    private volatile boolean controlling;
    private volatile boolean helloDone;

    GatewayLeaseClient(String host, int port, String address, String generation,
            Listener listener) {
        this.host = host;
        this.port = port;
        this.address = address;
        this.generation = generation;
        this.listener = listener;
    }

    void connect() {
        Thread thread = new Thread(new Runnable() {
            @Override public void run() { runConnection(); }
        }, "emthin-gateway-lease");
        thread.setDaemon(true);
        thread.start();
    }

    boolean isControlling() {
        return controlling && channel != 0 && !closed;
    }

    void switchMode(final boolean watchOnly) {
        if (closed || !helloDone) return;
        writes.execute(new Runnable() {
            @Override public void run() {
                closeChannel();
                attach(watchOnly ? "watch" : "control");
            }
        });
    }

    boolean sendInput(final byte[] bytes) {
        if (!isControlling() || bytes == null || bytes.length == 0) return false;
        writes.execute(new Runnable() {
            @Override public void run() {
                try {
                    int current = channel;
                    if (controlling && current != 0) writeFrame(current, DATA, bytes);
                } catch (IOException e) {
                    fail("transport");
                }
            }
        });
        return true;
    }

    void close() {
        closed = true;
        writes.shutdownNow();
        Socket current = socket;
        if (current != null) {
            try {
                current.close();
            } catch (IOException ignored) {
                // Best effort teardown.
            }
        }
    }

    private void runConnection() {
        try {
            Socket current = new Socket();
            socket = current;
            current.connect(new InetSocketAddress(host, port), 4000);
            current.setTcpNoDelay(true);
            current.setSoTimeout(30000);
            output = current.getOutputStream();
            InputStream input = current.getInputStream();
            sendHello();
            while (!closed) {
                int[] header = readHeader(input);
                int frameChannel = header[0];
                int kind = header[1];
                int length = header[2];
                byte[] payload = readPayload(input, length);
                handleFrame(frameChannel, kind, payload);
            }
        } catch (EOFException e) {
            if (!closed) fail("closed");
        } catch (IOException e) {
            if (!closed) fail("transport");
        } catch (RuntimeException e) {
            if (!closed) fail("protocol");
        } finally {
            closed = true;
            try {
                Socket current = socket;
                if (current != null) current.close();
            } catch (IOException ignored) {
                // Best effort teardown.
            }
        }
    }

    private void sendHello() throws IOException {
        try {
            JSONObject params = new JSONObject();
            params.put("version", 1);
            params.put("core_id", "6ab09a20fd1288983dfbbac2bb349231");
            params.put("features", new org.json.JSONArray()
                    .put("resize_notifications").put("automatic_input"));
            JSONObject request = new JSONObject();
            request.put("jsonrpc", "2.0");
            request.put("id", 1);
            request.put("method", "hello");
            request.put("params", params);
            writeFrame(0, DATA, request.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception e) {
            throw new IOException("hello request", e);
        }
    }

    private void attach(String requestedMode) {
        try {
            int id = nextId.getAndIncrement();
            pendingAttachId = id;
            JSONObject params = new JSONObject();
            params.put("address", address);
            params.put("generation", generation);
            params.put("mode", requestedMode);
            params.put("history", 0);
            JSONObject request = new JSONObject();
            request.put("jsonrpc", "2.0");
            request.put("id", id);
            request.put("method", "terminal.attach");
            request.put("params", params);
            writeFrame(0, DATA, request.toString().getBytes(StandardCharsets.UTF_8));
        } catch (Exception e) {
            fail("request");
        }
    }

    private void handleFrame(int frameChannel, int kind, byte[] payload) throws IOException {
        if (frameChannel == 0) {
            if (kind == DATA) handleControl(payload);
            return;
        }
        if (frameChannel != channel) return;
        if (kind == DATA) {
            sendCredit(payload.length);
            listener.onData(payload);
        } else if (kind == CLOSE) {
            String reason = new String(payload, StandardCharsets.UTF_8);
            boolean revoked = reason.contains("revoked");
            channel = 0;
            mode = null;
            leaseId = null;
            controlling = false;
            listener.onClosed(reason, revoked);
        } else if (kind == RESIZE) {
            if (payload.length >= 4) {
                int cols = (payload[0] & 0xff) | ((payload[1] & 0xff) << 8);
                int rows = (payload[2] & 0xff) | ((payload[3] & 0xff) << 8);
                listener.onResize(cols, rows);
            }
        }
    }

    private void handleControl(byte[] payload) {
        try {
            JSONObject reply = new JSONObject(new String(payload, StandardCharsets.UTF_8));
            int id = reply.optInt("id", -1);
            if (reply.has("error")) {
                JSONObject error = reply.optJSONObject("error");
                JSONObject data = error == null ? null : error.optJSONObject("data");
                String kind = data == null ? "request" : data.optString("kind", "request");
                if (id == pendingAttachId) pendingAttachId = 0;
                listener.onError(kind);
                return;
            }
            if (id == 1 && reply.has("result")) {
                helloDone = true;
                attach("watch");
                return;
            }
            if (id == pendingAttachId && reply.has("result")) {
                JSONObject result = reply.optJSONObject("result");
                if (result == null) {
                    listener.onError("protocol");
                    return;
                }
                pendingAttachId = 0;
                channel = result.optInt("channel", 0);
                mode = result.optString("mode", "");
                leaseId = result.isNull("lease_id") ? null : result.optString("lease_id", "");
                controlling = "control".equals(mode);
                listener.onReady(mode, leaseId);
            }
        } catch (Exception e) {
            fail("protocol");
        }
    }

    private void closeChannel() {
        int current = channel;
        if (current == 0) return;
        channel = 0;
        controlling = false;
        mode = null;
        leaseId = null;
        try {
            writeFrame(current, CLOSE, new byte[0]);
        } catch (IOException e) {
            fail("transport");
        }
    }

    private void sendCredit(int bytes) throws IOException {
        byte[] payload = new byte[4];
        payload[0] = (byte) (bytes & 0xff);
        payload[1] = (byte) ((bytes >>> 8) & 0xff);
        payload[2] = (byte) ((bytes >>> 16) & 0xff);
        payload[3] = (byte) ((bytes >>> 24) & 0xff);
        writeFrame(channel, CREDIT, payload);
    }

    private void writeFrame(int frameChannel, int kind, byte[] payload) throws IOException {
        OutputStream current = output;
        if (current == null) throw new IOException("not connected");
        byte[] header = new byte[9];
        putLe32(header, 0, frameChannel);
        header[4] = (byte) kind;
        putLe32(header, 5, payload.length);
        synchronized (writeLock) {
            current.write(header);
            current.write(payload);
            current.flush();
        }
    }

    private static int[] readHeader(InputStream input) throws IOException {
        byte[] header = readExact(input, 9);
        int channel = (header[0] & 0xff) | ((header[1] & 0xff) << 8)
                | ((header[2] & 0xff) << 16) | ((header[3] & 0xff) << 24);
        int kind = header[4] & 0xff;
        int length = (header[5] & 0xff) | ((header[6] & 0xff) << 8)
                | ((header[7] & 0xff) << 16) | ((header[8] & 0xff) << 24);
        if (length < 0 || length > MAX_FRAME) throw new IOException("frame too large");
        return new int[] {channel, kind, length};
    }

    private static byte[] readPayload(InputStream input, int length) throws IOException {
        return length == 0 ? new byte[0] : readExact(input, length);
    }

    private static byte[] readExact(InputStream input, int length) throws IOException {
        byte[] out = new byte[length];
        int offset = 0;
        while (offset < length) {
            int read = input.read(out, offset, length - offset);
            if (read < 0) throw new EOFException();
            offset += read;
        }
        return out;
    }

    private static void putLe32(byte[] out, int offset, int value) {
        out[offset] = (byte) (value & 0xff);
        out[offset + 1] = (byte) ((value >>> 8) & 0xff);
        out[offset + 2] = (byte) ((value >>> 16) & 0xff);
        out[offset + 3] = (byte) ((value >>> 24) & 0xff);
    }

    private void fail(String kind) {
        if (closed) return;
        listener.onError(kind);
    }
}
