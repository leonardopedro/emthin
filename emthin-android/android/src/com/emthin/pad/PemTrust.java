package com.emthin.pad;

import java.io.ByteArrayInputStream;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.security.KeyStore;
import java.security.cert.Certificate;
import java.security.cert.CertificateFactory;

import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManagerFactory;

/**
 * An {@link SSLContext} that trusts exactly the certificate in a PEM file.
 *
 * <p>Only the fixture path uses this: a scripted WSS peer presents a
 * self-signed certificate, and a real pairing must never be routed through a
 * caller-supplied trust anchor. The pad therefore builds this from a file the
 * fixture lane pushes into the app's own storage, and the pairing path keeps
 * the platform's default trust store.
 */
final class PemTrust {
    private PemTrust() {
    }

    /**
     * The platform trust store. A paired gateway presents a public CA
     * certificate, so nothing app-supplied is involved on that path.
     */
    static SSLContext system() throws IOException {
        try {
            return SSLContext.getDefault();
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IOException("platform trust store unavailable", e);
        }
    }

    static SSLContext load(String path) throws IOException {
        Certificate certificate;
        InputStream in;
        try {
            in = new FileInputStream(path);
        } catch (IOException e) {
            throw new IOException("cannot read fixture certificate: " + e.getMessage(), e);
        }
        try {
            certificate = CertificateFactory.getInstance("X.509")
                    .generateCertificate(new ByteArrayInputStream(readAll(in)));
        } catch (java.security.cert.CertificateException e) {
            throw new IOException("fixture certificate is not X.509", e);
        } finally {
            in.close();
        }
        try {
            KeyStore store = KeyStore.getInstance(KeyStore.getDefaultType());
            store.load(null, null);
            store.setCertificateEntry("fixture", certificate);
            TrustManagerFactory trust = TrustManagerFactory.getInstance(
                    TrustManagerFactory.getDefaultAlgorithm());
            trust.init(store);
            SSLContext context = SSLContext.getInstance("TLS");
            context.init(null, trust.getTrustManagers(), null);
            return context;
        } catch (java.security.GeneralSecurityException e) {
            throw new IOException("fixture trust store failed", e);
        }
    }

    private static byte[] readAll(InputStream in) throws IOException {
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        byte[] buffer = new byte[1024];
        int read;
        while ((read = in.read(buffer)) >= 0) out.write(buffer, 0, read);
        return out.toByteArray();
    }
}
