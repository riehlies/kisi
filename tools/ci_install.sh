#!/bin/sh
# Package installs of the CI (.github/workflows), with retries, time limits and a GitHub annotation per phase
# (`::notice::` with its duration), because the run logs are not always at hand and the downloads – not the
# checks – were what made runs slow or hang (apt mirrors, Playwright's browsers; the KiCad PPA, which is why the
# KiCad check left the CI).
#
#   sh tools/ci_install.sh ngspice            ngspice; .deb files kept in ~/apt-debs (cached)
#   sh tools/ci_install.sh playwright NAME    Playwright's NAME browser (chromium, firefox, webkit) with its
#                                             system packages; the Python package is installed by the caller
#
# A stalled apt phase is stopped, apt's leftovers are cleared (a killed apt-get keeps its lock and would fail every
# retry at once), and the retry uses the next Ubuntu mirror: the runners' default (azure.archive.ubuntu.com) stalled
# for over 20 minutes in some runs on 26 Oct 2026.
set -u
APT_OPTS="-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o DPkg::Lock::Timeout=180"
MIRRORS="http://archive.ubuntu.com/ubuntu/ http://mirrors.edge.kernel.org/ubuntu/"

LOG="${RUNNER_TEMP:-/tmp}/ci_install.log"
now() { date +%s; }
note() { echo "::notice title=ci_install::$1"; }

# the last download line of the log: which host a stalled or failed phase was fetching from
last_fetch() { grep -E '^(Get|Err|Ign|Hit):' "$LOG" 2>/dev/null | tail -n 1 | cut -c1-160; }

# stop what a killed phase left running and release apt's locks
apt_recover() {
  sudo pkill -9 -x apt-get 2>/dev/null
  sudo pkill -9 -x apt 2>/dev/null
  sudo pkill -9 -f '/usr/lib/apt/methods/' 2>/dev/null
  sleep 2
  sudo pkill -9 -x dpkg 2>/dev/null
  sudo rm -f /var/lib/dpkg/lock-frontend /var/lib/dpkg/lock /var/cache/apt/archives/lock /var/lib/apt/lists/lock
  sudo dpkg --configure -a > /dev/null 2>&1
}

# switch the Ubuntu archive to the next mirror of MIRRORS and refresh the package lists
apt_next_mirror() {
  m=$(echo "$MIRRORS" | cut -d' ' -f1)
  MIRRORS="$(echo "$MIRRORS" | cut -s -d' ' -f2-) $m"
  # GitHub's images list their mirrors in /etc/apt/apt-mirrors.txt (sources: mirror+file:...), others name them
  [ -f /etc/apt/apt-mirrors.txt ] && echo "$m" | sudo tee /etc/apt/apt-mirrors.txt > /dev/null
  for f in /etc/apt/sources.list.d/ubuntu.sources /etc/apt/sources.list; do
    [ -f "$f" ] && sudo sed -i -E "s#https?://[a-z0-9.-]*(archive|mirrors?)[a-z0-9.-]*/ubuntu/?#$m#g" "$f"
  done
  echo "::warning title=ci_install::Ubuntu mirror switched to $m"
  # shellcheck disable=SC2086
  timeout -k 10 180 sudo apt-get $APT_OPTS update > "$LOG" 2>&1 || echo "::warning title=ci_install::apt-get update on $m failed: $(last_fetch)"
}

# phase NAME LIMIT_S TRIES APT CMD...: run CMD up to TRIES times, each at most LIMIT_S seconds; annotate the outcome.
# APT=1: CMD uses apt – between tries, clear apt's leftovers and use the next mirror.
phase() {
  name=$1 limit=$2 tries=$3 apt=$4
  shift 4
  i=1
  while [ "$i" -le "$tries" ]; do
    t0=$(now)
    if timeout -k 10 "$limit" "$@" > "$LOG" 2>&1; then
      note "$name: $(($(now) - t0)) s (attempt $i)"
      return 0
    fi
    echo "::warning title=ci_install::$name: attempt $i failed or took more than ${limit} s ($(($(now) - t0)) s); last fetch: $(last_fetch)"
    tail -n 20 "$LOG"
    if [ "$apt" = 1 ] && [ "$i" -lt "$tries" ]; then
      apt_recover
      apt_next_mirror
    fi
    i=$((i + 1))
  done
  echo "::error title=ci_install::$name: failed after $tries attempts"
  return 1
}

case "${1:-}" in
  ngspice)
    mkdir -p "$HOME/apt-debs/partial"
    note "ngspice: $(find "$HOME/apt-debs" -name '*.deb' | wc -l) cached .deb files"
    # shellcheck disable=SC2086
    phase "apt-get update" 180 3 1 sudo apt-get $APT_OPTS update || exit 1
    # shellcheck disable=SC2086
    phase "apt-get install ngspice" 420 3 1 sudo apt-get $APT_OPTS install -y --no-install-recommends \
      -o Dir::Cache::Archives="$HOME/apt-debs" ngspice || exit 1
    sudo chown -R "$(id -u):$(id -g)" "$HOME/apt-debs"
    rm -rf "$HOME/apt-debs/partial" "$HOME/apt-debs/lock"
    ;;
  playwright)
    b=${2:?browser name}
    # system packages (apt) and the browser download (Playwright's CDN) separately, to see which one is slow.
    # Chromium runs with what GitHub's Ubuntu image has (it carries Chrome); its install-deps pulled ~100 MB of
    # fonts and codecs from the stalling mirror.
    if [ "$b" != chromium ]; then
      phase "playwright install-deps $b" 420 3 1 python3 -m playwright install-deps "$b" || exit 1
    fi
    phase "playwright install $b" 300 3 0 python3 -m playwright install "$b" || exit 1
    ;;
  *)
    sed -n '2,12p' "$0"
    exit 2
    ;;
esac
