//! HTTP client with retry logic and file download with progress.

use std::fs::{self, File};
use std::io::{Read, Write};

use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::time::Duration;

use crate::util;

const LAUNCHER_NAME: &str = "simple-mc-cli";
const LAUNCHER_VER: &str = env!("CARGO_PKG_VERSION");

/// BMCLAPI mirror URL transformation.
/// Replace Mojang/Forge/Fabric/NeoForge official domains with BMCLAPI mirror.
/// See: https://bmclapidoc.bangbang93.com/
fn mirror_url(url: &str) -> String {
    url.replace("https://launchermeta.mojang.com", "https://bmclapi2.bangbang93.com")
        .replace("https://launcher.mojang.com", "https://bmclapi2.bangbang93.com")
        .replace(
            "https://piston-meta.mojang.com",
            "https://bmclapi2.bangbang93.com",
        )
        .replace(
            "https://resources.download.minecraft.net",
            "https://bmclapi2.bangbang93.com/assets",
        )
        .replace(
            "https://libraries.minecraft.net",
            "https://bmclapi2.bangbang93.com/maven",
        )
        .replace(
            "https://maven.minecraftforge.net",
            "https://bmclapi2.bangbang93.com/maven",
        )
        .replace(
            "https://files.minecraftforge.net",
            "https://bmclapi2.bangbang93.com/maven",
        )
        .replace(
            "https://meta.fabricmc.net",
            "https://bmclapi2.bangbang93.com/fabric-meta",
        )
        .replace(
            "https://maven.fabricmc.net",
            "https://bmclapi2.bangbang93.com/maven",
        )
        .replace(
            "https://maven.neoforged.net",
            "https://bmclapi2.bangbang93.com/maven",
        )
}

/// Result type: (status_code, response_body_bytes) or error string.
pub type HttpResult = Result<(u16, Vec<u8>), String>;

/// Low-level HTTP request with retry logic.
pub fn http_request(
    method: &str,
    url: &str,
    body_data: Option<&[u8]>,
    json_data: Option<&serde_json::Value>,
    extra_headers: Option<&[(&str, &str)]>,
    timeout_secs: u64,
    max_retries: u32,
) -> HttpResult {
    http_request_inner(method, url, body_data, json_data, extra_headers, timeout_secs, max_retries, true)
}

/// Low-level HTTP request without BMCLAPI mirroring (uses original URL directly).
pub fn http_request_no_mirror(
    method: &str,
    url: &str,
    body_data: Option<&[u8]>,
    json_data: Option<&serde_json::Value>,
    extra_headers: Option<&[(&str, &str)]>,
    timeout_secs: u64,
    max_retries: u32,
) -> HttpResult {
    http_request_inner(method, url, body_data, json_data, extra_headers, timeout_secs, max_retries, false)
}

#[allow(clippy::too_many_arguments)]
fn http_request_inner(
    method: &str,
    url: &str,
    body_data: Option<&[u8]>,
    json_data: Option<&serde_json::Value>,
    extra_headers: Option<&[(&str, &str)]>,
    timeout_secs: u64,
    max_retries: u32,
    use_mirror: bool,
) -> HttpResult {
    let target_url = if use_mirror { mirror_url(url) } else { url.to_string() };
    let mut last_err = String::new();
    let mut last_body: Vec<u8> = Vec::new();

    for attempt in 0..max_retries {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(timeout_secs))
            .build();

        let mut req = match method {
            "POST" => agent.post(&target_url),
            _ => agent.get(&target_url),
        };

        req = req.set(
            "User-Agent",
            &format!("{}/{}", LAUNCHER_NAME, LAUNCHER_VER),
        );

        if json_data.is_some() {
            req = req.set("Content-Type", "application/json");
        } else if body_data.is_some() {
            req = req.set("Content-Type", "application/x-www-form-urlencoded");
        }

        if let Some(hdrs) = extra_headers {
            for (k, v) in hdrs {
                req = req.set(k, v);
            }
        }

        let result = if let Some(json) = json_data {
            req.send_json(json.clone())
        } else if let Some(data) = body_data {
            req.send_bytes(data)
        } else {
            req.call()
        };

        match result {
            Ok(resp) => {
                let status = resp.status();
                let mut body = Vec::new();
                let _ = resp.into_reader().read_to_end(&mut body);

                if status < 400 || (400..500).contains(&status) {
                    return Ok((status, body));
                }
                last_err = format!("HTTP {}", status);
                last_body = body;
            }
            Err(ureq::Error::Status(status, resp)) => {
                let mut body = Vec::new();
                let _ = resp.into_reader().read_to_end(&mut body);
                if (400..500).contains(&status) {
                    return Ok((status, body));
                }
                last_err = format!("HTTP {}", status);
                last_body = body;
            }
            Err(ureq::Error::Transport(e)) => {
                last_err = format!("Transport: {}", e);
            }
        }

        if attempt + 1 < max_retries {
            let delay = 2u64.pow(attempt);
            let name = url.rsplit('/').next().unwrap_or(url);
            crate::warn_msg!(
                "[retry {}/{}] {} failed ({}), retrying in {}s...",
                attempt + 1,
                max_retries - 1,
                name,
                last_err,
                delay
            );
            std::thread::sleep(Duration::from_secs(delay));
        }
    }

    if !last_body.is_empty() {
        Ok((0, last_body))
    } else {
        Err(last_err)
    }
}

pub fn http_get(url: &str) -> HttpResult {
    http_request("GET", url, None, None, None, 30, 3)
}

pub fn http_get_no_mirror(url: &str) -> HttpResult {
    http_request_no_mirror("GET", url, None, None, None, 30, 3)
}

pub fn http_get_hdrs(url: &str, headers: &[(&str, &str)]) -> HttpResult {
    http_request("GET", url, None, None, Some(headers), 30, 3)
}

pub fn http_post_json(url: &str, json: &serde_json::Value) -> HttpResult {
    http_request("POST", url, None, Some(json), None, 30, 3)
}

pub fn http_post_form(url: &str, data: &[u8]) -> HttpResult {
    http_request("POST", url, Some(data), None, None, 30, 3)
}

/// Download a file with progress bar and optional SHA-1 verification.
/// Returns Ok(()) on success, or Err with an error message on failure.
/// Automatically tries BMCLAPI mirror first, falls back to original URL.
pub fn download_file(
    url: &str,
    dest: &Path,
    label: &str,
    sha1_expected: Option<&str>,
    max_retries: u32,
    show_progress: bool,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).ok();
    }

    let display_label = if label.is_empty() {
        dest.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string()
    } else {
        label.to_string()
    };

    // Check existing file
    if dest.exists() {
        if let Some(expected) = sha1_expected {
            if util::sha1_file(dest) == expected {
                return Ok(());
            }
        } else if util::is_jar_intact(dest) {
            return Ok(());
        } else if dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            crate::warn_msg!("{} appears corrupted, re-downloading...", display_label);
        }
        fs::remove_file(dest).ok();
    }

    let mirrored = mirror_url(url);

    // Try mirrored URL first
    let result = download_file_inner(&mirrored, dest, &display_label, sha1_expected, max_retries, show_progress);
    if result.is_ok() || mirrored == url {
        return result;
    }

    // Fall back to original URL
    crate::warn_msg!("Mirror failed for {}, trying official URL...", display_label);
    download_file_inner(url, dest, &display_label, sha1_expected, 2, show_progress)
}

fn download_file_inner(
    url: &str,
    dest: &Path,
    display_label: &str,
    sha1_expected: Option<&str>,
    max_retries: u32,
    show_progress: bool,
) -> Result<(), String> {
    let mut last_err = String::new();
    for attempt in 0..max_retries {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .build();

        match agent
            .get(url)
            .set(
                "User-Agent",
                &format!("{}/{}", LAUNCHER_NAME, LAUNCHER_VER),
            )
            .call()
        {
            Ok(resp) => {
                let total = resp
                    .header("Content-Length")
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);

                let mut reader = resp.into_reader();
                let mut file = File::create(dest).map_err(|e| {
                    format!("Cannot create {}: {}", dest.display(), e)
                })?;
                let mut buf = [0u8; 65536];
                let pb = if show_progress && total > 0 {
                    let pb = ProgressBar::new(total);
                    pb.set_style(
                        ProgressStyle::default_bar()
                            .template(
                                "{msg:40} [{bar:25}] {bytes}/{total_bytes} ({eta})",
                            )
                            .unwrap(),
                    );
                    pb.set_message(display_label.to_string());
                    Some(pb)
                } else {
                    None
                };

                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            file.write_all(&buf[..n]).ok();
                            if let Some(ref pb) = pb {
                                pb.inc(n as u64);
                            }
                        }
                        Err(_) => break,
                    }
                }
                drop(file);

                if let Some(ref pb) = pb {
                    pb.finish_with_message(format!("{} done", display_label));
                }

                if let Some(expected) = sha1_expected {
                    if util::sha1_file(dest) != expected {
                        fs::remove_file(dest).ok();
                        last_err = format!("SHA-1 mismatch for {}", display_label);
                        crate::warn_msg!("{}, retrying...", last_err);
                        continue;
                    }
                }
                return Ok(());
            }
            Err(e) => {
                if dest.exists() {
                    fs::remove_file(dest).ok();
                }
                last_err = format!("{}", e);
                if attempt + 1 < max_retries {
                    let delay = 2u64.pow(attempt + 1);
                    crate::warn_msg!(
                        "{} failed ({}), retry {}/{} in {}s...",
                        display_label,
                        e,
                        attempt + 2,
                        max_retries,
                        delay
                    );
                    std::thread::sleep(Duration::from_secs(delay));
                }
            }
        }
    }
    Err(format!(
        "Download failed after {} attempts: {} -- {}",
        max_retries, display_label, last_err
    ))
}
