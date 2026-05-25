// Build script for the TTD Replay Engine shim.
//
// Inputs (in priority order):
//   1. `TTD_SDK_DIR` env var pointing at a layout matching the NuGet package:
//        $TTD_SDK_DIR/include/TTD/*.h
//        $TTD_SDK_DIR/lib/x64/TTDReplay.lib
//   2. The default layout produced by `.github/scripts/download-ttd.ps1`:
//        <workspace>/extension/resources/ttd-sdk/...
//
// Behavior:
//   - Compiles `cpp/shim.cpp` (C++20, MSVC) into a static library named `dhttd_shim`.
//   - Links `TTDReplay.lib` (the import library for `TTDReplay.dll`).
//   - Emits `cargo:runtime_dir=<workspace>/extension/resources/ttd/<arch>` so
//     downstream crates / test harnesses can locate the runtime DLLs
//     (`TTDReplay.dll`, `TTDReplayCPU.dll`).
//
// Note: this script does NOT copy the runtime DLLs into `target/<profile>/deps`.
// Callers are responsible for ensuring `TTDReplay.dll` and `TTDReplayCPU.dll`
// are resolvable by the Windows loader at runtime — either by adding the
// `runtime_dir` above to `PATH`, copying the DLLs next to the test/binary, or
// using `SetDllDirectory`/`AddDllDirectory` from the host process.

use std::env;
use std::path::{Path, PathBuf};

fn main() {
    if !cfg!(target_os = "windows") {
        // TTD is Windows-only; on other platforms produce an empty rlib so
        // `cargo check` on the workspace still works (downstream code that
        // actually calls into the FFI will fail to link on non-Windows, which
        // is the desired behavior).
        println!(
            "cargo:warning=morgagni-ttd-decoder-sys: TTD is Windows-only, building empty shim"
        );
        return;
    }

    let sdk_dir = locate_sdk_dir();
    let include_dir = sdk_dir.join("include");
    let arch = target_arch_subdir();
    let lib_dir = sdk_dir.join("lib").join(arch);

    if !include_dir.join("TTD/IReplayEngine.h").exists() {
        panic!(
            "TTD SDK headers not found at {}. Run .github/scripts/download-ttd.ps1 or set TTD_SDK_DIR.",
            include_dir.display()
        );
    }
    if !lib_dir.join("TTDReplay.lib").exists() {
        panic!(
            "TTDReplay.lib not found at {}. Run .github/scripts/download-ttd.ps1 or set TTD_SDK_DIR.",
            lib_dir.display()
        );
    }

    println!("cargo:rerun-if-changed=cpp/shim.cpp");
    println!("cargo:rerun-if-changed=cpp/shim.h");
    println!("cargo:rerun-if-env-changed=TTD_SDK_DIR");

    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .file("cpp/shim.cpp")
        .include(&include_dir)
        .flag_if_supported("/EHsc")
        .flag_if_supported("/W4")
        .flag_if_supported("/permissive-")
        .compile("dhttd_shim");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=TTDReplay");

    // Expose SDK_DIR to downstream crates that want to find the runtime DLLs.
    println!("cargo:sdk_dir={}", sdk_dir.display());
    println!("cargo:runtime_dir={}", runtime_dir_for_arch(arch).display());
}

fn locate_sdk_dir() -> PathBuf {
    if let Ok(d) = env::var("TTD_SDK_DIR") {
        return PathBuf::from(d);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/morgagni-ttd-decoder-sys -> workspace root
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    workspace
        .join("extension")
        .join("resources")
        .join("ttd-sdk")
}

fn target_arch_subdir() -> &'static str {
    match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "arm64",
        other => panic!("unsupported target arch for TTD SDK: {other:?}"),
    }
}

fn runtime_dir_for_arch(arch: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf();
    workspace
        .join("extension")
        .join("resources")
        .join("ttd")
        .join(arch)
}
