#!/usr/bin/env sh
# Keep the Rust build directories bounded: sweep artifacts older than a
# week, drop the coverage build, remove a directory whole when it is still
# above the cap. See docs/DEVELOPMENT.md "Build cache".
#
#   scripts/clean-build.sh            # sweep, cap 15 GB
#   CAP_GB=5 scripts/clean-build.sh   # tighter cap
#   scripts/clean-build.sh --all      # remove every build directory now
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
cap=${CAP_GB:-15}
days=${DAYS:-7}
all=0
[ "${1:-}" = "--all" ] && all=1
dirs="$repo/target $(dirname "$repo")/target-claude ${CARGO_TARGET_DIR:-}"
for d in $dirs; do
  [ -d "$d" ] || continue
  before=$(du -sm "$d" 2>/dev/null | cut -f1)
  if [ "$all" = 1 ]; then
    rm -rf "$d"; echo "$d: $((before / 1024)) GB -> removed"; continue
  fi
  rm -rf "$d/llvm-cov-target"
  if command -v cargo-sweep >/dev/null 2>&1; then
    cargo sweep --time "$days" --path "$repo" --target "$d" >/dev/null 2>&1
  else
    echo "cargo-sweep not installed (cargo install cargo-sweep): only the cap applies"
  fi
  after=$(du -sm "$d" 2>/dev/null | cut -f1)
  if [ "$((after / 1024))" -gt "$cap" ]; then
    rm -rf "$d"; echo "$d: $((before / 1024)) GB -> $((after / 1024)) GB after sweep, above $cap GB: removed"
  else
    echo "$d: $((before / 1024)) GB -> $((after / 1024)) GB"
  fi
done
rm -rf "$repo/web/coverage"
