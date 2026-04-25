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

/** Collect every `req.<ident>` field-access where the receiver identifier is exactly "req". */
function collectReqFieldAccesses(fn: Parser.SyntaxNode): Set<string> {
  const fields = new Set<string>();
  const stack: Parser.SyntaxNode[] = [fn];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur.type === "field_expression") {
      const value = cur.childForFieldName("value");
      const field = cur.childForFieldName("field");
      if (value && value.type === "identifier" && value.text === "req" && field) {
        fields.add(field.text);
      }
    }
    for (let i = 0; i < cur.namedChildCount; i++) {
      stack.push(cur.namedChild(i)!);
    }
  }
  return fields;
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
      if (callee && callee.text.endsWith("cached_fetch_json") && enclosing) {
        out.push({ call: node, enclosing });
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

    // Resolve the first argument: either an inline string literal, an
    // identifier bound to one earlier in the function, or a `format!(...)`
    // macro that contains the literal as its template.
    let cacheKey: string | null = null;
    const firstArg = args.namedChild(0);
    if (firstArg) {
      if (firstArg.type === "string_literal" || firstArg.type === "raw_string_literal") {
        cacheKey = firstArg.text;
      } else if (firstArg.type === "reference_expression") {
        const inner = firstArg.namedChild(firstArg.namedChildCount - 1);
        if (inner && inner.type === "identifier") {
          if (!letBindings.has(enclosing)) letBindings.set(enclosing, collectLetBindings(enclosing));
          const init = letBindings.get(enclosing)!.get(inner.text);
          if (init) {
            const lits = collectStringLiterals(init);
            cacheKey = lits[0] ?? null;
          }
        }
      } else if (firstArg.type === "identifier") {
        if (!letBindings.has(enclosing)) letBindings.set(enclosing, collectLetBindings(enclosing));
        const init = letBindings.get(enclosing)!.get(firstArg.text);
        if (init) {
          const lits = collectStringLiterals(init);
          cacheKey = lits[0] ?? null;
        }
      } else {
        // Fallback: collect any string literal nested under the arg
        // (covers `format!("…", …)` and other macro calls).
        const lits = collectStringLiterals(firstArg);
        cacheKey = lits[0] ?? null;
      }
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
