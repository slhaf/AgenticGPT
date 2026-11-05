// Modified by AgenticGPT maintainers: load the bootstrap at runtime for
// cross-architecture builds instead of generating an architecture-specific
// V8 startup snapshot.
use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-check-cfg=cfg(obscura_external_bootstrap)");
    println!("cargo:rustc-check-cfg=cfg(obscura_runtime_bootstrap)");
    println!("cargo:rerun-if-changed=js/bootstrap.js");
    println!("cargo:rerun-if-changed=js/indexed_db.js");
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let snapshot_path = out_dir.join("OBSCURA_SNAPSHOT.bin");

    // V8 startup snapshots encode the CPU architecture. Same-architecture
    // GNU/musl builds can use the host snapshot; cross-architecture builds
    // execute the same bootstrap source when each V8 runtime starts.
    let target = std::env::var("TARGET").unwrap_or_default();
    let host = std::env::var("HOST").unwrap_or_default();
    let target_arch = target.split('-').next().unwrap_or_default();
    let host_arch = host.split('-').next().unwrap_or_default();
    let is_cross_arch = target_arch != host_arch;

    let bootstrap_source = include_str!("js/bootstrap.js")
        .replace("/* INDEXED_DB */", include_str!("js/indexed_db.js"));
    let bootstrap_js = bootstrap_source.as_str();

    if is_cross_arch {
        let bootstrap_path = out_dir.join("bootstrap.js");
        std::fs::write(&bootstrap_path, bootstrap_js).expect("Failed to write bootstrap.js");
        println!("cargo:rustc-cfg=obscura_runtime_bootstrap");
        println!("cargo:rustc-env=OBSCURA_BOOTSTRAP_PATH={}", bootstrap_path.display());
        std::fs::write(&snapshot_path, []).expect("Failed to write empty snapshot placeholder");
        println!("cargo:rustc-env=OBSCURA_SNAPSHOT_PATH={}", snapshot_path.display());
    } else {
        // Native build: use the host V8 directly (snapshot matches the target).
        generate_host_snapshot(bootstrap_js, &snapshot_path);
    }
}

/// Generate a snapshot using the host V8 (for native builds or as fallback).
fn generate_host_snapshot(bootstrap_js: &str, snapshot_path: &PathBuf) {
    assert!(bootstrap_js.is_ascii(), "bootstrap source must be ASCII; use Unicode escapes in literals");
    // Compiler intrinsics are confined to this build process and an unretained hook.
    deno_core::v8::V8::set_flags_from_string("--allow-natives-syntax");
    let compiler = r#""use strict";
Object.defineProperty(globalThis, '__obscura_snapshot_compile', {
  configurable: true,
  value(helpers) {
    const functions = new Set(helpers);
    const targets = [
      [Node, ['nodeType', 'textContent', 'parentNode', 'isConnected', 'getRootNode', 'appendChild', 'insertBefore']],
      [Element, ['tagName', 'innerHTML', 'getAttribute', 'setAttribute', 'querySelector', 'querySelectorAll', 'id']],
      [Document, ['documentElement', 'getElementById', 'querySelector', 'querySelectorAll', 'createElement', 'createTextNode']],
      [HTMLImageElement, ['src']],
      [Response, ['text', 'json']],
      [Headers, ['get', 'set']],
      [URL, ['href', 'toString']],
    ];
    for (const [constructor, names] of targets) {
      functions.add(constructor);
      for (const name of names) {
        const descriptor = Object.getOwnPropertyDescriptor(constructor.prototype, name);
        if (!descriptor) throw Error('Missing snapshot member: ' + constructor.name + '.' + name);
        for (const fn of [descriptor.value, descriptor.get, descriptor.set]) {
          if (typeof fn === 'function') functions.add(fn);
        }
      }
    }
    for (const fn of functions) %EnsureFeedbackVectorForFunction(fn);
  },
});
"#;
    let bootstrap_js = format!("{compiler}{bootstrap_js}");
    let output = deno_core::snapshot::create_snapshot(
        deno_core::snapshot::CreateSnapshotOptions {
            cargo_manifest_dir: env!("CARGO_MANIFEST_DIR"),
            startup_snapshot: None,
            skip_op_registration: true,
            // Extension sources are stored once outside V8's heap, not copied into every isolate.
            extensions: vec![deno_core::Extension {
                name: "obscura_bootstrap",
                js_files: vec![deno_core::ExtensionFileSource::new_computed(
                    "<obscura:bootstrap>", bootstrap_js.into(),
                )].into(),
                ..Default::default()
            }],
            extension_transpiler: None,
            with_runtime_cb: Some(Box::new(|runtime| {
                runtime.execute_script("<obscura:snapshot-check>",
                    "if ('__obscura_snapshot_compile' in globalThis) throw Error('Snapshot compiler hook was retained');")
                    .expect("snapshot compiler hook must not reach page realms");
            })),
        },
        None,
    )
    .expect("Failed to create V8 snapshot");

    std::fs::write(&snapshot_path, &*output.output).expect("Failed to write snapshot");
    println!("cargo:rustc-cfg=obscura_external_bootstrap");
    println!(
        "cargo:rustc-env=OBSCURA_SNAPSHOT_PATH={}",
        snapshot_path.display()
    );

    for file in &output.files_loaded_during_snapshot {
        println!("cargo:rerun-if-changed={}", file.display());
    }
}

