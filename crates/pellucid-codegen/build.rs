// Build scripts run at compile time, not in the production
// process; the workspace's runtime-discipline lints (no panic, no
// expect, no env::var) are not applicable here.
#![allow(
    clippy::panic,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::disallowed_methods,
    clippy::print_stdout
)]
//! pellucid-codegen build script.
//!
//! Two responsibilities:
//!
//! 1. Walk the workspace's `proto/` tree, hash every `.proto` file
//!    in deterministic order, and emit the combined SHA-256 digest
//!    to `$OUT_DIR/proto_digest.txt`.
//! 2. Re-run when any `.proto` (or `buf.{yaml,gen.yaml}`) changes,
//!    so a schema edit invalidates the digest immediately.
//!
//! `pellucid-codegen::PROTO_DIGEST` reads the resulting file via
//! `include_str!`. The `pellucid-handlers/src/generated/`
//! tree carries a committed digest at
//! `crates/pellucid-handlers/src/generated/.proto.sha256` produced
//! by `bun run gen`. A unit test in `pellucid-codegen` compares
//! `PROTO_DIGEST` against the committed digest and fails CI when
//! they drift — i.e. the proto schema changed but `bun run gen`
//! was not re-run.
//!
//! When a real sebuf plugin is available on `$PATH`, `bun run gen`
//! will regenerate both `crates/pellucid-handlers/src/generated/`
//! AND `.proto.sha256` from the proto schema; until then the
//! committed `.proto.sha256` is hand-aligned with the canonical
//! generated tree.

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR set by cargo");
    // The proto/ tree lives at the workspace root, two levels up
    // from this crate's manifest dir (`crates/pellucid-codegen/`).
    let workspace_root = Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root resolves from crate manifest")
        .to_path_buf();
    let proto_root = workspace_root.join("proto");

    let mut proto_files: Vec<PathBuf> = Vec::new();
    if proto_root.exists() {
        collect_proto_files(&proto_root, &mut proto_files);
    }
    proto_files.sort();

    let mut hasher = Sha256::new();
    for path in &proto_files {
        let rel = path.strip_prefix(&proto_root).unwrap_or(path);
        // Hash both the relative path and the contents so a
        // rename or content edit both move the digest.
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        let contents = fs::read(path).unwrap_or_else(|e| {
            panic!("read proto file {}: {e}", path.display());
        });
        hasher.update(&contents);
        hasher.update(b"\0");
        // Force a rebuild whenever any proto changes.
        println!("cargo:rerun-if-changed={}", path.display());
    }

    // Also re-run if proto/buf.{yaml,gen.yaml} change since they
    // affect how a real sebuf invocation would render output.
    for cfg in ["buf.yaml", "buf.gen.yaml"] {
        let p = proto_root.join(cfg);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
            hasher.update(cfg.as_bytes());
            hasher.update(b"\0");
            let contents = fs::read(&p).unwrap_or_default();
            hasher.update(&contents);
            hasher.update(b"\0");
        }
    }

    let digest = hex_lower(&hasher.finalize());

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let out_path = Path::new(&out_dir).join("proto_digest.txt");
    let mut f = fs::File::create(&out_path)
        .unwrap_or_else(|e| panic!("create {}: {e}", out_path.display()));
    f.write_all(digest.as_bytes())
        .unwrap_or_else(|e| panic!("write digest: {e}"));

    // Touch every file we walked so cargo invalidates this script
    // when the directory layout itself changes (file added/removed).
    println!("cargo:rerun-if-changed={}", proto_root.display());
}

fn collect_proto_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_proto_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "proto") {
            out.push(path);
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
