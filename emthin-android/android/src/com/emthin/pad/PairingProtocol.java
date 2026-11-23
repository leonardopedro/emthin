package com.emthin.pad;

import java.io.IOException;
import java.util.Locale;
import java.util.Map;

/**
 * `POST /v1/pair` then `GET /v1/pair/<id>` every two seconds until a terminal
 * answer (docs/HERDR_GATEWAY_V1.md §Pairing and listeners).
 *
 * <p>The request ID is a tracker, not a credential: only an approval for the
 * Core ID the link named becomes an endpoint, and a mismatched Core ID is its
 * own state rather than a success. Every HTTP status the contract names maps to
 * a state, and anything else — a redirect, a 5xx, a body that is not the
 * bounded JSON object — is {@link State#INVALID}.
 */
final class PairingProtocol {
    static final int MAX_BODY = 4 * 1024;
    static final long POLL_MS = 2000;

    enum State {
        PENDING,
        APPROVED,
        DENIED,
        EXPIRED,
        FORBIDDEN,
        CORE_MISMATCH,
        UNAVAILABLE,
        INVALID;

        /** True once polling must stop. */
        boolean terminal() {
            return this != PENDING;
        }
    }

    static final class Result {
        final State state;
        final String requestId;
        final int expiresInS;
        final String coreId;

        Result(State state, String requestId, int expiresInS, String coreId) {
            this.state = state;
            this.requestId = requestId;
            this.expiresInS = expiresInS;
            this.coreId = coreId;
        }

        String describe() {
            return state.name().toLowerCase(Locale.ROOT);
        }
    }

    interface Http {
        Reply request(String method, String url, String body) throws IOException;
    }

    static final class Reply {
        final int status;
        final String body;

        Reply(int status, String body) {
            this.status = status;
            this.body = body == null ? "" : body;
        }
    }

    private PairingProtocol() {
    }

    static Result start(Http http, PairingLink link) throws IOException {
        String body = "{\"version\":1,\"core_id\":" + GatewayJson.quote(link.coreId) + "}";
        Reply reply = http.request("POST", link.pairUrl(), body);
        return interpret(reply, link, true);
    }

    static Result poll(Http http, PairingLink link, String requestId) throws IOException {
        if (requestId == null || requestId.isEmpty() || requestId.length() > 128) {
            return new Result(State.INVALID, requestId, 0, null);
        }
        Reply reply = http.request("GET", link.pairUrl() + "/" + requestId, null);
        return interpret(reply, link, false);
    }

    private static Result interpret(Reply reply, PairingLink link, boolean started) {
        if (reply.status == 429 || reply.status == 503) {
            return new Result(State.UNAVAILABLE, null, 0, null);
        }
        if (reply.status == 400) return new Result(State.INVALID, null, 0, null);
        if (reply.status == 403) return new Result(State.FORBIDDEN, null, 0, null);
        if (reply.status == 404) return new Result(State.EXPIRED, null, 0, null);
        if (reply.status == 409) return new Result(State.CORE_MISMATCH, null, 0, null);
        boolean accepted = started ? (reply.status == 200 || reply.status == 202) : reply.status == 200;
        if (!accepted) return new Result(State.INVALID, null, 0, null);

        Map<String, Object> object;
        try {
            object = GatewayJson.parseObject(reply.body, MAX_BODY);
        } catch (IllegalArgumentException e) {
            return new Result(State.INVALID, null, 0, null);
        }
        if (object.containsKey("error")) {
            String kind = errorKind(object);
            if ("core_mismatch".equals(kind)) return new Result(State.CORE_MISMATCH, null, 0, null);
            if ("forbidden".equals(kind)) return new Result(State.FORBIDDEN, null, 0, null);
            return new Result(State.INVALID, null, 0, null);
        }
        Object state = object.get("state");
        if (!(state instanceof String)) return new Result(State.INVALID, null, 0, null);
        String value = (String) state;
        String requestId = stringField(object, "request_id");
        String coreId = stringField(object, "core_id");
        int expires = intField(object, "expires_in_s");
        if ("pending".equals(value)) {
            if (requestId == null) return new Result(State.INVALID, null, 0, null);
            return new Result(State.PENDING, requestId, expires, null);
        }
        if ("approved".equals(value)) {
            if (!PairingLink.isCoreId(coreId)) return new Result(State.INVALID, null, 0, null);
            if (!coreId.equals(link.coreId)) return new Result(State.CORE_MISMATCH, null, 0, coreId);
            return new Result(State.APPROVED, requestId, expires, coreId);
        }
        if ("denied".equals(value)) return new Result(State.DENIED, requestId, 0, null);
        if ("expired".equals(value)) return new Result(State.EXPIRED, requestId, 0, null);
        return new Result(State.INVALID, null, 0, null);
    }

    private static String errorKind(Map<String, Object> object) {
        Object error = object.get("error");
        if (!(error instanceof Map)) return null;
        Object kind = ((Map<?, ?>) error).get("kind");
        return kind instanceof String ? (String) kind : null;
    }

    private static String stringField(Map<String, Object> object, String key) {
        Object value = object.get(key);
        return value instanceof String ? (String) value : null;
    }

    private static int intField(Map<String, Object> object, String key) {
        Object value = object.get(key);
        if (value instanceof Long) {
            long number = (Long) value;
            return number > Integer.MAX_VALUE ? Integer.MAX_VALUE : (int) number;
        }
        return 0;
    }
}
