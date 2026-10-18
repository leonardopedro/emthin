#!/usr/bin/env bash
# waydroid-sudo-fixes.sh — everything from the waydroid fix that needs root.
#
#   sudo ./scripts/waydroid-sudo-fixes.sh [--reboot]
#
# Sections:
#   A. waydroid shell network proof.
#      `waydroid shell` refuses to run unless euid==0 (it execs into the LXC
#      container via lxc-attach), so these are exactly the checks the
#      unprivileged agent could not run. Note the CLI does NOT propagate the
#      inner command's exit status (it uses subprocess.run without check),
#      so every assertion greps for expected output instead of using $?.
#      A FROZEN container is auto-unfrozen by `waydroid shell` and re-frozen
#      afterwards, so the idle power-save state does not matter.
#   B. Commit the waydroid-nftables change in /etc/nixos (its .git is
#      root-owned, which is why the agent could not commit it). Only
#      configuration.nix is committed; the three pre-existing untracked
#      files are left alone. Idempotent: no-op when already committed.
#   C. Report booted-vs-system kernel drift (booted 7.2.8 vs system 7.2.9).
#      Harmless for waydroid (the nft path loads no modules), but optional
#      kernel modules stay mismatched until reboot. --reboot restarts into
#      the system kernel — only when you are ready; it ends your session.
#   D. emthin pad app proof. `pm`/`am`/`logcat` all exec into the container,
#      so they need euid 0. Cold-starts the activity (force-stop → clear
#      logcat → am start -W) and asserts three independent signals: the
#      package resolves, logcat shows "Displayed com.emthin.pad/…" (first
#      frame drawn → System.loadLibrary succeeded), and no crash dump for
#      the package landed in the freshly cleared buffer. Skipped unless the
#      APK was installed first (see emthin/emthin-android/build-apk.sh).
#
# --repair-net: if the Section A ping checks fail, restart the waydroid
#      session as the session user (root cannot drive a user systemd unit
#      without the user's bus) and re-check. This fixes the observed
#      failure mode: the container's DHCP renewal (T1) lands inside an
#      idle freeze, netd loses the address on thaw and never re-DISCOVERs.
#      Kills running Android apps — same effect as rebooting the phone.
#
# Exit status: 0 = every check and the commit passed, 1 = something failed,
# 2 = bad usage.

set -u
PATH=/run/current-system/sw/bin:$PATH
export PATH

FAILURES=0
REBOOT=0
REPAIR_NET=0

usage() {
    cat <<EOF
usage: sudo $0 [--reboot] [--repair-net]

  --reboot     after the checks, reboot into the system kernel when the
               booted kernel drifts from it (ends your graphical session)
  --repair-net on network-check failure, restart the waydroid session and
               re-check (kills running Android apps)
EOF
}

for arg in "$@"; do
    case "$arg" in
        --reboot) REBOOT=1 ;;
        --repair-net) REPAIR_NET=1 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $arg" >&2; usage >&2; exit 2 ;;
    esac
done

if [ "$(id -u)" -ne 0 ]; then
    echo "must run as root:  sudo $0" >&2
    exit 1
fi

say()  { printf '%s\n' "$*"; }
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILURES=$((FAILURES + 1)); }

# check_shell LABEL EXPECTED_REGEX COMMAND...
# Runs COMMAND, greps EXPECTED_REGEX in its combined output (the waydroid
# CLI swallows the inner exit code, so output is the only reliable signal).
check_shell() {
    local label=$1 expect=$2 out
    shift 2
    out=$(timeout 45 "$@" 2>&1)
    if printf '%s\n' "$out" | grep -qE "$expect"; then
        pass "$label"
    else
        fail "$label"
        say "     cmd: $*"
        say "     out: $(printf '%s' "$out" | tr '\n' ' ' | cut -c1-300)"
    fi
}

# network_checks — the three in-container ping probes (need euid 0).
network_checks() {
    check_shell "ping host bridge 192.168.240.1 (nft input chain)" \
        '2 packets transmitted, 2 received' \
        waydroid shell -- ping -c 2 -W 2 192.168.240.1
    check_shell "ping internet 1.1.1.1 (nft masquerade/NAT)" \
        '2 packets transmitted, 2 received' \
        waydroid shell -- ping -c 2 -W 3 1.1.1.1
    check_shell "DNS via dnsmasq (ping dns.google)" \
        '2 packets transmitted, 2 received' \
        waydroid shell -- ping -c 2 -W 4 dns.google
}

# repair_waydroid_session — restart the session as the session user over
# her own D-Bus/systemd user bus, then wait for Android to finish booting.
repair_waydroid_session() {
    local suser sdir tries
    suser=${SUDO_USER:-leo}
    sdir=$(id -u "$suser" 2>/dev/null || echo 1000)
    say "repair: restarting waydroid session as $suser (container DHCP stuck) ..."
    if ! runuser -u "$suser" -- env \
            XDG_RUNTIME_DIR="/run/user/$sdir" \
            DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$sdir/bus" \
            systemctl --user restart waydroid-session; then
        fail "restart waydroid-session as $suser"
        return 1
    fi
    # Android needs ~30s to reach boot_completed; poll instead of guessing.
    tries=0
    while [ "$tries" -lt 18 ]; do
        if timeout 20 waydroid shell -- getprop sys.boot_completed 2>/dev/null \
                | grep -q '^1'; then
            return 0
        fi
        sleep 5
        tries=$((tries + 1))
    done
    fail "android reached boot_completed after session restart"
    return 1
}

# ---- A. waydroid shell (in-container) checks ----------------------------
say "=== A. waydroid shell checks (need euid 0) ==="

if ! command -v waydroid >/dev/null 2>&1; then
    fail "waydroid on PATH"
else
    status_out=$(waydroid status 2>&1)
    if printf '%s\n' "$status_out" | grep -q 'Session:.*RUNNING'; then
        pass "session RUNNING"
        check_shell "android fully booted (getprop sys.boot_completed = 1)" \
            '^1[[:space:]]*$' \
            waydroid shell -- getprop sys.boot_completed
        net_before=$FAILURES
        network_checks
        if [ "$FAILURES" -gt "$net_before" ] && [ "$REPAIR_NET" -eq 1 ]; then
            say "note: network checks failed — attempting --repair-net"
            first_fails=$((FAILURES - net_before))
            if repair_waydroid_session; then
                # Verdict is the post-repair state; the first-pass FAIL
                # lines above stay in the log for the record.
                FAILURES=$net_before
                say "     re-checking after repair ($first_fails check(s) failed before)"
                network_checks
            fi
        elif [ "$FAILURES" -gt "$net_before" ]; then
            say "     hint: re-run with --repair-net to restart the waydroid session;"
            say "           container DHCP goes stale when a T1 renewal falls inside an idle freeze."
        fi
    else
        fail "session RUNNING"
        say "     start it first as user leo: systemctl --user start waydroid-session"
        say "     status: $(printf '%s' "$status_out" | tr '\n' ' ')"
    fi
fi

# ---- B. commit the /etc/nixos configuration change ----------------------
say ""
say "=== B. commit /etc/nixos configuration change ==="

if git -C /etc/nixos status --porcelain -- configuration.nix | grep -q .; then
    name=$(git -C /etc/nixos config user.name || true)
    email=$(git -C /etc/nixos config user.email || true)
    if [ -z "${name:-}" ] || [ -z "${email:-}" ]; then
        name=$(git -C /etc/nixos log -1 --format=%an)
        email=$(git -C /etc/nixos log -1 --format=%ae)
    fi
    msg=$(mktemp /tmp/waydroid-commit.XXXXXX)
    cat >"$msg" <<'MSG'
nixos update: switch waydroid to nftables backend

The booted kernel ships no ip_tables modules, so waydroid-net.sh's
iptables-legacy path always failed ("Table does not exist") and the
session never reached RUNNING. Use pkgs.waydroid-nftables instead:
USE_NFTABLES=1 flips LXC_USE_NFT=true and puts nft on the wrapper
PATH. The host firewall backend is left untouched.

🤖 Generated with Codebuff
Co-Authored-By: Codebuff <noreply@codebuff.com>
MSG
    if git -C /etc/nixos add configuration.nix \
        && git -C /etc/nixos -c user.name="$name" -c user.email="$email" \
           commit -F "$msg" -- configuration.nix; then
        pass "configuration.nix committed"
    else
        fail "git commit of configuration.nix"
    fi
    rm -f "$msg"
else
    pass "configuration.nix already committed"
fi

# ---- C. booted-vs-system kernel drift -----------------------------------
say ""
say "=== C. kernel drift ==="

booted=$(uname -r)
sysk=$(readlink -f /run/current-system/kernel 2>/dev/null || true)
syskver=""
if [ -n "$sysk" ]; then
    d=$(basename "$(dirname "$sysk")")   # e.g. <hash>-linux-7.2.9
    case "$d" in
        *-linux-*) syskver=${d#*-linux-} ;;
    esac
fi

if [ -z "$syskver" ]; then
    say "info booted=$booted ; system kernel path=$sysk (version not parsed)"
elif [ "$booted" = "$syskver" ]; then
    pass "booted kernel $booted matches the system profile"
else
    say "note booted kernel: $booted  !=  system kernel: $syskver"
    say "     waydroid works either way (its nft path loads no modules),"
    say "     but out-of-tree/optional modules stay mismatched until reboot."
    if [ "$REBOOT" -eq 1 ]; then
        say "rebooting into the system kernel now (--reboot) ..."
        systemctl reboot
        exit 0
    fi
    say "     re-run with --reboot when convenient."
fi

# ---- D. emthin pad app (launch + logcat proof) --------------------------
say ""
say "=== D. emthin pad app ==="

if ! timeout 45 waydroid shell -- pm path com.emthin.pad 2>&1 \
        | grep -qE '^package:.*com\.emthin\.pad'; then
    say "info com.emthin.pad not installed — skipping app checks"
    say "     build+install: cd ~/Projects/emthin/emthin-android && ./build-apk.sh"
    say "                     waydroid app install build/emthin-pad.apk"
else
    pass "com.emthin.pad installed (pm path)"

    # Cold start: stop the app, wipe the logcat ring buffer, then
    # `am start -W` (blocks until the launch finishes). A cleared buffer
    # means every crash marker below is from *this* launch.
    timeout 45 waydroid shell -- am force-stop com.emthin.pad >/dev/null 2>&1
    timeout 45 waydroid logcat -- -c >/dev/null 2>&1
    check_shell "am start -W com.emthin.pad/.MainActivity" \
        'Status: ok|Warning: Activity not started' \
        waydroid shell -- am start -W --user 0 -n com.emthin.pad/.MainActivity
    sleep 2

    log_out=$(timeout 45 waydroid logcat -- -d -t 800 2>&1)

    # First frame drawn → the JNI lib loaded and onCreate returned.
    if printf '%s\n' "$log_out" | grep -q 'Displayed com\.emthin\.pad/'; then
        pass "MainActivity displayed (logcat: Displayed com.emthin.pad)"
    else
        fail "MainActivity displayed (no 'Displayed com.emthin.pad' line)"
    fi

    # Any AndroidRuntime dump / link error for our package in the fresh buffer.
    if printf '%s\n' "$log_out" \
            | grep -qE 'Process: com\.emthin\.pad|UnsatisfiedLinkError|ActivityNotFound'; then
        fail "no crash for com.emthin.pad in logcat"
        say "     $(printf '%s' "$log_out" \
            | grep -E 'Process: com\.emthin\.pad|UnsatisfiedLinkError|FATAL EXCEPTION' \
            | tail -3 | tr '\n' ' ' | cut -c1-300)"
    else
        pass "no crash for com.emthin.pad in logcat"
    fi

    # Independent host-side signal: the Android process is alive in /proc
    # (container namespaces do not hide it from the host).
    if pgrep -f '^com\.emthin\.pad$' >/dev/null 2>&1; then
        pass "app process alive on host (pid $(pgrep -f '^com\.emthin\.pad$' | tr '\n' ' '))"
    else
        fail "app process alive on host"
    fi
fi

# ---- summary ------------------------------------------------------------
say ""
if [ "$FAILURES" -eq 0 ]; then
    say "all checks passed"
else
    say "$FAILURES check(s) FAILED"
fi
exit $((FAILURES > 0 ? 1 : 0))
