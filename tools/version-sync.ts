#!/usr/bin/env bun
/**
 * tools/version-sync.ts — verifies that every package's declared version
 * matches the workspace Cargo.toml's [workspace.package].version.
 *
 * Pellucid pins one version across the entire stack so a v0.2.3 webview
 * always ships against a v0.2.3 Rust edge and a v0.2.3 Tauri build.
 * Drift caused real surprises during the original WorldMonitor releases
 * (the desktop installer would advertise an older version than the
 * webview chunk it bundled).
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { exit } from "node:process";

const WORKSPACE_VERSION_RE = /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m;

export interface VersionSource {
  path: string;
  version: string | null;
}

export function readWorkspaceVersion(repoRoot: string): string {
  const cargo = readFileSync(join(repoRoot, "Cargo.toml"), "utf8");
  const m = WORKSPACE_VERSION_RE.exec(cargo);
  if (!m) throw new Error("could not find [workspace.package].version in Cargo.toml");
  return m[1]!;
}

export function readJsonVersion(repoRoot: string, rel: string): VersionSource {
  try {
    const obj = JSON.parse(readFileSync(join(repoRoot, rel), "utf8")) as {
      version?: string;
    };
    return { path: rel, version: obj.version ?? null };
  } catch {
    return { path: rel, version: null };
  }
}

export function readAllVersions(repoRoot: string): VersionSource[] {
  return [
    readJsonVersion(repoRoot, "package.json"),
    readJsonVersion(repoRoot, "webview/package.json"),
    readJsonVersion(repoRoot, "convex/package.json"),
    readJsonVersion(repoRoot, "tools/package.json"),
    readJsonVersion(repoRoot, "crates/pellucid-tauri/tauri.conf.json"),
  ];
}

export interface DriftReport {
  expected: string;
  drifts: VersionSource[];
}

export function findDrift(repoRoot: string): DriftReport {
  const expected = readWorkspaceVersion(repoRoot);
  const drifts: VersionSource[] = [];
  for (const src of readAllVersions(repoRoot)) {
    if (src.version !== null && src.version !== expected) drifts.push(src);
  }
  return { expected, drifts };
}

async function main(): Promise<void> {
  const repoRoot = process.cwd();
  const { expected, drifts } = findDrift(repoRoot);
  if (drifts.length === 0) {
    process.stdout.write(`[version-sync] ok — all versions == ${expected}\n`);
    exit(0);
  }
  for (const d of drifts) {
    process.stderr.write(
      `[version-sync] ${d.path}: expected ${expected} but found ${d.version ?? "<missing>"}\n`,
    );
  }
  exit(1);
}

if (import.meta.main) {
  await main();
}
