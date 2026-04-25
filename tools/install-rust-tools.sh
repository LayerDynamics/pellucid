#!/usr/bin/env bash
# tools/install-rust-tools.sh — install or refresh the Rust dev tools
# the universal CI gate (`just check`) depends on.
#
# Usage: tools/install-rust-tools.sh [--check]
#   --check  Print version of every required tool; exit non-zero if any missing.

set -euo pipefail

# Tool name + invocation used to print version.
# Some tools (cargo-llvm-cov) are invoked as `cargo <subcmd>` rather than as
# a top-level binary, so the version probe is encoded as a shell snippet.
declare -a REQUIRED_NAMES=(
    "cargo-nextest"
    "cargo-llvm-cov"
    "cargo-audit"
    "cargo-deny"
)

probe_version() {
    case "$1" in
        cargo-nextest) cargo nextest --version 2>/dev/null | head -1 ;;
        cargo-llvm-cov) cargo llvm-cov --version 2>/dev/null | head -1 ;;
        cargo-audit) cargo audit --version 2>/dev/null | head -1 ;;
        cargo-deny) cargo deny --version 2>/dev/null | head -1 ;;
        *) return 1 ;;
    esac
}

is_present() {
    case "$1" in
        cargo-nextest) cargo nextest --version >/dev/null 2>&1 ;;
        cargo-llvm-cov) cargo llvm-cov --version >/dev/null 2>&1 ;;
        cargo-audit) cargo audit --version >/dev/null 2>&1 ;;
        cargo-deny) cargo deny --version >/dev/null 2>&1 ;;
        *) return 1 ;;
    esac
}

mode="install"
if [[ "${1:-}" == "--check" ]]; then
    mode="check"
fi

missing=()
for tool in "${REQUIRED_NAMES[@]}"; do
    if is_present "${tool}"; then
        printf "%-20s %s\n" "${tool}" "$(probe_version "${tool}")"
    else
        missing+=("${tool}")
    fi
done

if [[ "${#missing[@]}" -eq 0 ]]; then
    exit 0
fi

if [[ "${mode}" == "check" ]]; then
    printf 'Missing tools: %s\n' "${missing[*]}" >&2
    printf 'Run tools/install-rust-tools.sh to install them.\n' >&2
    exit 1
fi

for tool in "${missing[@]}"; do
    printf '\nInstalling %s...\n' "${tool}"
    case "${tool}" in
        cargo-nextest) cargo install --locked cargo-nextest ;;
        cargo-llvm-cov)
            cargo install --locked cargo-llvm-cov
            rustup component add llvm-tools-preview
            ;;
        cargo-audit) cargo install --locked cargo-audit ;;
        cargo-deny) cargo install --locked cargo-deny ;;
        *)
            printf 'Unknown tool: %s\n' "${tool}" >&2
            exit 2
            ;;
    esac
done

printf '\nAll required Rust tools present.\n'
