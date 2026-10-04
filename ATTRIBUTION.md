# Attribution

Borrowed patterns and components that concern *this* repo. The authoritative,
cross-repo table — including what was deliberately **not** adopted — is
[`../ATTRIBUTION.md`](../ATTRIBUTION.md).

| source | licence | what was adapted | where it landed |
|---|---|---|---|
| smithay (fork `ningxilai/smithay`, branch `emskin-patches`) | MIT | Wayland compositor substrate; the x11/text_input/seat_data patches are carried on the branch | the compositor itself — `AGENTS.md` "smithay is a fork" |
| driftwm (sibling repo, same author) | GPL-3.0 | canvas mechanics for the document desktop | `handlers/apps.rs`, `docs/REWRITE_PLAN.md` |
| velyst (git dep, tag `v0.0.1`) | see upstream | page rasterisation for the document layer | `doc_render.rs` |

## Notes

This repo is GPL-3.0. It receives Apache-2.0 and MIT code without
incompatibility, and none of the rows above re-licence anything: smithay stays
MIT, driftwm stays GPL.

The substrate deserves one warning that is easy to lose: `smithay` is a **fork
with patches**, and `AGENTS.md` records that reading a clean upstream checkout
will not match what this binary links against. Three patched files each hide a
bug described under "Key Gotchas"
