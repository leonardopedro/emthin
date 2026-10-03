#!/usr/bin/env bash
# Step 7, configuration `none`: no keyboard injection at all.
#
# The clipboard half of step 7 does not need keys, so it still runs. This
# backend exists so `--config none` is a real, runnable choice rather than a
# special case the driver has to keep handling: the checks are written against
# the injection seam, so "no backend" is just a backend that declines.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

inject_available() { return 1; }
inject_note() {
  printf 'no injection; the clipboard checks run, the keyboard half does not'
}
inject_type() { return 1; }
inject_key()  { return 1; }