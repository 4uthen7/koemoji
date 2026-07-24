// @4uthent / tkmt_wonderkid
// build.rs — GPU アクセラレーション用 whisper-cli のビルド
//   Windows: nvcc があれば CUDA
//   macOS:   常に Metal
//   Linux:   nvcc があれば CUDA

use std::process::Command;

fn main() {
    let can_build = if cfg!(target_os = "macos") {
        // macOS は Metal が使える前提（Apple Silicon / Intel 両対応）
        true
    } else {
        // Windows / Linux: nvcc が必要
        Command::new("nvcc").arg("--version").output()
            .map(|o| o.status.success()).unwrap_or(false)
    };

    if can_build {
        let backend = if cfg!(target_os = "macos") { "Metal" } else { "CUDA" };
        println!("cargo:warning=GPU backend: {backend} — building whisper-cli-gpu");
        build_whisper_gpu(backend);
    } else {
        println!("cargo:warning=nvcc not found — skipping GPU build (CPU only)");
    }

    tauri_build::build();
}

fn build_whisper_gpu(backend: &str) {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let build_dir = std::path::Path::new(&out_dir).join("whisper-gpu-build");

    let whisper_dir = std::path::Path::new("whisper.cpp");
    if !whisper_dir.join("CMakeLists.txt").exists() {
        println!("cargo:warning=cloning whisper.cpp...");
        let status = Command::new("git")
            .args(["clone", "https://github.com/ggerganov/whisper.cpp.git"])
            .status().expect("git clone failed");
        if !status.success() { panic!("git clone whisper.cpp failed"); }
    }

    std::fs::create_dir_all(&build_dir).ok();

    let cmake_flag = match backend {
        "Metal" => "-DWHISPER_METAL=ON",
        _       => "-DWHISPER_CUDA=ON",
    };

    let status = Command::new("cmake")
        .args(["-B", build_dir.to_str().unwrap(),
               "-S", whisper_dir.to_str().unwrap(),
               cmake_flag,
               "-DCMAKE_BUILD_TYPE=Release"])
        .status().expect("cmake failed");
    if !status.success() {
        panic!("cmake configure failed — {backend} のセットアップを確認してください");
    }

    let status = Command::new("cmake")
        .args(["--build", build_dir.to_str().unwrap(),
               "--config", "Release",
               "--target", "whisper-cli",
               "--parallel"])
        .status().expect("cmake build failed");
    if !status.success() { panic!("cmake build failed"); }

    let exe_name = if cfg!(target_os = "windows") { "whisper-cli.exe" } else { "whisper-cli" };
    let candidates = [
        build_dir.join("bin").join("Release").join(exe_name),
        build_dir.join("bin").join(exe_name),
        build_dir.join("bin").join(format!("Release/{exe_name}")),
    ];
    let src = candidates.into_iter().find(|p| p.exists());

    if let Some(src) = src {
        let dst_name = if cfg!(target_os = "windows") { "whisper-cli-gpu.exe" }
                       else { "whisper-cli-gpu" };
        let dst = std::path::Path::new(&out_dir).join(dst_name);
        std::fs::copy(&src, &dst).expect("failed to copy whisper-cli-gpu");
        println!("cargo:rustc-env=WHISPER_CLI_GPU={}", dst.display());
        println!("cargo:warning=whisper-cli-gpu built ({backend}): {}", dst.display());
    } else {
        println!("cargo:warning=whisper-cli-gpu binary not found after build");
    }
}
