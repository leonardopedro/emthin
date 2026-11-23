package com.emthin.pad;

import java.io.Closeable;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;

/**
 * What {@link GatewayLeaseClient} needs from a connection: a byte stream in and
 * one emthin frame per {@link #send(byte[])}.
 *
 * <p>The scripted fixture speaks the frame envelope over plain TCP; a paired
 * gateway speaks it over TLS WebSocket messages. Both carry the same frames, so
 * only this seam differs.
 */
interface GatewayTransport extends Closeable {
    InputStream input();

    void send(byte[] packet) throws IOException;

    @Override
    void close();

    interface Connector {
        GatewayTransport open() throws IOException;
    }

    /** Plain TCP, used by the device fixture lane. */
    static GatewayTransport tcp(String host, int port, int connectTimeoutMs) throws IOException {
        Socket socket = new Socket();
        try {
            socket.connect(new InetSocketAddress(host, port), connectTimeoutMs);
            socket.setTcpNoDelay(true);
            socket.setSoTimeout(30000);
        } catch (IOException e) {
            socket.close();
            throw e;
        }
        return new Tcp(socket);
    }

    final class Tcp implements GatewayTransport {
        private final Socket socket;
        private final InputStream input;
        private final Object lock = new Object();

        Tcp(Socket socket) throws IOException {
            this.socket = socket;
            this.input = socket.getInputStream();
        }

        @Override public InputStream input() {
            return input;
        }

        @Override public void send(byte[] packet) throws IOException {
            OutputStream out = socket.getOutputStream();
            synchronized (lock) {
                out.write(packet);
                out.flush();
            }
        }

        @Override public void close() {
            try {
                socket.close();
            } catch (IOException ignored) {
                // Best effort teardown.
            }
        }
    }
}
