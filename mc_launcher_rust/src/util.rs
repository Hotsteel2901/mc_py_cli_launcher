//! Platform utilities, hashing, file operations.

use std::fs::File;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::{env, fs};

use sha1::{Digest, Sha1};

/// Detect OS name (matches Mojang naming).
pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

/// Detect architecture.
pub fn os_arch() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "unknown"
    }
}

/// Default game directory: <exe_dir>/minecraft
pub fn default_game_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("minecraft")
}

/// Compute SHA-1 hash of a file.
pub fn sha1_file(path: &Path) -> String {
    let mut hasher = Sha1::new();
    if let Ok(mut f) = File::open(path) {
        let mut buf = [0u8; 65536];
        loop {
            match f.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => hasher.update(&buf[..n]),
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

/// Check if a JAR file appears intact (starts with ZIP magic).
pub fn is_jar_intact(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(m) if m.len() >= 22 => File::open(path).ok().map_or(false, |mut f| {
            let mut magic = [0u8; 4];
            f.read_exact(&mut magic).is_ok() && &magic == b"PK\x03\x04"
        }),
        _ => false,
    }
}

/// Convert a Maven coordinate string to a relative path.
pub fn maven_rel_path(name: &str) -> String {
    let parts: Vec<&str> = name.split(':').collect();
    let (g, a, v) = (parts[0], parts[1], parts[2]);
    let classifier = parts.get(3).copied().unwrap_or("");
    let jar = if classifier.is_empty() {
        format!("{}-{}.jar", a, v)
    } else {
        format!("{}-{}-{}.jar", a, v, classifier)
    };
    format!("{}/{}/{}/{}", g.replace('.', "/"), a, v, jar)
}

/// Generate an offline-mode UUID from a username (Mojang algorithm).
pub fn offline_uuid(username: &str) -> String {
    let name = format!("OfflinePlayer:{}", username);
    let digest = md5::Md5::digest(name.as_bytes());
    let mut b = <[u8; 16]>::from(digest);
    b[6] = (b[6] & 0x0F) | 0x30;
    b[8] = (b[8] & 0x3F) | 0x80;
    let h = hex::encode(b);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// Find a free TCP port.
#[allow(dead_code)]
pub fn find_free_port() -> u16 {
    TcpStream::connect(("127.0.0.1", 0))
        .ok()
        .and_then(|s| s.local_addr().ok())
        .map(|a| a.port())
        .unwrap_or(25565)
}

/// Format a UUID string to include dashes if it's 32 hex chars.
pub fn format_uuid(raw: &str) -> String {
    if raw.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &raw[0..8],
            &raw[8..12],
            &raw[12..16],
            &raw[16..20],
            &raw[20..32]
        )
    } else {
        raw.to_string()
    }
}
