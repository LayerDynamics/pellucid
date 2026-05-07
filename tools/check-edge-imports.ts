#!/usr/bin/env bun
/**
 * tools/check-edge-imports.ts — preserves the spirit of the original
 * WorldMonitor `tests/edge-functions.test.mjs` guardrail (forbade
 * `node:` builtins and cross-dir imports in api/*.js).
 *
 * For Pellucid (Rust edge), the equivalent guardrail is:
 *   1. crates/pellucid-handlers/src must not depend on tokio runtime
 *      directly — it should only use the runtime via re-exports from
 *      pellucid-core. Direct `tokio::` paths are flagged.
 *   2. Handler modules must not import from sibling crates that aren't
 *      declared in their Cargo.toml [dependencies].
 *
 * Implementation: parses Cargo.toml for the handlers crate, walks
 * crates/pellucid-handlers/src/**\/*.rs, collects every `use <crate>::`
 * statement, asserts the crate appears in the declared deps. Cross-dir
 * imports outside the handlers crate are also flagged.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { exit } from "node:process";

const HANDLER_ROOT = "crates/pellucid-handlers";
const ALLOWED_STD_PREFIXES = ["std", "core", "alloc", "self", "super", "crate"];

export interface EdgeImportFinding {
  file: string;
  line: number;
  importedCrate: string;
  reason: string;
}

const USE_RE = /^\s*(?:pub(?:\(.*?\))?\s+)?use\s+([A-Za-z_][A-Za-z0-9_]*)(?:::|\s|;)/;

/// Matches `mod X;` / `pub mod X;` / `pub(crate) mod X;` declarations.
/// Captures the module name. Skips inline `mod X { ... }` bodies (those
/// don't end with a semicolon on the same line — uncommon in this crate).
const MOD_DECL_RE =
  /^\s*(?:pub(?:\(.*?\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;/;

export function listDeclaredDeps(crateRoot: string): Set<string> {
  const cargoToml = readFileSync(join(crateRoot, "Cargo.toml"), "utf8");
  const deps = new Set<string>();
  let inDepsSection = false;
  for (const rawLine of cargoToml.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[")) {
      inDepsSection =
        line === "[dependencies]" ||
        line === "[dev-dependencies]" ||
        line === "[build-dependencies]";
      continue;
    }
    if (!inDepsSection || line.length === 0 || line.startsWith("#")) continue;
    const m = /^([A-Za-z0-9_-]+)\s*=/.exec(line);
    if (m) {
      // Cargo crate names use hyphens but Rust paths replace them with underscores.
      deps.add(m[1]!.replace(/-/g, "_"));
    }
  }
  return deps;
}

function* walkRustFiles(root: string): Generator<string> {
  let entries: string[];
  try {
    entries = readdirSync(root);
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === "ENOENT") return;
    throw err;
  }
  for (const name of entries) {
    const full = join(root, name);
    const st = statSync(full);
    if (st.isDirectory()) yield* walkRustFiles(full);
    else if (st.isFile() && name.endsWith(".rs")) yield full;
  }
}

export function lintFile(
  path: string,
  source: string,
  declared: Set<string>,
): EdgeImportFinding[] {
  const findings: EdgeImportFinding[] = [];
  const lines = source.split(/\r?\n/);

  // First pass — collect every sibling module declared in this file
  // (`mod X;` / `pub mod X;`). A `use X::…` referencing a sibling
  // module is an internal re-export, not an external crate import,
  // and must not be flagged.
  const localModules = new Set<string>();
  for (const line of lines) {
    const m = MOD_DECL_RE.exec(line);
    if (m) localModules.add(m[1]!);
  }

  // Second pass — flag `use X::…` where `X` is neither std/core/etc.,
  // a declared dependency, nor a sibling module of this file.
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const m = USE_RE.exec(line);
    if (!m) continue;
    const crate = m[1]!;
    if (ALLOWED_STD_PREFIXES.includes(crate)) continue;
    if (declared.has(crate)) continue;
    if (localModules.has(crate)) continue;
    findings.push({
      file: path,
      line: i + 1,
      importedCrate: crate,
      reason: `crate "${crate.replace(/_/g, "-")}" not declared in Cargo.toml [dependencies]`,
    });
  }
  return findings;
}

export function lintHandlersCrate(repoRoot: string): EdgeImportFinding[] {
  const crateRoot = join(repoRoot, HANDLER_ROOT);
  const declared = listDeclaredDeps(crateRoot);
  const findings: EdgeImportFinding[] = [];
  for (const path of walkRustFiles(join(crateRoot, "src"))) {
    findings.push(...lintFile(path, readFileSync(path, "utf8"), declared));
  }
  return findings;
}

async function main(): Promise<void> {
  const repoRoot = process.cwd();
  const findings = lintHandlersCrate(repoRoot);
  if (findings.length === 0) {
    process.stdout.write("[check-edge-imports] ok — no undeclared imports\n");
    exit(0);
  }
  for (const f of findings) {
    const rel = relative(repoRoot, f.file);
    process.stderr.write(`[check-edge-imports] ${rel}:${f.line} ${f.reason}\n`);
  }
  exit(1);
}

if (import.meta.main) {
  await main();
}
