fn main() {
    // Tell cargo to rebuild whenever the frontend bundle changes.
    // `tauri_build::build()` emits `rerun-if-changed` for tauri.conf.json
    // but NOT for the `frontendDist` directory itself, so Swatinem-cached
    // CI runs that restore a target/ built before `webview/dist/` existed
    // skip the asset-embedding codegen and produce a binary that returns
    // "asset not found: index.html" at runtime. Watching index.html in
    // the dist directly invalidates the cache the moment the frontend
    // rebuild produces a new file. Path is relative to this crate
    // (matches `frontendDist = "../../webview/dist"` in tauri.conf.json).
    println!("cargo:rerun-if-changed=../../webview/dist/index.html");
    println!("cargo:rerun-if-changed=../../webview/dist");

    tauri_build::build();
}
