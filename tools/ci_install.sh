#!/bin/sh
# Package installs of the CI (.github/workflows), with retries, time limits and a GitHub annotation per phase
# (`::notice::` with its duration), because the run logs are not always at hand and the downloads – not the
# checks – were what made runs slow or hang (apt mirrors, the KiCad PPA, Playwright's browsers).
#
#   sh tools/ci_install.sh kicad              ngspice and KiCad 9 (PPA); .deb files kept in ~/apt-debs (cached)
#   sh tools/ci_install.sh playwright NAME    Playwright's NAME browser (chromium, firefox, webkit) with its
#                                             system packages; the Python package is installed by the caller
set -u
APT_OPTS="-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o DPkg::Lock::Timeout=180"

LOG="${RUNNER_TEMP:-/tmp}/ci_install.log"
now() { date +%s; }
note() { echo "::notice title=ci_install::$1"; }

# phase NAME LIMIT_S TRIES CMD...: run CMD up to TRIES times, each at most LIMIT_S seconds; annotate the outcome
phase() {
  name=$1 limit=$2 tries=$3
  shift 3
  i=1
  while [ "$i" -le "$tries" ]; do
    t0=$(now)
    if timeout "$limit" "$@" > "$LOG" 2>&1; then
      note "$name: $(($(now) - t0)) s (attempt $i)"
      return 0
    fi
    echo "::warning title=ci_install::$name: attempt $i failed or took more than ${limit} s ($(($(now) - t0)) s)"
    tail -n 20 "$LOG"
    i=$((i + 1))
  done
  echo "::error title=ci_install::$name: failed after $tries attempts"
  return 1
}

case "${1:-}" in
  kicad)
    mkdir -p "$HOME/apt-debs/partial"
    note "kicad: $(find "$HOME/apt-debs" -name '*.deb' | wc -l) cached .deb files"
    phase "add the KiCad 9 PPA" 180 3 sudo add-apt-repository -y ppa:kicad/kicad-9.0-releases || exit 1
    # shellcheck disable=SC2086
    phase "apt-get update" 180 3 sudo apt-get $APT_OPTS update || exit 1
    # shellcheck disable=SC2086
    phase "apt-get install ngspice kicad" 420 3 sudo apt-get $APT_OPTS install -y --no-install-recommends \
      -o Dir::Cache::Archives="$HOME/apt-debs" ngspice kicad || exit 1
    sudo chown -R "$(id -u):$(id -g)" "$HOME/apt-debs"
    rm -rf "$HOME/apt-debs/partial" "$HOME/apt-debs/lock"
    ;;
  playwright)
    b=${2:?browser name}
    # system packages (apt) and the browser download (Playwright's CDN) separately, to see which one is slow
    phase "playwright install-deps $b" 420 3 python3 -m playwright install-deps "$b" || exit 1
    phase "playwright install $b" 300 3 python3 -m playwright install "$b" || exit 1
    ;;
  *)
    sed -n '2,9p' "$0"
    exit 2
    ;;
esac
