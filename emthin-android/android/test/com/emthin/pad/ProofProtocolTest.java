package com.emthin.pad;

/**
 * The proof client's contract with mm0web, checked against the exact answers
 * the backend's {@code verify_or_publish} and {@code get_proof} produce. The
 * first case is the regression this class exists for: a rejected pair used to
 * be displayed as "verified" because any response body counted as success.
 */
public final class ProofProtocolTest {
    private static int checks;

    private static void check(boolean condition, String what) {
        checks++;
        if (!condition) throw new AssertionError("failed: " + what);
    }

    private static boolean refused(Runnable r) {
        try {
            r.run();
            return false;
        } catch (IllegalArgumentException e) {
            return e.getMessage() != null && !e.getMessage().isEmpty();
        }
    }

    public static void main(String[] args) {
        submitOutcomes();
        proofRecords();
        serverAddresses();
        formAndLinks();
        System.out.println("ProofProtocolTest: " + checks + " checks passed");
    }

    private static void submitOutcomes() {
        ProofProtocol.Outcome o;

        o = ProofProtocol.parseSubmit(400,
                "{\"accepted\":false,\"stage\":\"mm0-rs\",\"log\":\"error: unknown sort Nope\\nmore\"}", false);
        check(o.kind == ProofProtocol.Outcome.Kind.REJECTED, "a 400 verdict is a rejection, not a verification");
        check(!o.verified(), "a rejection is never verified");
        check("mm0-rs".equals(o.stage), "the rejection names its stage");
        check(o.describe().equals("rejected at stage mm0-rs: error: unknown sort Nope"), "and says so with the first diagnostic line");
        check(o.proofId.isEmpty() && o.state().isEmpty(), "a rejection creates no record");
        o = ProofProtocol.parseSubmit(400, "{\"accepted\":false,\"stage\":\"mm0-c\",\"log\":\"\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.REJECTED, "a rejected publication is a rejection too");
        check(o.describe().equals("rejected at stage mm0-c"), "empty diagnostic omits the colon");
        o = ProofProtocol.parseSubmit(400,
                "{\"accepted\":false,\"stage\":\"mm0-c\",\"log\":\"$ mm0-rs compile proof.mm1 proof.mmb\\n\\n"
                + "$ mm0-c proof.mmb < proof.mm0\\nmm0-c: sort Nope not declared\\n\"}", false);
        check(o.describe().equals("rejected at stage mm0-c: mm0-c: sort Nope not declared"),
                "multi-stage rejection extracts the failing mm0-c diagnostic line");
        o = ProofProtocol.parseSubmit(400,
                "{\"accepted\":false,\"stage\":\"mm0-rs\",\"log\":\"$ mm0-rs compile proof.mm1 proof.mmb\\n"
                + "error: unexpected token\\n\"}", false);
        check(o.describe().equals("rejected at stage mm0-rs: error: unexpected token"),
                "single-stage rejection skips the command banner");
        o = ProofProtocol.parseSubmit(400,
                "{\"accepted\":false,\"stage\":\"mm0-rs\",\"log\":\"$ mm0-rs compile proof.mm1 proof.mmb\\n"
                + "warning: hole in proof\\nrejected: mm0-rs emitted a warning\\n\"}", false);
        check(o.describe().equals("rejected at stage mm0-rs: warning: hole in proof"),
                "warning proof-hole rejection surfaces the warning line");

        o = ProofProtocol.parseSubmit(200, "{\"accepted\":true,\"stage\":\"ok\",\"log\":\"\"}", false);
        check(o.kind == ProofProtocol.Outcome.Kind.ACCEPTED && o.verified(), "200 accepted verifies");
        check(o.proofId.isEmpty(), "verification alone mints no proof id");
        check(ProofProtocol.parseSubmit(202, "{\"accepted\":true,\"stage\":\"ok\"}", false).kind
                == ProofProtocol.Outcome.Kind.ERROR, "verify answers 200 or nothing");
        check(ProofProtocol.parseSubmit(200, "{\"accepted\":false,\"stage\":\"x\"}", false).kind
                == ProofProtocol.Outcome.Kind.ERROR, "accepted:false on a 2xx is not trusted as either verdict");

        o = ProofProtocol.parseSubmit(202, "{\"accepted\":true,\"queued\":true,\"proof\":\"sort-Prop-89561941\","
                + "\"status\":\"/status?project=unfer&id=sort-Prop-89561941\",\"log\":\"\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.QUEUED, "202 queued is a queued publication");
        check("sort-Prop-89561941".equals(o.proofId), "with its proof id");
        check(ProofProtocol.STATE_QUEUED.equals(o.state()), "and a queued record");

        o = ProofProtocol.parseSubmit(200, "{\"accepted\":true,\"queued\":false,\"already\":true,"
                + "\"proof\":\"p-1\",\"record\":647037,\"doi\":\"10.5072/zenodo.647037\","
                + "\"url\":\"https://sandbox.zenodo.org/records/647037\",\"status\":\"/status\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.ALREADY_PUBLISHED, "an idempotent re-publish is recognized");
        check("10.5072/zenodo.647037".equals(o.doi) && o.url.endsWith("/647037"), "with its DOI and record");
        check(ProofProtocol.STATE_PUBLISHED.equals(o.state()), "as a published record");

        o = ProofProtocol.parseSubmit(502, "{\"accepted\":true,\"published\":false,\"proof\":\"p-2\","
                + "\"log\":\"\",\"error\":\"ZENODO_API_KEY is not set\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.NOT_PUBLISHED && o.verified(), "no key: verified, not published");
        check(o.describe().contains("ZENODO_API_KEY"), "the reason reaches the user");
        check(ProofProtocol.STATE_VERIFIED.equals(o.state()), "the record is verified, not queued");

        o = ProofProtocol.parseSubmit(503, "{\"accepted\":true,\"queued\":false,\"proof\":\"p-3\","
                + "\"log\":\"\",\"error\":\"queue full\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.QUEUE_UNAVAILABLE, "a full queue is its own answer");

        o = ProofProtocol.parseSubmit(429, "rate limited: at most 10 requests per 60s\n", true);
        check(o.kind == ProofProtocol.Outcome.Kind.RATE_LIMITED, "the per-minute plain-text limit");
        check(o.message.startsWith("rate limited: at most 10"), "keeps the backend's sentence");
        o = ProofProtocol.parseSubmit(429, "{\"error\":\"daily-cap\",\"message\":\"daily cap: at most 5\\n\"}", true);
        check(o.kind == ProofProtocol.Outcome.Kind.RATE_LIMITED && o.message.startsWith("daily cap"),
                "the daily cap's JSON message");

        o = ProofProtocol.parseSubmit(400, "mm0 spec is required\n", false);
        check(o.kind == ProofProtocol.Outcome.Kind.BAD_REQUEST && o.message.equals("mm0 spec is required"),
                "a plain-text 400 is a refusal");
        check(ProofProtocol.parseSubmit(500, "cannot save the project index: disk full\n", true).kind
                == ProofProtocol.Outcome.Kind.ERROR, "a 500 is an error");
        check(ProofProtocol.parseSubmit(200, "<html>Accepted</html>", false).kind
                == ProofProtocol.Outcome.Kind.ERROR, "an HTML page (no Accept honoured) is not a verdict");
        check(ProofProtocol.parseSubmit(-1, "timeout", false).kind == ProofProtocol.Outcome.Kind.ERROR,
                "no response is an error");
        check(ProofProtocol.parseSubmit(202, "{\"accepted\":true,\"queued\":true}", true).kind
                == ProofProtocol.Outcome.Kind.ERROR, "a queued answer without an id is not usable");
        check(ProofProtocol.parseSubmit(202, "{\"accepted\":true,\"queued\":true,\"proof\":\"a/../b\"}", true).kind
                == ProofProtocol.Outcome.Kind.ERROR, "an id that is not a slug is refused");
        check(ProofProtocol.parseSubmit(200, "{\"accepted\":true,\"accepted\":false}", false).kind
                == ProofProtocol.Outcome.Kind.ERROR, "duplicate keys are not the protocol");
    }

    private static void proofRecords() {
        ProofProtocol.Record r = ProofProtocol.parseProof(200, "{\"id\":\"p-1\",\"title\":\"t\",\"fingerprint\":\"f\","
                + "\"seq\":1,\"verified\":true,\"record\":647037,\"state\":\"published\","
                + "\"doi\":\"10.5072/zenodo.647037\",\"url\":\"https://sandbox.zenodo.org/records/647037\","
                + "\"status\":\"/status\",\"mm0\":\"sort Prop;\\n\",\"mm1\":\"sort Prop;\\n\"}", "p-1");
        check(ProofProtocol.STATE_PUBLISHED.equals(r.state) && r.settled(), "a published record is settled");
        check(r.doi.equals("10.5072/zenodo.647037"), "with its DOI");
        r = ProofProtocol.parseProof(200, "{\"id\":\"p-1\",\"state\":\"queued\",\"doi\":null,\"url\":null,"
                + "\"record\":null}", "p-1");
        check(!r.settled() && r.doi.isEmpty(), "a queued record keeps being followed; null DOI is empty");
        check(ProofProtocol.parseProof(200, "{\"id\":\"p-1\",\"state\":\"publish-failed\"}", "p-1").settled(),
                "a failed publication stops the poll");
        check(refused(() -> ProofProtocol.parseProof(404, "{\"error\":\"no such proof\"}", "p-1")),
                "a missing record is reported");
        check(refused(() -> ProofProtocol.parseProof(200, "{\"id\":\"p-2\",\"state\":\"published\"}", "p-1")),
                "an answer for another proof is refused");
        check(refused(() -> ProofProtocol.parseProof(200, "{\"id\":\"p-1\"}", "p-1")),
                "a record without a state is refused");
        check(refused(() -> ProofProtocol.parseProof(500, "boom", "p-1")), "a server error is reported");
        check(refused(() -> ProofProtocol.parseProof(200, "not json", "p-1")), "garbage is reported");
    }

    private static void serverAddresses() {
        check("https://proofs.example.org".equals(ProofProtocol.checkBase(" https://proofs.example.org/ ", false)),
                "https is accepted and normalized");
        check("https://example.org:8443/mm0".equals(ProofProtocol.checkBase("https://example.org:8443/mm0//", false)),
                "a port and a reverse-proxy prefix are kept");
        check(refused(() -> ProofProtocol.checkBase("http://127.0.0.1:18919", false)),
                "the shipping app refuses cleartext, loopback included");
        check("http://127.0.0.1:18919".equals(ProofProtocol.checkBase("http://127.0.0.1:18919", true)),
                "the test package may use a local cleartext fixture");
        check(refused(() -> ProofProtocol.checkBase("", false)), "an empty address asks to be set");
        check(refused(() -> ProofProtocol.checkBase("ftp://example.org", true)), "other schemes are refused");
        check(refused(() -> ProofProtocol.checkBase("https://user:pw@example.org", false)),
                "credentials in the address are refused");
        check(refused(() -> ProofProtocol.checkBase("https://example.org/?x=1", false)), "a query is refused");
        check(refused(() -> ProofProtocol.checkBase("https://example.org/#f", false)), "a fragment is refused");
        check(refused(() -> ProofProtocol.checkBase("https://exa mple.org", false)), "spaces are refused");
        check(refused(() -> ProofProtocol.checkBase("https:///path", false)), "a missing host is refused");
        check("unfer".equals(ProofProtocol.checkProject(" unfer ")), "a project slug is trimmed");
        check(refused(() -> ProofProtocol.checkProject("a&b")), "a project cannot smuggle a parameter");
        check(refused(() -> ProofProtocol.checkProject("")), "a project is required");
    }

    private static void formAndLinks() {
        String body = ProofProtocol.formBody("unfer", "sort Prop", "sort Prop;\n", null);
        check(body.equals("project=unfer&title=sort+Prop&mm0=sort+Prop%3B%0A&mm1=sort+Prop%3B%0A"),
                "the form body encodes and falls back to the spec as the proof: " + body);
        check(ProofProtocol.formBody("p", "t", "a&b=c", "∀").contains("mm0=a%26b%3Dc&mm1=%E2%88%80"),
                "reserved and non-ASCII characters are encoded");
        check("peano".equals(ProofProtocol.titleFor("peano.mm0")), "the title drops .mm0");
        check("peano".equals(ProofProtocol.titleFor("primary:Download/peano.MM0")), "and a provider path");
        check("proof".equals(ProofProtocol.titleFor("")), "an unnamed file still has a title");
        check("https://doi.org/10.5072/zenodo.1".equals(
                ProofProtocol.resultLink("https://b", "unfer", "p", "10.5072/zenodo.1", "https://z/records/1")),
                "a DOI resolves through doi.org");
        check("https://z/records/1".equals(ProofProtocol.resultLink("https://b", "unfer", "p", "", "https://z/records/1")),
                "else the archive record");
        check("https://b/status?project=unfer&id=p-1".equals(ProofProtocol.resultLink("https://b", "unfer", "p-1", "", "")),
                "else the backend's status page");
        check("https://b/status?project=unfer&id=p-1".equals(
                ProofProtocol.resultLink("https://b", "unfer", "p-1", "", "http://insecure/records/1")),
                "a cleartext record URL is not handed to the note");
        check("/proof?project=unfer&id=a+b".equals(ProofProtocol.proofPath("unfer", "a b")), "the proof path encodes");
        check(ProofProtocol.validId("emthin-sb-20261121135454-3307780-5a720607"), "a real proof id is valid");
        check(!ProofProtocol.validId("a b") && !ProofProtocol.validId("a?b") && !ProofProtocol.validId(""),
                "ids with spaces, query characters or nothing are not");
        check(ProofProtocol.describeState("queued", "").contains("publishing"), "a queued record reads as publishing");
        check(ProofProtocol.describeState("published", "10.1/x").contains("10.1/x"), "a published record shows its DOI");
    }
}
