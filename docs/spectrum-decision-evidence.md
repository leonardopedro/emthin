# SpectrumOS decision-record excerpt — licence-routed evidence

**Why this file lives in `emthin/`:** the workspace policy (see
`../ATTRIBUTION.md`, "Licence routing policy") keeps only MIT/Apache-2.0
(or equivalent) derived material in the permissively licensed projects.
The SpectrumOS decision records are **not** MIT/Apache — their SPDX
headers read `GFDL-1.3-no-invariants-or-later OR CC-BY-SA-4.0` — so any
verbatim excerpt from them is kept here, in this GPL-3.0 project, and the
australVM plan paraphrases instead of quoting.

Source: `../spectrum/Documentation/doc/decisions/005-virtual-machine-monitor.adoc`
(read firsthand 2026-10-18).

```
// SPDX-FileCopyrightText: 2022 Unikie
// SPDX-License-Identifier: GFDL-1.3-no-invariants-or-later OR CC-BY-SA-4.0
```

Verbatim excerpt (licence: GFDL-1.3-no-invariants-or-later OR
CC-BY-SA-4.0, © 2022 Unikie):

> Spectrum will use cloud-hypervisor as the primary VMM

This is the only verbatim text from SpectrumOS's non-permissively licensed
material anywhere in the workspace; everything else taken from SpectrumOS
is either factual reference (paths, patch file names, protocol constants)
or derived from its permissively licensed files (kconfig intent from
`vm/sys/net/default.nix`, SPDX MIT; vhost-user wire layouts from the
rust-vmm `vhost` patches, Apache-2.0 AND LicenseRef-BSD-3-Clause-Google;
the cloud-hypervisor REST surface from `docs/api.md`, CC-BY-4.0 with
attribution).
