#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLCHAIN_FILE="${ROOT_DIR}/rust-toolchain.toml"
CHANNEL=""

if [ -f "${TOOLCHAIN_FILE}" ]; then
  CHANNEL="$(sed -nE 's/^channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "${TOOLCHAIN_FILE}" | head -n1)"
fi

if [ -z "${CHANNEL}" ]; then
  CHANNEL="stable"
fi

CARGO_BIN=""
RUSTC_BIN=""

DIRECT_CARGO="${HOME}/.rustup/toolchains/${CHANNEL}/bin/cargo"
DIRECT_RUSTC="${HOME}/.rustup/toolchains/${CHANNEL}/bin/rustc"
if [ -x "${DIRECT_CARGO}" ] && [ -x "${DIRECT_RUSTC}" ]; then
  CARGO_BIN="${DIRECT_CARGO}"
  RUSTC_BIN="${DIRECT_RUSTC}"
fi

if [ -z "${CARGO_BIN}" ]; then
  shopt -s nullglob
  CANDIDATES=( "${HOME}/.rustup/toolchains/${CHANNEL}-"*/bin/cargo )
  shopt -u nullglob
  for candidate in "${CANDIDATES[@]}"; do
    candidate_rustc="${candidate%/cargo}/rustc"
    if [ -x "${candidate}" ] && [ -x "${candidate_rustc}" ]; then
      CARGO_BIN="${candidate}"
      RUSTC_BIN="${candidate_rustc}"
      break
    fi
  done
fi

if [ -z "${CARGO_BIN}" ]; then
  echo "error: Rust toolchain '${CHANNEL}' not found under ${HOME}/.rustup/toolchains" >&2
  echo "install it with rustup, then retry (for example: rustup toolchain install ${CHANNEL})" >&2
  exit 1
fi

export RUSTC="${RUSTC_BIN}"
exec "${CARGO_BIN}" "$@"
