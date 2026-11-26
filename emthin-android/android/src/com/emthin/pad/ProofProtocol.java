package com.emthin.pad;

import java.io.UnsupportedEncodingException;
import java.net.URI;
import java.net.URISyntaxException;
import java.net.URLEncoder;
import java.util.Locale;
import java.util.Map;

/**
 * The proof client's half of the mm0web contract, without Android.
 *
 * <p>Verification and publication stay in the backend: this class only decides
 * which server the client may talk to, builds the existing form body, and reads
 * the backend's answer by HTTP status <em>and</em> JSON fields. Reading only the
 * body was how a rejected proof (HTTP 400 with {@code accepted:false}) used to
 * be shown as "verified", so every status the backend documents maps to its own
 * outcome here and anything else is an error, never a success.
 */
final class ProofProtocol {
    /** The backend's own per-field cap ({@code max_proof_bytes}). */
    static final int MAX_FILE_BYTES = 4 * 1024 * 1024;
    /** A /proof answer carries both files back, plus metadata. */
    static final int MAX_RESPONSE_CHARS = 2 * MAX_FILE_BYTES + 64 * 1024;
    static final int MAX_FIELD_CHARS = 200;

    static final String STATE_QUEUED = "queued";
    static final String STATE_PUBLISHED = "published";
    static final String STATE_VERIFIED = "verified";
    static final String STATE_PUBLISH_FAILED = "publish-failed";

    private ProofProtocol() { }

    /**
     * Validate and normalize a backend base URL. HTTPS is required; plain HTTP
     * is accepted only when {@code allowCleartext} (the disposable test APK,
     * whose manifest is the only one that permits cleartext traffic).
     *
     * @return the base without a trailing slash
     * @throws IllegalArgumentException with a user-facing reason
     */
    static String checkBase(String raw, boolean allowCleartext) {
        if (raw == null || raw.trim().isEmpty()) {
            throw new IllegalArgumentException("Set the proof server address first");
        }
        String text = raw.trim();
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c <= 0x20 || c == 0x7f) {
                throw new IllegalArgumentException("The server address contains spaces or control characters");
            }
        }
        if (text.length() > 512) throw new IllegalArgumentException("The server address is too long");
        URI uri;
        try {
            uri = new URI(text);
        } catch (URISyntaxException e) {
            throw new IllegalArgumentException("The server address is not a URL");
        }
        String scheme = uri.getScheme() == null ? "" : uri.getScheme().toLowerCase(Locale.ROOT);
        if (scheme.equals("http")) {
            if (!allowCleartext) {
                throw new IllegalArgumentException("The proof server must use https://");
            }
        } else if (!scheme.equals("https")) {
            throw new IllegalArgumentException("The proof server must use https://");
        }
        if (uri.getHost() == null || uri.getHost().isEmpty()) {
            throw new IllegalArgumentException("The server address has no host");
        }
        if (uri.getRawUserInfo() != null) {
            throw new IllegalArgumentException("Do not put credentials in the server address");
        }
        if (uri.getRawQuery() != null || uri.getRawFragment() != null) {
            throw new IllegalArgumentException("The server address must not have a query or fragment");
        }
        String path = uri.getRawPath() == null ? "" : uri.getRawPath();
        while (path.endsWith("/")) path = path.substring(0, path.length() - 1);
        StringBuilder out = new StringBuilder(scheme).append("://").append(uri.getRawAuthority());
        return out.append(path).toString();
    }

    /**
     * A project slug for the backend's index: letters, digits, '-', '_' and '.'.
     *
     * @throws IllegalArgumentException with a user-facing reason
     */
    static String checkProject(String raw) {
        String project = raw == null ? "" : raw.trim();
        if (project.isEmpty()) throw new IllegalArgumentException("Set a project name");
        if (project.length() > 64) throw new IllegalArgumentException("The project name is too long");
        for (int i = 0; i < project.length(); i++) {
            char c = project.charAt(i);
            boolean ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
                    || (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.';
            if (!ok) throw new IllegalArgumentException("Use letters, digits, '-', '_' or '.' in the project name");
        }
        return project;
    }

    /** A proof title from the spec's display name, without its extension. */
    static String titleFor(String displayName) {
        String name = displayName == null ? "" : displayName.trim();
        int slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf(':'));
        if (slash >= 0) name = name.substring(slash + 1);
        if (name.toLowerCase(Locale.ROOT).endsWith(".mm0")) name = name.substring(0, name.length() - 4);
        StringBuilder clean = new StringBuilder();
        for (int i = 0; i < name.length() && clean.length() < 120; i++) {
            char c = name.charAt(i);
            clean.append(Character.isISOControl(c) ? ' ' : c);
        }
        String title = clean.toString().trim();
        return title.isEmpty() ? "proof" : title;
    }

    /** The backend's form body; the proof falls back to the spec, as the web form does. */
    static String formBody(String project, String title, String mm0, String mm1) {
        return "project=" + enc(project)
                + "&title=" + enc(title)
                + "&mm0=" + enc(mm0)
                + "&mm1=" + enc(mm1 == null ? mm0 : mm1);
    }

    static String proofPath(String project, String id) {
        return "/proof?project=" + enc(project) + "&id=" + enc(id);
    }

    static String statusPath(String project, String id) {
        return "/status?project=" + enc(project) + "&id=" + enc(id);
    }

    private static String enc(String value) {
        try {
            return URLEncoder.encode(value, "UTF-8");
        } catch (UnsupportedEncodingException e) {
            throw new AssertionError(e);
        }
    }

    /** What one verify or publish request answered. */
    static final class Outcome {
        enum Kind {
            /** Verify only: both checkers accepted the pair. */
            ACCEPTED,
            /** A checker rejected the pair; nothing was stored or uploaded. */
            REJECTED,
            /** Publish: verified and handed to the backend's Zenodo worker. */
            QUEUED,
            /** Publish: these exact bytes were already published. */
            ALREADY_PUBLISHED,
            /** Publish: verified and stored, but the backend has no Zenodo key. */
            NOT_PUBLISHED,
            /** Publish: verified and stored, the queue refused the job. */
            QUEUE_UNAVAILABLE,
            /** Too many requests from this client. */
            RATE_LIMITED,
            /** The backend refused the request as malformed. */
            BAD_REQUEST,
            /** Anything the contract does not name. */
            ERROR
        }

        final Kind kind;
        final String proofId;
        final String stage;
        final String doi;
        final String url;
        final String message;

        Outcome(Kind kind, String proofId, String stage, String doi, String url, String message) {
            this.kind = kind;
            this.proofId = proofId == null ? "" : proofId;
            this.stage = stage == null ? "" : stage;
            this.doi = doi == null ? "" : doi;
            this.url = url == null ? "" : url;
            this.message = message == null ? "" : message;
        }

        /** True when the checkers accepted the pair, whatever happened after. */
        boolean verified() {
            return kind == Kind.ACCEPTED || kind == Kind.QUEUED || kind == Kind.ALREADY_PUBLISHED
                    || kind == Kind.NOT_PUBLISHED || kind == Kind.QUEUE_UNAVAILABLE;
        }

        /** The record state this outcome implies, or "" when none was created. */
        String state() {
            switch (kind) {
                case QUEUED: return STATE_QUEUED;
                case ALREADY_PUBLISHED: return STATE_PUBLISHED;
                case NOT_PUBLISHED:
                case QUEUE_UNAVAILABLE: return STATE_VERIFIED;
                default: return "";
            }
        }

        /** One line for the status view. */
        String describe() {
            switch (kind) {
                case ACCEPTED: return "verified: both checkers accepted the pair";
                case REJECTED: return "rejected at stage " + (stage.isEmpty() ? "?" : stage)
                        + (message.isEmpty() ? "" : ": " + message);
                case QUEUED: return "verified; publishing to Zenodo";
                case ALREADY_PUBLISHED: return "already published" + (doi.isEmpty() ? "" : " (" + doi + ")");
                case NOT_PUBLISHED: return "verified and stored; not published: " + message;
                case QUEUE_UNAVAILABLE: return "verified and stored; publishing unavailable: " + message;
                case RATE_LIMITED: return "rate limited: " + message;
                case BAD_REQUEST: return "refused: " + message;
                default: return "error: " + message;
            }
        }
    }

    /**
     * Read a POST /api/v1/verify or /api/v1/publish answer.
     *
     * @param code HTTP status, or a negative value when no response arrived
     * @param body the response text, may be null
     */
    static Outcome parseSubmit(int code, String body, boolean publish) {
        if (code < 0) return error(body == null || body.isEmpty() ? "no response from the server" : body);
        Map<String, Object> json = null;
        try {
            json = GatewayJson.parseObject(body, MAX_RESPONSE_CHARS);
        } catch (IllegalArgumentException ignored) {
            // Plain-text answers (400 bad request, 429 per-minute limit) are
            // part of the contract; they are classified by status below.
        }
        String text = json == null ? firstLine(body) : "";
        if (code == 429) {
            String msg = json != null ? str(json, "message") : text;
            return new Outcome(Outcome.Kind.RATE_LIMITED, null, null, null, null,
                    msg.isEmpty() ? "try again later" : firstLine(msg));
        }
        if (json == null) {
            if (code == 400) return new Outcome(Outcome.Kind.BAD_REQUEST, null, null, null, null, text);
            return error("HTTP " + code + (text.isEmpty() ? "" : ": " + text));
        }
        Object accepted = json.get("accepted");
        if (Boolean.FALSE.equals(accepted)) {
            // Only a 400 carries a checker verdict; a 2xx saying "not accepted"
            // is not something this backend says, so it is not trusted either way.
            if (code != 400) return error("HTTP " + code + " with accepted:false");
            return new Outcome(Outcome.Kind.REJECTED, null, str(json, "stage"), null, null,
                    diagnosticLine(str(json, "log")));
        }
        if (!Boolean.TRUE.equals(accepted)) {
            return error("HTTP " + code + (str(json, "error").isEmpty() ? "" : ": " + str(json, "error")));
        }
        String id = str(json, "proof");
        if (!publish) {
            if (code != 200) return error("HTTP " + code + " for a verification");
            return new Outcome(Outcome.Kind.ACCEPTED, null, str(json, "stage"), null, null, null);
        }
        if (!validId(id)) return error("HTTP " + code + " without a usable proof id");
        if (code == 202 && Boolean.TRUE.equals(json.get("queued"))) {
            return new Outcome(Outcome.Kind.QUEUED, id, null, null, null, null);
        }
        if (code == 200 && Boolean.TRUE.equals(json.get("already"))) {
            return new Outcome(Outcome.Kind.ALREADY_PUBLISHED, id, null, str(json, "doi"),
                    str(json, "url"), null);
        }
        if (code == 502) {
            return new Outcome(Outcome.Kind.NOT_PUBLISHED, id, null, null, null, str(json, "error"));
        }
        if (code == 503) {
            return new Outcome(Outcome.Kind.QUEUE_UNAVAILABLE, id, null, null, null, str(json, "error"));
        }
        return error("HTTP " + code + " not understood for a publication");
    }

    /** A proof record as GET /proof reports it. */
    static final class Record {
        final String state;
        final String doi;
        final String url;

        Record(String state, String doi, String url) {
            this.state = state == null ? "" : state;
            this.doi = doi == null ? "" : doi;
            this.url = url == null ? "" : url;
        }

        /** No further change will come from the backend's worker. */
        boolean settled() {
            return !STATE_QUEUED.equals(state);
        }
    }

    /**
     * Read a GET /proof answer for {@code expectedId}.
     *
     * @throws IllegalArgumentException with a user-facing reason
     */
    static Record parseProof(int code, String body, String expectedId) {
        if (code == 404) throw new IllegalArgumentException("the server has no record of " + expectedId);
        if (code != 200) throw new IllegalArgumentException("HTTP " + code + " reading the proof status");
        Map<String, Object> json;
        try {
            json = GatewayJson.parseObject(body, MAX_RESPONSE_CHARS);
        } catch (IllegalArgumentException e) {
            throw new IllegalArgumentException("the proof status was not readable");
        }
        if (!expectedId.equals(str(json, "id"))) {
            throw new IllegalArgumentException("the server answered for a different proof");
        }
        String state = str(json, "state");
        if (state.isEmpty()) throw new IllegalArgumentException("the proof status has no state");
        return new Record(state, str(json, "doi"), str(json, "url"));
    }

    /**
     * The link handed back to the note: the DOI resolver once there is a DOI,
     * else the archive record, else the backend's own status page.
     */
    static String resultLink(String base, String project, String id, String doi, String url) {
        if (doi != null && !doi.isEmpty()) return "https://doi.org/" + doi;
        if (url != null && url.startsWith("https://")) return url;
        return base + statusPath(project, id);
    }

    /** One line for a record state. */
    static String describeState(String state, String doi) {
        if (STATE_PUBLISHED.equals(state)) return "published" + (doi == null || doi.isEmpty() ? "" : " (" + doi + ")");
        if (STATE_QUEUED.equals(state)) return "publishing to Zenodo";
        if (STATE_PUBLISH_FAILED.equals(state)) return "publication failed; the backend will retry or can be asked again";
        if (STATE_VERIFIED.equals(state)) return "verified and stored; not published";
        return state == null || state.isEmpty() ? "no proof" : state;
    }

    /** Content-addressed ids are short ASCII slugs; anything else is not inserted into a note. */
    static boolean validId(String id) {
        if (id == null || id.isEmpty() || id.length() > MAX_FIELD_CHARS) return false;
        for (int i = 0; i < id.length(); i++) {
            char c = id.charAt(i);
            if (c <= 0x20 || c >= 0x7f || c == '&' || c == '?' || c == '#' || c == '/') return false;
        }
        return true;
    }

    private static Outcome error(String message) {
        return new Outcome(Outcome.Kind.ERROR, null, null, null, null, message);
    }

    private static String str(Map<String, Object> json, String key) {
        Object value = json.get(key);
        if (!(value instanceof String)) return "";
        String s = (String) value;
        return s.length() > 4096 ? s.substring(0, 4096) : s;
    }

    private static String firstLine(String text) {
        if (text == null) return "";
        String t = text.trim();
        int nl = t.indexOf('\n');
        if (nl >= 0) t = t.substring(0, nl).trim();
        return t.length() > MAX_FIELD_CHARS ? t.substring(0, MAX_FIELD_CHARS) : t;
    }

    /**
     * Pick the first diagnostic line produced by the failing checker stage,
     * skipping {@code mm0web}'s {@code $ <cmd>} command headers.
     */
    private static String diagnosticLine(String log) {
        if (log == null) return "";
        String[] lines = log.split("\n");
        int lastHeader = -1;
        for (int i = 0; i < lines.length; i++) {
            lines[i] = lines[i].trim();
            if (lines[i].startsWith("$ ")) lastHeader = i;
        }
        for (int i = lastHeader + 1; i < lines.length; i++) {
            if (!lines[i].isEmpty()) {
                return lines[i].length() > MAX_FIELD_CHARS
                        ? lines[i].substring(0, MAX_FIELD_CHARS) : lines[i];
            }
        }
        for (String line : lines) {
            if (!line.isEmpty() && !line.startsWith("$ ")) {
                return line.length() > MAX_FIELD_CHARS
                        ? line.substring(0, MAX_FIELD_CHARS) : line;
            }
        }
        return firstLine(log);
    }
}
