//! HTTP client with retry logic and file download with progress.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::util;

const LAUNCHER_NAME: &str = "simple-mc-cli";
const LAUNCHER_VER: &str = "2.1.0";

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
    let mut last_err = String::new();
    let mut last_body: Vec<u8> = Vec::new();

    for attempt in 0..max_retries {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(timeout_secs))
            .build();

        let mut req = match method {
            "POST" => agent.post(url),
            _ => agent.get(url),
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
/// Returns Ok on success; panics via die! on unrecoverable failure.
pub fn download_file(
    url: &str,
    dest: &Path,
    label: &str,
    sha1_expected: Option<&str>,
    max_retries: u32,
    show_progress: bool,
) {
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
                return;
            }
        } else if util::is_jar_intact(dest) {
            return;
        } else if dest.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            crate::warn_msg!("{} appears corrupted, re-downloading...", display_label);
        }
        fs::remove_file(dest).ok();
    }

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
                let mut file = File::create(dest).unwrap();
                let mut buf = [0u8; 65536];
                let mut downloaded: u64 = 0;

                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            file.write_all(&buf[..n]).ok();
                            downloaded += n as u64;
                            if show_progress && total > 0 {
                                let pct = (downloaded * 100 / total).min(100) as usize;
                                let filled = pct * 25 / 100;
                                let bar: String = (0..25)
                                    .map(|i| {
                                        if i < filled { '\u{2588}' } else { '\u{2591}' }
                                    })
                                    .collect();
                                print!(
                                    "\r  {:40} {} {:3}%",
                                    display_label, bar, pct
                                );
                                io::stdout().flush().ok();
                            }
                        }
                        Err(_) => break,
                    }
                }
                drop(file);

                if show_progress {
                    if total > 0 {
                        let bar_done: String = (0..25).map(|_| '\u{2588}').collect();
                        let mb = downloaded as f64 / 1_048_576.0;
                        println!(
                            "\r  {:40} {} {:.1} MB",
                            display_label, bar_done, mb
                        );
                    } else {
                        println!(
                            "\r  {:40} done ({:.0} KB)",
                            display_label,
                            downloaded as f64 / 1024.0
                        );
                    }
                }

                if let Some(expected) = sha1_expected {
                    if util::sha1_file(dest) != expected {
                        fs::remove_file(dest).ok();
                        crate::warn_msg!(
                            "SHA-1 mismatch for {}, retrying...",
                            display_label
                        );
                        continue;
                    }
                }
                return;
            }
            Err(e) => {
                if dest.exists() {
                    fs::remove_file(dest).ok();
                }
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
                } else {
                    crate::die!(
                        format!(
                            "Download failed after {} attempts: {} -- {}",
                            max_retries, display_label, e
                        )
                    );
                }
            }
        }
    }
}
