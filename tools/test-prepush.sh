#!/usr/bin/env bash
# tools/test-prepush.sh — verifies the pre-push hook actually fails on a
# broken commit. Drives the hook against a temporary worktree containing
# a deliberate cargo fmt violation; asserts the hook exits non-zero and
# names the failing step. Asserts skip env var (PELLUCID_SKIP_PREPUSH=1)
# is honored.

set -euo pipefail

REPO="$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"
HOOK="${REPO}/.husky/pre-push"

if [[ ! -x "${HOOK}" ]]; then
    printf '[test-prepush] hook %s missing or not executable\n' "${HOOK}" >&2
    exit 2
fi

WORK="$(mktemp -d -t pellucid-prepush)"
trap 'rm -rf "${WORK}"' EXIT

git -C "${REPO}" worktree add --detach "${WORK}/copy" main >/dev/null

# Inject a deliberate fmt violation.
TARGET="${WORK}/copy/crates/pellucid-core/src/lib.rs"
{
    printf '\n//bad fmt: missing space and unaligned\n'
    printf 'pub fn _bad_fmt(  )  ->  () {  }\n'
} >> "${TARGET}"

set +e
( cd "${WORK}/copy" && bash "${HOOK}" ) >"${WORK}/out" 2>&1
RC=$?
set -e

if [[ ${RC} -eq 0 ]]; then
    printf '[test-prepush] expected non-zero exit on bad fmt, got 0\n' >&2
    cat "${WORK}/out" >&2
    git -C "${REPO}" worktree remove --force "${WORK}/copy" >/dev/null 2>&1 || true
    exit 1
fi

if ! grep -q "FAILED: cargo fmt --check" "${WORK}/out"; then
    printf '[test-prepush] hook failed but did not name cargo fmt step\n' >&2
    cat "${WORK}/out" >&2
    git -C "${REPO}" worktree remove --force "${WORK}/copy" >/dev/null 2>&1 || true
    exit 1
fi

printf '[test-prepush] OK — hook exited %d on bad fmt and named the failing step\n' "${RC}"

# Skip env var path: should exit 0 even with the bad fmt still in place.
set +e
( cd "${WORK}/copy" && PELLUCID_SKIP_PREPUSH=1 bash "${HOOK}" ) >"${WORK}/skip-out" 2>&1
SKIP_RC=$?
set -e

if [[ ${SKIP_RC} -ne 0 ]]; then
    printf '[test-prepush] PELLUCID_SKIP_PREPUSH=1 should exit 0 but got %d\n' "${SKIP_RC}" >&2
    cat "${WORK}/skip-out" >&2
    git -C "${REPO}" worktree remove --force "${WORK}/copy" >/dev/null 2>&1 || true
    exit 1
fi

if ! grep -q "skipping universal gate" "${WORK}/skip-out"; then
    printf '[test-prepush] skip env var did not log the expected message\n' >&2
    cat "${WORK}/skip-out" >&2
    git -C "${REPO}" worktree remove --force "${WORK}/copy" >/dev/null 2>&1 || true
    exit 1
fi

printf '[test-prepush] OK — PELLUCID_SKIP_PREPUSH=1 honored\n'

git -C "${REPO}" worktree remove --force "${WORK}/copy" >/dev/null 2>&1 || true
printf '[test-prepush] all assertions passed\n'
