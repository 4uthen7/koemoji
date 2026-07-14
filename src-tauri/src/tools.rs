use std::env;
use std::path::{Path, PathBuf};

/// GUI アプリではシェルの PATH が引き継がれないことがあるため、
/// PATH に加えて Homebrew などの代表的な場所も探す。
pub fn find_executable(name: &str) -> Option<PathBuf> {
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            if let Some(candidate) = find_in_dir(&dir, name) {
                return Some(candidate);
            }
        }
    }

    let mut common_dirs = Vec::new();
    #[cfg(not(target_os = "windows"))]
    common_dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = env::var_os("ProgramFiles") {
            common_dirs.push(PathBuf::from(program_files).join("Tesseract-OCR"));
        }
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            common_dirs.push(
                PathBuf::from(local_app_data)
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links"),
            );
        }
        if let Some(chocolatey) = env::var_os("ChocolateyInstall") {
            common_dirs.push(PathBuf::from(chocolatey).join("bin"));
        }
    }
    if let Some(home) = env::var_os("HOME") {
        let home = PathBuf::from(home);
        common_dirs.push(home.join(".local/bin"));
        common_dirs.push(home.join("bin"));
    }

    common_dirs
        .into_iter()
        .find_map(|dir| find_in_dir(&dir, name))
}

fn find_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidate = dir.join(name);
    if is_executable_file(&candidate) {
        return Some(candidate);
    }
    #[cfg(target_os = "windows")]
    {
        let candidate = dir.join(format!("{name}.exe"));
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}
