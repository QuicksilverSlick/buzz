#!/usr/bin/env bash
# =============================================================================
# ci-apt-retry.sh — run an apt-backed CI command with a per-attempt deadline
# =============================================================================
# Ubuntu mirrors on hosted runners are sometimes degraded: most requests hit
# apt's Acquire timeouts and apt keeps re-queuing them, so one apt step can run
# past the job timeout without failing. This kills each attempt after a
# deadline, stops any apt-get/dpkg the attempt left behind, and retries.
# Downloaded .debs stay in /var/cache/apt/archives, so a retry resumes where
# the last attempt stopped.
#
# Between attempts it also repairs packages a killed dpkg left half-unpacked
# (`dpkg --configure -a` alone can't, and every later install then fails on
# unmet dependencies), backs off, demotes the mirror apt tries first in
# /etc/apt/apt-mirrors.txt, and refreshes the package index. apt fails over to
# the next mirror on errors but never on a mirror that is merely slow, and a
# fresh index clears version skew from a mirror caught mid-sync. Each step has
# its own deadline, so this bounds each invocation (about 31m with the
# defaults). Size job timeouts to fit the bound of every invocation in the job
# plus the job's real work. An attempt that passes skips all of this.
#
# Ephemeral hosted runners only: between attempts it stops every apt-get/dpkg
# on the machine, not just the ones it started. Don't use it on a self-hosted
# runner or a developer machine.
#
# It also writes the apt retry/timeout options to apt.conf.d, so commands that
# call apt-get themselves (e.g. `playwright install-deps`) pick them up too.
#
# Usage:
#   scripts/ci-apt-retry.sh <command> [args...]
#
# Env:
#   CI_APT_ATTEMPTS         Attempts before failing (default: 3).
#   CI_APT_ATTEMPT_SECONDS  Deadline per attempt (default: 420). Healthy apt
#                           steps finish in under 4 minutes; degraded ones run
#                           past 13.
#   CI_APT_REPAIR_SECONDS   Deadline for each repair and index refresh step
#                           between attempts (default: 60).
#   CI_APT_BACKOFF_SECONDS  Pause before retry N is N times this (default: 15).
#   CI_APT_MIRRORS          Mirror list to rotate between attempts (default:
#                           /etc/apt/apt-mirrors.txt; skipped if missing).
# =============================================================================
set -euo pipefail

attempts="${CI_APT_ATTEMPTS:-3}"
deadline="${CI_APT_ATTEMPT_SECONDS:-420}"
repair_deadline="${CI_APT_REPAIR_SECONDS:-60}"
backoff="${CI_APT_BACKOFF_SECONDS:-15}"
mirrors="${CI_APT_MIRRORS:-/etc/apt/apt-mirrors.txt}"

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

printf '%s\n' \
  'Acquire::Retries "3";' \
  'Acquire::http::Timeout "30";' \
  'Acquire::https::Timeout "30";' \
  'DPkg::Lock::Timeout "120";' |
  sudo tee /etc/apt/apt.conf.d/80-ci-timeouts >/dev/null

# Intentionally machine-wide: any other apt-get/dpkg would hold the same lock.
reap_package_managers() {
  sudo pkill -TERM -x 'apt-get|dpkg' || true
  for _ in $(seq 1 15); do
    pgrep -x 'apt-get|dpkg' >/dev/null || return 0
    sleep 1
  done
  sudo pkill -KILL -x 'apt-get|dpkg' || true
}

# apt tries the mirror with the lowest priority:N first. Give that one a
# priority past the others, so each retry starts on a different mirror.
rotate_mirrors() {
  local rotated
  rotated=$(awk '
    match($0, /priority:[0-9]+/) {
      p = substr($0, RSTART + 9, RLENGTH - 9) + 0
      if (first == "" || p < min) { min = p; first = NR }
      if (p > max) max = p
    }
    { line[NR] = $0 }
    END {
      if (first != "") sub(/priority:[0-9]+/, "priority:" (max + 1), line[first])
      for (i = 1; i <= NR; i++) print line[i]
    }' "$mirrors" 2>/dev/null) && [ -n "$rotated" ] || return 0
  printf '%s\n' "$rotated" | sudo tee "$mirrors" >/dev/null
}

for attempt in $(seq 1 "$attempts"); do
  rc=0
  timeout --kill-after=15s "$deadline" "$@" || rc=$?
  [ "$rc" -eq 0 ] && exit 0
  if [ "$rc" -eq 124 ] || [ "$rc" -eq 137 ]; then
    echo "::warning::attempt $attempt/$attempts timed out after ${deadline}s: $*"
  else
    echo "::warning::attempt $attempt/$attempts failed (exit $rc): $*"
  fi
  # timeout(1) signals only its own process group, but sudo runs its command
  # in a new one. When apt-get is not sudo's direct command (playwright
  # install-deps runs `sudo sh -c "apt-get update && apt-get install ..."`),
  # the forwarded signal stops sh and apt-get survives, holding the dpkg lock
  # so every retry is a lock wait. Stop any survivor before retrying, and
  # before giving up so no apt-get outlives the step.
  reap_package_managers
  [ "$attempt" -eq "$attempts" ] && break
  # A killed apt/dpkg run can leave dpkg half-configured, or a package
  # half-unpacked that only apt-get -f install (from the cached .debs) fixes.
  # Repair before retrying, each step under its own deadline so a stuck repair
  # can't eat the job.
  timeout --kill-after=15s "$repair_deadline" sudo dpkg --configure -a || true
  timeout --kill-after=15s "$repair_deadline" sudo apt-get -f install -y || true
  # Give a degraded or mid-sync mirror time, retry on another one, and resolve
  # the retry against a fresh index.
  sleep $((backoff * attempt))
  rotate_mirrors || true
  timeout --kill-after=15s "$repair_deadline" sudo apt-get update || true
done
echo "::error::all $attempts attempts failed: $*"
exit 1
