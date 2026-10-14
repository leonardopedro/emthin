# workspace-backup — a mirror, not a source

Byte-identical copies of the **workspace-root coordination files**
(`../ATTRIBUTION.md` at the root, the `PLAN_*`/`PROJECT_REVIEW_*` plan docs,
`docs/user-visible-errors.csv`, and the workspace `scripts/` gates), mirrored
into this git checkout because **the workspace root is not a git repository**
and those files would otherwise have no version control anywhere.

**Copy, never move** (owner decision, 2026-10-14): every link, gate and
documented invocation assumes the files live at the root — seven repos link
`../ATTRIBUTION.md`, `check-all-gates` runs `scripts/check-*` from the root,
the cron line does `cd /home/leo/Projects && bash scripts/check-all-gates`.
Moving them would break all of it; the mirror gives them git history without
touching a single path.

## Contract

- **The root files are authoritative.** Edit there, then refresh:

  ```sh
  bash scripts/sync-workspace-backup     # from the workspace root: copy + verify
  bash scripts/check-backup-sync         # verify only (wired into check-all-gates)
  ```

  and commit the updated mirror.

- **Drift fails loudly.** `scripts/check-backup-sync` (auto-discovered by
  `check-all-gates`) diffs every file in `scripts/workspace-backup-files`
  against its source; a committed mirror that no longer matches fails the
  workspace sweep until re-synced.

- **Two gates skip this directory on purpose**, because scanning a duplicate
  of already-scanned text produces verdicts that depend on where the copy
  sits, not on what it says:
  - `emthin/scripts/check-links` — the plan docs' links are root-relative
    (`../celld`, `../driftwm`, …) and cannot resolve from inside `emthin/`;
    their targets are the root's business, not this repo's.
  - `scripts/check-attribution` — every claim here is checked at its source
    (the root plan docs are scanned as root docs); `workspace-backup` is
    excluded so identical text is not resolved against a different window.

Both exclusions carry the reasoning in the script itself. What they cost in
double-checking they pay back as the drift gate above: this mirror is either
byte-identical to the gated source or the sweep is red.
