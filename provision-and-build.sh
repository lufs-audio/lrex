#!/usr/bin/env bash
# provision-and-build.sh — one-shot provision + build for lufs-recorder / lrex.
#
# Brings a fresh machine (built for macOS / klaxon; also works on Linux) to
# runnable binaries:
#   1. ensures a C toolchain / SDK  (Xcode CLT on macOS; ALSA+JACK dev on Linux)
#   2. ensures a Rust toolchain      (installs rustup if missing)
#   3. builds --release              (produces BOTH `lufs-recorder` and the
#                                      short `lrex` alias from one build — see
#                                      src/lib.rs + src/bin/*.rs)
#   4. prints the binary paths + next steps
#
# Usage:
#   bash provision-and-build.sh            # provision + build
#   bash provision-and-build.sh --run-devices   # ...then list your devices/ports
#
# Safe to re-run: every step is idempotent.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$REPO_DIR"

say()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m warn:\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m error:\033[0m %s\n' "$*" >&2; exit 1; }

OS="$(uname -s)"

# --- 1. C toolchain / SDK -----------------------------------------------------
case "$OS" in
  Darwin)
    if ! xcode-select -p >/dev/null 2>&1; then
      say "Installing Xcode Command Line Tools (a GUI prompt will appear)…"
      xcode-select --install || true
      warn "Finish the Xcode CLT install in the dialog, then re-run this script."
      # Don't hard-fail: the install runs asynchronously.
      exit 0
    fi
    say "Xcode Command Line Tools present."
    ;;
  Linux)
    if ! pkg-config --exists alsa 2>/dev/null; then
      warn "ALSA dev headers not found. cpal/midir need them on Linux."
      if command -v apt-get >/dev/null 2>&1; then
        say "Installing libasound2-dev libjack-jackd2-dev pkg-config (sudo)…"
        sudo apt-get update && sudo apt-get install -y libasound2-dev libjack-jackd2-dev pkg-config
      elif command -v dnf >/dev/null 2>&1; then
        say "Installing alsa-lib-devel pkgconf-pkg-config (sudo)…"
        sudo dnf -y install alsa-lib-devel pkgconf-pkg-config gcc
      else
        die "Install ALSA dev headers with your package manager, then re-run."
      fi
    fi
    say "ALSA dev headers present."
    ;;
  *)
    warn "Unrecognized OS '$OS' — proceeding, but only macOS + Linux are tested."
    ;;
esac

# --- 2. Rust toolchain --------------------------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
  # rustup may be installed but not on PATH in this shell.
  if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
fi
if ! command -v cargo >/dev/null 2>&1; then
  say "Installing Rust via rustup (stable)…"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
# rust-toolchain.toml pins the channel + components (rustfmt, clippy); rustup
# auto-installs them on first cargo invocation.
say "Using $(cargo --version)"

# --- 3. Build ----------------------------------------------------------------
say "Building lufs-recorder + lrex (release)…"
cargo build --release

BIN_FULL="$REPO_DIR/target/release/lufs-recorder"
BIN_SHORT="$REPO_DIR/target/release/lrex"
[ -x "$BIN_FULL" ] || die "build finished but $BIN_FULL is missing"
[ -x "$BIN_SHORT" ] || die "build finished but $BIN_SHORT (the lrex alias) is missing"

say "Built: $BIN_FULL"
say "Built: $BIN_SHORT  (short alias — same binary, same behavior)"
echo
say "Next steps (using the short alias; lufs-recorder works identically):"
cat <<EOF
  # 1. See your devices + MIDI ports (find the exact names):
  $BIN_SHORT devices

  # 2. Write a config you can edit (device, midi_port, out_dir, profiles):
  $BIN_SHORT init-config     # -> ~/.config/lufs-recorder/config.toml

  # 3. Pre-flight without recording:
  $BIN_SHORT record --dry-run

  # 4. Record a maxpatch-parity take (mic 1-2 + piano 9-10 + Nord Stage 3),
  #    Ctrl-C to stop; the take is verified before it's declared good:
  $BIN_SHORT record --name idea

  # 5. Multi-device (v0.5): capture two devices into one take.
  #    $BIN_SHORT record --device-track "DEVICE A:mic=1,2" --device-track "DEVICE B:call=1,2"

  # macOS: the first 'record' triggers a microphone permission prompt for your
  # terminal (System Settings ▸ Privacy & Security ▸ Microphone). Approve it.
EOF

# --- 4. Optional: list devices now -------------------------------------------
if [ "${1:-}" = "--run-devices" ]; then
  echo
  say "Your current devices + MIDI ports:"
  "$BIN_SHORT" devices || warn "device enumeration failed (no audio backend on this machine?)"
fi
