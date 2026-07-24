// @4uthent / tkmt_wonderkid
// build.rs — nvcc があれば whisper.cpp を CUDA 対応でビルドし whisper-cli-cuda.exe を生成

use std::process::Command;

fn main() {
    let has_cuda = Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if has_cuda {
        println!("cargo:warning=nvcc found — building whisper-cli-cuda.exe");
        build_whisper_cli_cuda();
    } else {
        println!("cargo:warning=nvcc not found — skipping CUDA build (CPU only)");
    }

    tauri_build::build();
}

fn build_whisper_cli_cuda() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let build_dir = std::path::Path::new(&out_dir).join("whisper-cuda-build");

    // whisper.cpp を clone（なければ）
    let whisper_dir = std::path::Path::new("whisper.cpp");
    if !whisper_dir.join("CMakeLists.txt").exists() {
        println!("cargo:warning=cloning whisper.cpp...");
        let status = Command::new("git")
            .args(["clone", "https://github.com/ggerganov/whisper.cpp.git"])
            .status()
            .expect("git clone failed");
        if !status.success() {
            panic!("git clone whisper.cpp failed");
        }
    }

    std::fs::create_dir_all(&build_dir).ok();

    // cmake configure（CUDA 有効）
    let status = Command::new("cmake")
        .args([
            "-B", build_dir.to_str().unwrap(),
            "-S", whisper_dir.to_str().unwrap(),
            "-DWHISPER_CUDA=ON",
            "-DCMAKE_BUILD_TYPE=Release",
        ])
        .status()
        .expect("cmake failed");
    if !status.success() {
        panic!("cmake configure failed — CUDA Toolkit が正しくインストールされているか確認してください");
    }

    // cmake build（main ターゲット = whisper-cli）
    let status = Command::new("cmake")
        .args(["--build", build_dir.to_str().unwrap(), "--config", "Release", "--target", "main", "--parallel"])
        .status()
        .expect("cmake build failed");
    if !status.success() {
        panic!("cmake build failed");
    }

    // whisper-cli.exe をターゲットディレクトリにコピー
    let exe_name = if cfg!(target_os = "windows") { "main.exe" }
        else if cfg!(target_os = "macos") { "main" }
        else { "main" };

    let candidates = [
        build_dir.join("bin").join("Release").join(exe_name),
        build_dir.join("bin").join(exe_name),
        build_dir.join("bin").join(format!("Release/{exe_name}")),
    ];
    let src = candidates.into_iter().find(|p| p.exists());

    if let Some(src) = src {
        let dst_name = if cfg!(target_os = "windows") { "whisper-cli-cuda.exe" }
            else { "whisper-cli-cuda" };
        let dst = std::path::Path::new(&out_dir).join(dst_name);
        std::fs::copy(&src, &dst).expect("failed to copy whisper-cli-cuda");

        // 環境変数でパスを通知
        println!("cargo:rustc-env=WHISPER_CLI_CUDA={}", dst.display());
        println!("cargo:warning=whisper-cli-cuda built: {}", dst.display());
    } else {
        println!("cargo:warning=whisper-cli-cuda binary not found after build");
    }
}
