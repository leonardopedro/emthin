package com.emthin.pad;

import com.google.zxing.BarcodeFormat;
import com.google.zxing.EncodeHintType;
import com.google.zxing.common.BitMatrix;
import com.google.zxing.qrcode.QRCodeWriter;

import java.util.EnumMap;
import java.util.Map;

/**
 * The scanner without a camera: render a code with ZXing's writer, decode it
 * with the app's reader, and check the result is a link the paste path accepts.
 */
public final class QrScanTest {
    private static final String LINK =
            "https://fleet.leo.ts.net/pair?v=1&core=6ab09a20fd1288983dfbbac2bb349231";
    private static int checks;

    private static void check(boolean condition, String what) {
        checks++;
        if (!condition) throw new AssertionError("failed: " + what);
    }

    public static void main(String[] args) throws Exception {
        byte[] image = render(LINK, 240);
        check(LINK.equals(QrScan.decodeLuminance(image, 240, 240)),
                "a rendered pairing link decodes to itself");
        check(PairingLink.parse(QrScan.decodeLuminance(image, 240, 240)) != null,
                "and the paste path accepts what the scanner produces");

        byte[] nv21 = new byte[240 * 240 * 3 / 2];
        System.arraycopy(image, 0, nv21, 0, image.length);
        for (int i = image.length; i < nv21.length; i++) nv21[i] = (byte) 0x80;
        check(LINK.equals(QrScan.decodeNv21(nv21, 240, 240)),
                "an NV21 frame decodes from its luminance plane alone");

        byte[] blank = new byte[240 * 240];
        for (int i = 0; i < blank.length; i++) blank[i] = (byte) 0xff;
        check(QrScan.decodeLuminance(blank, 240, 240) == null, "a blank frame has no code");
        check(QrScan.decodeLuminance(new byte[16], 240, 240) == null, "a short frame is refused");
        check(QrScan.decodeNv21(null, 240, 240) == null, "a missing frame is refused");
        check(QrScan.decodeLuminance(image, 0, 240) == null, "a zero width is refused");

        check(QrScan.wantsCamera(false, true, true, false),
                "a granted permission with a surface opens the preview");
        check(!QrScan.wantsCamera(false, false, true, false),
                "no open before the grant: the surface is laid out under the prompt");
        check(!QrScan.wantsCamera(false, true, false, false),
                "no open without a surface to preview on");
        check(!QrScan.wantsCamera(false, true, true, true),
                "a second open would replace the camera already streaming");
        check(!QrScan.wantsCamera(true, true, true, false),
                "a finished screen opens nothing, including on resume");

        System.out.println("QrScanTest: " + checks + " checks passed");
    }

    private static byte[] render(String text, int size) throws Exception {
        Map<EncodeHintType, Object> hints = new EnumMap<>(EncodeHintType.class);
        BitMatrix matrix = new QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, size, size, hints);
        byte[] out = new byte[size * size];
        for (int y = 0; y < size; y++) {
            for (int x = 0; x < size; x++) {
                out[y * size + x] = matrix.get(x, y) ? (byte) 0 : (byte) 0xff;
            }
        }
        return out;
    }
}
