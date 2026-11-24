package com.emthin.pad;

import com.google.zxing.BinaryBitmap;
import com.google.zxing.PlanarYUVLuminanceSource;
import com.google.zxing.ReaderException;
import com.google.zxing.common.HybridBinarizer;
import com.google.zxing.qrcode.QRCodeReader;

/**
 * A QR frame to text, or nothing.
 *
 * <p>Decoding is the only part of scanning that can be tested without a camera,
 * so it is a separate class with no Android framework in it: the host Java suite
 * renders a code with the same ZXing jar's writer and decodes it here. The
 * camera feed arrives as NV21, whose first {@code width * height} bytes are the
 * luminance plane the decoder wants, so no colour conversion is needed.
 */
final class QrScan {
    private QrScan() {
    }

    /** The payload of the first QR code in an NV21 frame, or null. */
    static String decodeNv21(byte[] frame, int width, int height) {
        if (frame == null || width <= 0 || height <= 0) return null;
        int needed = width * height;
        if (frame.length < needed) return null;
        byte[] luminance = new byte[needed];
        System.arraycopy(frame, 0, luminance, 0, needed);
        return decodeLuminance(luminance, width, height);
    }

    /**
     * Whether the preview may be opened yet.
     *
     * <p>The surface exists before the permission answer, and the user may
     * leave for Settings and come back with the grant, so "now" is not the
     * only moment to decide: the same rule runs on the surface callback, after
     * the prompt and on every resume. Opening early reports a denial as a
     * missing camera, so nothing is opened until the grant is real.
     */
    static boolean wantsCamera(boolean finished, boolean granted, boolean hasSurface,
            boolean cameraOpen) {
        return !finished && granted && hasSurface && !cameraOpen;
    }

    /** The payload of the first QR code in a grey frame, or null. */
    static String decodeLuminance(byte[] luminance, int width, int height) {
        if (luminance == null || width <= 0 || height <= 0) return null;
        if (luminance.length < width * height) return null;
        PlanarYUVLuminanceSource source = new PlanarYUVLuminanceSource(
                luminance, width, height, 0, 0, width, height, false);
        try {
            return new QRCodeReader()
                    .decode(new BinaryBitmap(new HybridBinarizer(source)))
                    .getText();
        } catch (ReaderException e) {
            // No code in this frame is the normal case while the camera is
            // still being pointed at one.
            return null;
        }
    }
}
