#!/usr/bin/env bash
set -euo pipefail

APP_NAME="wonder-of-u"
BIN_NAME="wonder-of-u"
CRATE_PATH="crates/wonder-of-u-cli"
PROFILE="release"

PREFIX="${WONDER_OF_U_INSTALL_PREFIX:-}"
FORCE=0
SKIP_BUILD=0
UNINSTALL=0

usage() {
  cat <<'USAGE'
wonder-of-u installer

Usage:
  ./install.sh [options]

Options:
  --prefix <dir>    Install under <dir>/bin
  --debug           Build the debug profile instead of release
  --skip-build      Copy an existing target/<profile>/wonder-of-u binary
  --force           Overwrite an existing installed binary
  --uninstall       Remove the installed binary from the selected prefix
  -h, --help        Show this help

Environment:
  WONDER_OF_U_INSTALL_PREFIX  Default install prefix when --prefix is omitted

Examples:
  ./install.sh
  ./install.sh --prefix "$HOME/.local"
  ./install.sh --debug --force
  ./install.sh --uninstall
USAGE
}

log() {
  printf '\033[1;34m==>\033[0m %s\n' "$*"
}

warn() {
  printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2
}

die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

repo_root() {
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd
}

choose_prefix() {
  if [[ -n "$PREFIX" ]]; then
    printf '%s\n' "$PREFIX"
    return
  fi

  if [[ -n "${HOME:-}" ]]; then
    printf '%s\n' "$HOME/.local"
    return
  fi

  if [[ -w "/usr/local/bin" ]]; then
    printf '%s\n' "/usr/local"
    return
  fi

  die "could not choose an install prefix; pass --prefix <dir>"
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      [[ $# -ge 2 ]] || die "--prefix requires a directory"
      PREFIX="$2"
      shift 2
      ;;
    --debug)
      PROFILE="debug"
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --force)
      FORCE=1
      shift
      ;;
    --uninstall)
      UNINSTALL=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown option: $1"
      ;;
  esac
done

ROOT="$(repo_root)"
PREFIX="$(choose_prefix)"
BIN_DIR="$PREFIX/bin"
INSTALL_PATH="$BIN_DIR/$BIN_NAME"
TARGET_BIN="$ROOT/target/$PROFILE/$BIN_NAME"

if [[ "$UNINSTALL" -eq 1 ]]; then
  if [[ -e "$INSTALL_PATH" ]]; then
    log "Removing $INSTALL_PATH"
    rm -f -- "$INSTALL_PATH"
  else
    warn "$INSTALL_PATH does not exist"
  fi
  exit 0
fi

need_command cargo

cd "$ROOT"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  if [[ "$PROFILE" == "release" ]]; then
    log "Building $APP_NAME ($PROFILE)"
    cargo build --release -p wonder-of-u-cli
  else
    log "Building $APP_NAME ($PROFILE)"
    cargo build -p wonder-of-u-cli
  fi
else
  log "Skipping build"
fi

[[ -x "$TARGET_BIN" ]] || die "binary not found: $TARGET_BIN"

mkdir -p -- "$BIN_DIR"

if [[ -e "$INSTALL_PATH" && "$FORCE" -ne 1 ]]; then
  die "$INSTALL_PATH already exists; pass --force to overwrite"
fi

log "Installing $INSTALL_PATH"
cp -- "$TARGET_BIN" "$INSTALL_PATH"
chmod 0755 "$INSTALL_PATH"

log "Installed $("$INSTALL_PATH" --version)"

case ":${PATH:-}:" in
  *":$BIN_DIR:"*) ;;
  *)
    warn "$BIN_DIR is not in PATH"
    warn "Add this to your shell profile: export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac

