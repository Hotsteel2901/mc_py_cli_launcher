//! Java detection and Mojang runtime download.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rayon::prelude::*;

use crate::http;
use crate::util;

const MOJANG_JAVA_MANIFEST: &str =
    "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Get the Java major version number.
pub fn java_major(java_path: &str) -> Option<u32> {
    let output = Command::new(java_path)
        .arg("-version")
        .output()
        .ok()?;

    let text = String::from_utf8_lossy(&output.stderr);
    // Match "version "1.8.0_..." or "version "17.0.1"..."
    let re = regex::Regex::new(r#"version "(?:1\.)?(\d+)"#).ok()?;
    let cap = re.captures(&text)?;
    cap.get(1)?.as_str().parse().ok()
}

/// Find all Java candidates on the system.
pub fn find_java_candidates() -> Vec<String> {
    let java_bin = if cfg!(windows) { "java.exe" } else { "java" };
    let mut candidates: Vec<String> = Vec::new();

    // JAVA_HOME
    if let Ok(jh) = std::env::var("JAVA_HOME") {
        let je = Path::new(&jh).join("bin").join(java_bin);
        if je.exists() {
            candidates.push(je.to_string_lossy().to_string());
        }
    }

    // PATH
    if let Ok(path) = which::which("java") {
        candidates.push(path.to_string_lossy().to_string());
    }

    // Platform-specific scanning
    let mut scanned = scan_java_installations();
    candidates.append(&mut scanned);

    // Deduplicate by resolved path
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    for c in candidates {
        let key = std::fs::canonicalize(&c)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| c.clone());
        if seen.insert(key) {
            result.push(c);
        }
    }
    result
}

#[cfg(windows)]
fn scan_java_installations() -> Vec<String> {
    let mut scanned = Vec::new();
    let java_exe = "java.exe";

    // Windows registry
    let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
    for key_path in &[
        r"SOFTWARE\Eclipse Adoptium\JDK",
        r"SOFTWARE\Eclipse Foundation\JDK",
        r"SOFTWARE\JavaSoft\JDK",
    ] {
        if let Ok(jdk_key) = hklm.open_subkey_with_flags(key_path, winreg::enums::KEY_READ) {
            for name in jdk_key.enum_keys().flatten() {
                if let Ok(sub_key) = jdk_key.open_subkey_with_flags(&name, winreg::enums::KEY_READ) {
                    if let Ok(jh) = sub_key.get_value::<String, _>("Path") {
                        let je = Path::new(&jh).join("bin").join(java_exe);
                        if je.exists() {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    for key_path in &[
        r"SOFTWARE\Eclipse Adoptium\JDK",
        r"SOFTWARE\JavaSoft\JDK",
    ] {
        if let Ok(jdk_key) = hkcu.open_subkey_with_flags(key_path, winreg::enums::KEY_READ) {
            for name in jdk_key.enum_keys().flatten() {
                if let Ok(sub_key) = jdk_key.open_subkey_with_flags(&name, winreg::enums::KEY_READ) {
                    if let Ok(jh) = sub_key.get_value::<String, _>("Path") {
                        let je = Path::new(&jh).join("bin").join(java_exe);
                        if je.exists() {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    // User home JDK dirs
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let home = Path::new(&userprofile);
        if let Ok(entries) = fs::read_dir(home) {
            let mut jdk_dirs: Vec<_> = entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("jdk-")
                })
                .collect();
            jdk_dirs.sort_by_key(|e| e.file_name());
            jdk_dirs.reverse();
            for d in jdk_dirs {
                let je = d.path().join("bin").join(java_exe);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // Common install dirs
    for base in &[
        r"C:\Program Files\Java",
        r"C:\Program Files\Eclipse Adoptium",
        r"C:\Program Files\Microsoft",
        r"C:\Program Files\Eclipse Foundation",
        r"C:\Program Files (x86)\Java",
        r"C:\Program Files\Zulu",
        r"C:\Program Files\Amazon Corretto",
        r"C:\Program Files\ojdkbuild",
        r"C:\tools\java",
    ] {
        let bp = Path::new(base);
        if bp.exists() {
            if let Ok(entries) = fs::read_dir(bp) {
                for d in entries.flatten() {
                    if d.path().is_dir() {
                        // Direct java in jdk/bin/
                        let je = d.path().join("bin").join(java_exe);
                        if je.exists() && !scanned.contains(&je.to_string_lossy().to_string()) {
                            scanned.push(je.to_string_lossy().to_string());
                        }
                        // Subdir check (nested JDKs)
                        if let Ok(sub) = fs::read_dir(d.path()) {
                            for sd in sub.flatten() {
                                if sd.path().is_dir() {
                                    let je2 = sd.path().join("bin").join(java_exe);
                                    if je2.exists() && !scanned.contains(&je2.to_string_lossy().to_string()) {
                                        scanned.push(je2.to_string_lossy().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    scanned
}

#[cfg(not(windows))]
fn scan_java_installations() -> Vec<String> {
    let mut scanned = Vec::new();
    let java_bin = "java";
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    for base in &[
        "/usr/lib/jvm",
        "/usr/local/opt",
        "/opt/java",
    ] {
        let bp = Path::new(base);
        if bp.exists() {
            if let Ok(entries) = fs::read_dir(bp) {
                let mut dirs: Vec<_> = entries.flatten().collect();
                dirs.sort_by_key(|e| e.file_name());
                dirs.reverse();
                for d in dirs {
                    let je = d.path().join("bin").join(java_bin);
                    if je.exists() {
                        scanned.push(je.to_string_lossy().to_string());
                    }
                }
            }
        }
    }

    // SDKMAN
    let sdkman = home.join(".sdkman/candidates/java");
    if sdkman.exists() {
        if let Ok(entries) = fs::read_dir(&sdkman) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // .jdks
    let jdks = home.join(".jdks");
    if jdks.exists() {
        if let Ok(entries) = fs::read_dir(&jdks) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // ASDF
    let asdf = home.join(".asdf/installs/java");
    if asdf.exists() {
        if let Ok(entries) = fs::read_dir(&asdf) {
            let mut dirs: Vec<_> = entries.flatten().collect();
            dirs.sort_by_key(|e| e.file_name());
            dirs.reverse();
            for d in dirs {
                let je = d.path().join("bin").join(java_bin);
                if je.exists() {
                    scanned.push(je.to_string_lossy().to_string());
                }
            }
        }
    }

    // Homebrew OpenJDK symlink
    let hb = Path::new("/usr/local/opt/openjdk/bin/java");
    if hb.exists() {
        scanned.push(hb.to_string_lossy().to_string());
    }

    scanned
}

/// Check for a suitable Java installation. Optionally require a specific major version.
/// Returns the path to the java executable.
pub fn check_java(required_major: Option<u32>) -> Option<String> {
    let candidates = find_java_candidates();

    if let Some(req) = required_major {
        for c in &candidates {
            if java_major(c) == Some(req) {
                return Some(c.clone());
            }
        }
        return None;
    }

    if candidates.is_empty() {
        crate::die!(
            "Java not found. Install Java 17+ from https://adoptium.net/",
            "If you already installed Java, set JAVA_HOME."
        );
    }

    let java = &candidates[0];
    let ver = java_major(java);
    if let Some(v) = ver {
        if v < 17 {
            crate::warn_msg!(
                "Java {} detected. Minecraft 1.18+ needs Java 17+.",
                v
            );
            crate::info!("Found at: {}", java);
        }
    }
    Some(java.clone())
}

/// Get the platform key for Mojang's Java runtime manifest.
fn mojang_java_platform() -> Option<&'static str> {
    match (util::os_name(), util::os_arch()) {
        ("windows", "arm64") => Some("windows-arm64"),
        ("windows", _) => Some("windows-x64"),
        ("osx", "arm64") => Some("mac-os-arm64"),
        ("osx", _) => Some("mac-os"),
        ("linux", "x86_64") => Some("linux"),
        _ => None,
    }
}

/// Download Mojang's bundled Java runtime. Returns path to java executable.
pub fn download_mojang_java(game_dir: &Path, component: &str, max_workers: usize) -> Option<String> {
    let dest_root = game_dir.join("java").join(component);
    let exe_name = if cfg!(windows) { "java.exe" } else { "java" };

    // Check if already downloaded
    let existing = find_java_in_dir(&dest_root, exe_name);
    if let Some(path) = existing {
        return Some(path);
    }

    let plat = mojang_java_platform()?;

    // Fetch platform manifest
    let (status, body) = http::http_get(MOJANG_JAVA_MANIFEST).ok()?;
    if status != 200 {
        crate::warn_msg!("Mojang Java runtime index fetch failed ({})", status);
        return None;
    }

    let entries: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let platform_entries = entries[plat][component].as_array()?;
    let first = platform_entries.first()?;
    let ver_name = first["version"]["name"].as_str().unwrap_or("?");

    // Fetch component manifest
    let manifest_url = first["manifest"]["url"].as_str()?;
    let (status2, body2) = http::http_get(manifest_url).ok()?;
    if status2 != 200 {
        crate::warn_msg!("Mojang Java runtime manifest fetch failed ({})", status2);
        return None;
    }

    let files_data: serde_json::Value = serde_json::from_slice(&body2).ok()?;
    let files = files_data["files"].as_object()?;

    // Collect downloads
    let mut downloads: Vec<(String, PathBuf, Option<String>)> = Vec::new();
    let mut executables: Vec<PathBuf> = Vec::new();
    let mut links: Vec<(PathBuf, String)> = Vec::new();

    for (rel, info) in files {
        let target = dest_root.join(rel);
        match info["type"].as_str() {
            Some("directory") => {
                fs::create_dir_all(&target).ok();
            }
            Some("file") => {
                let raw = &info["downloads"]["raw"];
                let url = raw["url"].as_str();
                let sha1 = raw["sha1"].as_str();
                if let Some(url) = url {
                    let need_dl = match (target.exists(), sha1) {
                        (true, Some(expected)) => {
                            crate::util::sha1_file(&target) != expected
                        }
                        (true, None) => false,
                        (false, _) => true,
                    };
                    if need_dl {
                        downloads.push((
                            url.to_string(),
                            target.clone(),
                            sha1.map(String::from),
                        ));
                    }
                }
                if info["executable"].as_bool().unwrap_or(false) {
                    executables.push(target);
                }
            }
            Some("link") => {
                if let Some(link_target) = info["target"].as_str() {
                    links.push((target, link_target.to_string()));
                }
            }
            _ => {}
        }
    }

    if downloads.is_empty() {
        let found = find_java_in_dir(&dest_root, exe_name);
        return found;
    }

    crate::info!(
        "Downloading Java runtime {} ({}): {} files [{} threads]...",
        ver_name,
        component,
        downloads.len(),
        max_workers
    );

    let total = downloads.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let fail = std::sync::atomic::AtomicUsize::new(0);

    downloads.par_iter().for_each(|(url, dest, sha1)| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            http::download_file(
                url,
                dest,
                "",
                sha1.as_deref(),
                2,
                false,
            );
            true
        }));
        match result {
            Ok(true) => {
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let f = fail.load(std::sync::atomic::Ordering::Relaxed);
                let pct = (n + f) * 100 / total;
                let filled = pct * 25 / 100;
                let bar: String = (0..25)
                    .map(|i| if i < filled { '\u{2588}' } else { '\u{2591}' })
                    .collect();
                print!(
                    "\r  [{:3}%] {} {}/{} files",
                    pct, bar, n + f, total
                );
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
            _ => {
                fail.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });
    println!();

    let fails = fail.load(std::sync::atomic::Ordering::Relaxed);
    if fails > 0 {
        crate::warn_msg!("{} Java runtime file(s) failed to download.", fails);
        return None;
    }

    // Set executable permissions on non-Windows
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        for t in &executables {
            if let Ok(meta) = fs::metadata(t) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o755);
                fs::set_permissions(t, perms).ok();
            }
        }
        for (target, link_target) in &links {
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).ok();
                }
                std::os::unix::fs::symlink(link_target, target).ok();
            }
        }
    }

    let java_bins = find_java_in_dir(&dest_root, exe_name);
    if java_bins.is_none() {
        crate::warn_msg!("Java runtime downloaded but java executable not found.");
        return None;
    }
    crate::success!("Java runtime {} installed -> {}", ver_name, dest_root.display());
    java_bins
}

fn find_java_in_dir(root: &Path, exe_name: &str) -> Option<String> {
    let pattern = format!("{}/**/bin/{}", root.display(), exe_name);
    let matches = glob::glob(&pattern).ok()?;
    for entry in matches.flatten() {
        return Some(entry.to_string_lossy().to_string());
    }
    None
}
