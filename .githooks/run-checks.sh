#!/usr/bin/env bash
set -uo pipefail

hook="${1:-hook}"
skip_raw="${SKIP:-}"
skip_lower="$(printf '%s' "$skip_raw" | tr '[:upper:]' '[:lower:]' | tr -d '[:space:]')"

is_skipped() {
  local name="$1"
  case ",${skip_lower}," in
    *,all,*) return 0 ;;
    *,"${name}",*) return 0 ;;
  esac
  return 1
}

run() {
  local name="$1"; shift
  if is_skipped "$name"; then
    printf '  [SKIP] %s\n' "$name"
    return 0
  fi
  printf '  [RUN]  %s\n' "$name"
  if ! "$@"; then
    printf '  [FAIL] %s\n' "$name"
    printf '\n>> %s: "%s" failed. Fix it, or bypass with SKIP=%s\n' "$hook" "$name" "$name"
    exit 1
  fi
}

printf '>> %s: running AGENTS.md validation suite\n' "$hook"

run fmt      cargo fmt --all -- --check
run clippy   cargo clippy --all-targets --workspace -- -D warnings
run test     cargo test --workspace
run doc      cargo doc --workspace --no-deps
run release  cargo build --release

printf '>> %s: all checks passed.\n' "$hook"
exit 0
