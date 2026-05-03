#!/usr/bin/env bun
/**
 * tools/check-cache-keys.ts — M1 fix lint (SPEC-001 §6.3, §24.3 M1).
 *
 * Walks every Rust handler under crates/pellucid-handlers/src and ensures
 * that any cached_fetch_json invocation either:
 *   (a) hardcodes the cache-key string (no request-body fields referenced
 *       by the handler appear in the key), OR
 *   (b) references every request-body / query-arg field that the handler
 *       reads via a `req.<field>` access in the surrounding function body.
 *
 * Implementation: tree-sitter-rust parses each .rs file, we walk the AST
 * to find macro/method calls named `cached_fetch_json`, extract the key
 * string literal, then collect `req.<ident>` accesses from the enclosing
 * function and compare.
 *
 * The lint operates on a directory passed as the first argument (default:
 * `crates/pellucid-handlers/src`). Empty input passes — handlers land at
 * T2.5; this script ships at T0.10 so the gate is in place from day one.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { argv, exit } from "node:process";

import Parser from "tree-sitter";
// tree-sitter-rust ships its grammar without TS types and the cast on
// import doesn't compose with bun's type stripping; cast at the call
// site instead (see buildParser below).
import RustGrammar from "tree-sitter-rust";

export interface CacheKeyFinding {
  file: string;
  line: number;
  cacheKey: string;
  missingFields: string[];
}

export interface LintReport {
  filesScanned: number;
  callsFound: number;
  findings: CacheKeyFinding[];
}

function buildParser(): Parser {
  const p = new Parser();
  // tree-sitter's setLanguage takes the language object exposed by
  // tree-sitter-rust; the package's CommonJS export is the language.
  p.setLanguage(RustGrammar as unknown as Parameters<Parser["setLanguage"]>[0]);
  return p;
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
    if (st.isDirectory()) {
      yield* walkRustFiles(full);
    } else if (st.isFile() && name.endsWith(".rs")) {
      yield full;
    }
  }
}

/** Collect every string-literal child anywhere in a subtree. */
function collectStringLiterals(node: Parser.SyntaxNode): string[] {
  const out: string[] = [];
  const stack: Parser.SyntaxNode[] = [node];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur.type === "string_literal" || cur.type === "raw_string_literal") {
      out.push(cur.text);
    }
    for (let i = 0; i < cur.namedChildCount; i++) {
      stack.push(cur.namedChild(i)!);
    }
  }
  return out;
}

/**
 * Walk every `let <ident> = <init>` declaration in a function and return
 * the map ident → initializer text. Used when a cached_fetch_json call
 * receives an identifier argument: we resolve it back to the literal it
 * was bound from in the same function.
 */
function collectLetBindings(fn: Parser.SyntaxNode): Map<string, Parser.SyntaxNode> {
  const bindings = new Map<string, Parser.SyntaxNode>();
  const stack: Parser.SyntaxNode[] = [fn];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur.type === "let_declaration") {
      const pattern = cur.childForFieldName("pattern");
      const value = cur.childForFieldName("value");
      if (pattern && value) {
        if (pattern.type === "identifier") {
          bindings.set(pattern.text, value);
        }
      }
    }
    for (let i = 0; i < cur.namedChildCount; i++) {
      stack.push(cur.namedChild(i)!);
    }
  }
  return bindings;
}

/** Receiver names treated as "the request" for field-access scraping.
 *  Every public Pellucid handler uses one of these conventions. */
const REQUEST_RECEIVERS = new Set(["req", "request", "q", "query"]);

/** Collect every `<receiver>.<ident>` field-access where the
 *  receiver identifier is one of REQUEST_RECEIVERS. */
function collectReqFieldAccesses(fn: Parser.SyntaxNode): Set<string> {
  const fields = new Set<string>();
  const stack: Parser.SyntaxNode[] = [fn];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur.type === "field_expression") {
      const value = cur.childForFieldName("value");
      const field = cur.childForFieldName("field");
      if (
        value &&
        value.type === "identifier" &&
        REQUEST_RECEIVERS.has(value.text) &&
        field
      ) {
        fields.add(field.text);
      }
    }
    for (let i = 0; i < cur.namedChildCount; i++) {
      stack.push(cur.namedChild(i)!);
    }
  }
  return fields;
}

/** Strip turbofish from a callee text. Examples:
 *   "cached_fetch_json"                              → "cached_fetch_json"
 *   "cached_fetch_json::<FlightStatus, _, _>"        → "cached_fetch_json"
 *   "pellucid_cache::cached_fetch_json::<T, _, _>"   → "pellucid_cache::cached_fetch_json"
 */
function stripTurbofish(text: string): string {
  const idx = text.indexOf("::<");
  return idx === -1 ? text : text.slice(0, idx);
}

/** Inspect the cache-key argument; return `true` iff it's a
 *  method call on a request-receiver (e.g. `req.cache_key()` or
 *  `q.cache_key()`). When so, the caller can trust the key
 *  template to incorporate every request field — the typed
 *  `cache_key` method is the canonical source of truth (lives
 *  in the generated tree and round-trips through the
 *  cache-key-template constant). */
function isRequestKeyMethodCall(node: Parser.SyntaxNode): boolean {
  if (node.type !== "call_expression") return false;
  const fn = node.childForFieldName("function");
  if (!fn || fn.type !== "field_expression") return false;
  const recv = fn.childForFieldName("value");
  if (!recv || recv.type !== "identifier") return false;
  return REQUEST_RECEIVERS.has(recv.text);
}

/** Find each call expression whose callee name is `cached_fetch_json`. */
function findCachedFetchCalls(
  root: Parser.SyntaxNode,
): Array<{ call: Parser.SyntaxNode; enclosing: Parser.SyntaxNode }> {
  const out: Array<{ call: Parser.SyntaxNode; enclosing: Parser.SyntaxNode }> = [];
  const stack: Array<{ node: Parser.SyntaxNode; fn: Parser.SyntaxNode | null }> = [
    { node: root, fn: null },
  ];
  while (stack.length > 0) {
    const { node, fn } = stack.pop()!;
    const enclosing = node.type === "function_item" ? node : fn;
    if (node.type === "call_expression") {
      const callee = node.childForFieldName("function");
      if (callee && enclosing) {
        const calleeText = stripTurbofish(callee.text);
        if (calleeText.endsWith("cached_fetch_json")) {
          out.push({ call: node, enclosing });
        }
      }
    }
    for (let i = 0; i < node.namedChildCount; i++) {
      stack.push({ node: node.namedChild(i)!, fn: enclosing });
    }
  }
  return out;
}

export function lintFile(parser: Parser, path: string, source: string): CacheKeyFinding[] {
  const tree = parser.parse(source);
  const calls = findCachedFetchCalls(tree.rootNode);
  const findings: CacheKeyFinding[] = [];

  const letBindings = new Map<Parser.SyntaxNode, Map<string, Parser.SyntaxNode>>();

  for (const { call, enclosing } of calls) {
    const args = call.childForFieldName("arguments");
    if (!args) continue;

    // The cache-key argument is the THIRD positional argument to
    // `cached_fetch_json(pool, registry, key, tier, fetcher)`.
    // Tolerate the legacy 1-arg shape (used by the bad-fixture
    // file) by also checking the first arg.
    let cacheKey: string | null = null;
    let trustedKeyMethodCall = false;
    const candidates: Parser.SyntaxNode[] = [];
    if (args.namedChildCount >= 3) candidates.push(args.namedChild(2)!);
    if (args.namedChild(0)) candidates.push(args.namedChild(0)!);
    for (const candidate of candidates) {
      // Trust path: if the key argument is `&req.cache_key()` /
      // `q.cache_key()` / similar, the typed method is the
      // canonical source of truth for the template — skip the
      // field-comparison check.
      const inner =
        candidate.type === "reference_expression"
          ? candidate.namedChild(candidate.namedChildCount - 1) ?? candidate
          : candidate;
      if (inner && isRequestKeyMethodCall(inner)) {
        trustedKeyMethodCall = true;
        break;
      }
      // Or `&key` where `key` was bound from `req.cache_key()`
      // earlier in the function.
      if (candidate.type === "reference_expression") {
        const refInner = candidate.namedChild(candidate.namedChildCount - 1);
        if (refInner && refInner.type === "identifier") {
          if (!letBindings.has(enclosing))
            letBindings.set(enclosing, collectLetBindings(enclosing));
          const init = letBindings.get(enclosing)!.get(refInner.text);
          if (init && isRequestKeyMethodCall(init)) {
            trustedKeyMethodCall = true;
            break;
          }
        }
      }
    }
    if (trustedKeyMethodCall) {
      // Fully trusted — no findings to add for this call.
      continue;
    }

    // Production signature is `cached_fetch_json(pool, registry, key, tier, fetcher)`
    // → cache key at index 2. Legacy fixture shape uses 1-arg
    // `cached_fetch_json(key, tier, fetcher)` → key at index 0.
    // Try the production position first, fall back to the legacy one.
    const keyArgCandidates: Parser.SyntaxNode[] = [];
    if (args.namedChildCount >= 3) keyArgCandidates.push(args.namedChild(2)!);
    if (args.namedChild(0)) keyArgCandidates.push(args.namedChild(0)!);
    for (const arg of keyArgCandidates) {
      if (arg.type === "string_literal" || arg.type === "raw_string_literal") {
        cacheKey = arg.text;
      } else if (arg.type === "reference_expression") {
        const inner = arg.namedChild(arg.namedChildCount - 1);
        if (inner && inner.type === "identifier") {
          if (!letBindings.has(enclosing)) letBindings.set(enclosing, collectLetBindings(enclosing));
          const init = letBindings.get(enclosing)!.get(inner.text);
          if (init) {
            const lits = collectStringLiterals(init);
            cacheKey = lits[0] ?? null;
          }
        } else if (inner) {
          const lits = collectStringLiterals(inner);
          cacheKey = lits[0] ?? null;
        }
      } else if (arg.type === "identifier") {
        if (!letBindings.has(enclosing)) letBindings.set(enclosing, collectLetBindings(enclosing));
        const init = letBindings.get(enclosing)!.get(arg.text);
        if (init) {
          const lits = collectStringLiterals(init);
          cacheKey = lits[0] ?? null;
        }
      } else {
        // Fallback: collect any string literal nested under the arg
        // (covers `format!("…", …)` and other macro calls).
        const lits = collectStringLiterals(arg);
        cacheKey = lits[0] ?? null;
      }
      if (cacheKey !== null) break;
    }
    if (cacheKey === null) continue;

    const reqFields = collectReqFieldAccesses(enclosing);
    const missing: string[] = [];
    for (const f of reqFields) {
      if (!cacheKey.includes(`{${f}}`) && !cacheKey.includes(`${f}:`)) {
        missing.push(f);
      }
    }
    if (missing.length > 0) {
      findings.push({
        file: path,
        line: call.startPosition.row + 1,
        cacheKey,
        missingFields: missing,
      });
    }
  }

  return findings;
}

export function lintDirectory(root: string): LintReport {
  const parser = buildParser();
  const findings: CacheKeyFinding[] = [];
  let filesScanned = 0;
  let callsFound = 0;
  for (const path of walkRustFiles(root)) {
    filesScanned += 1;
    const source = readFileSync(path, "utf8");
    const tree = parser.parse(source);
    callsFound += findCachedFetchCalls(tree.rootNode).length;
    const fileFindings = lintFile(parser, path, source);
    findings.push(...fileFindings);
  }
  return { filesScanned, callsFound, findings };
}

function formatFinding(finding: CacheKeyFinding, repoRoot: string): string {
  const rel = relative(repoRoot, finding.file);
  return `${rel}:${finding.line} cache key ${finding.cacheKey} omits request fields {${finding.missingFields.join(", ")}}`;
}

async function main(): Promise<void> {
  const root = argv[2] ?? "crates/pellucid-handlers/src";
  const report = lintDirectory(root);
  if (report.findings.length === 0) {
    process.stdout.write(
      `[check-cache-keys] ${report.filesScanned} file(s), ${report.callsFound} cached_fetch_json call(s), 0 findings\n`,
    );
    exit(0);
  }
  for (const f of report.findings) {
    process.stderr.write(`[check-cache-keys] ${formatFinding(f, process.cwd())}\n`);
  }
  process.stderr.write(
    `[check-cache-keys] ${report.findings.length} finding(s) across ${report.filesScanned} file(s)\n`,
  );
  exit(1);
}

if (import.meta.main) {
  await main();
}
