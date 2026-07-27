//! CurseForge API client — search, project info, files, download.
//! Ported from HMCL's CurseForgeRemoteAddonRepository.java.
//!
//! API key resolution (same strategy as HMCL):
//! 1. Runtime env var CURSEFORGE_API_KEY (highest priority, user override)
//! 2. Compile-time baked-in key (build env CURSEFORGE_API_KEY, embedded in binary)
//!
//! This way users don't need to configure anything — like HMCL.

use std::path::{Path, PathBuf};

use crate::http;

const CF_API: &str = "https://api.curseforge.com/v1";

/// Compile-time baked-in API key (from build environment).
/// If CURSEFORGE_API_KEY was set during `cargo build`, it's embedded here.
const BUILTIN_API_KEY: &str = match option_env!("CURSEFORGE_API_KEY") {
    Some(k) => k,
    None => "",
};

/// Get the CurseForge API key.
/// Priority: runtime env var > compile-time baked-in key.
fn api_key() -> Option<String> {
    // 1. Runtime env var (user override)
    if let Ok(k) = std::env::var("CURSEFORGE_API_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    // 2. Compile-time baked-in key
    if !BUILTIN_API_KEY.is_empty() {
        return Some(BUILTIN_API_KEY.to_string());
    }
    None
}

/// Check if CurseForge is available (has API key).
pub fn is_available() -> bool {
    api_key().is_some()
}

/// GET request with X-API-KEY header.
fn api_get(path: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let key = api_key().ok_or_else(|| {
        "CurseForge API key not set. Set CURSEFORGE_API_KEY environment variable.".to_string()
    })?;

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in params {
        serializer.append_pair(k, v);
    }
    let qs = serializer.finish();

    let url = if qs.is_empty() {
        format!("{}{}", CF_API, path)
    } else {
        format!("{}{}?{}", CF_API, path, qs)
    };

    let headers: [(&str, &str); 1] = [("X-API-KEY", &key)];
    let (status, body) = http::http_get_hdrs(&url, &headers)?;
    if status != 200 {
        let hint = String::from_utf8_lossy(&body)
            .chars()
            .take(300)
            .collect::<String>();
        return Err(format!("CurseForge API returned {} for {}: {}", status, path, hint));
    }
    serde_json::from_slice(&body).map_err(|e| format!("Failed to parse CurseForge response: {}", e))
}

/// Search for mods on CurseForge.
/// Returns a unified format similar to Modrinth hits for compatibility.
/// gameId=432 is Minecraft, classId=6 is mod.
pub fn search_projects(
    query: &str,
    limit: u32,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut params: Vec<(&str, String)> = vec![
        ("gameId", "432".to_string()),
        ("classId", "6".to_string()),
        ("searchFilter", query.to_string()),
        ("pageSize", limit.to_string()),
        ("index", "0".to_string()),
        ("sortField", "2".to_string()), // Popularity
        ("sortOrder", "desc".to_string()),
    ];

    if let Some(gv) = game_version {
        params.push(("gameVersion", gv.to_string()));
    }

    // CurseForge doesn't have a direct loader filter in search;
    // loaders are embedded in gameVersions. We filter client-side later.
    let _ = loader;

    let str_params: Vec<(&str, &str)> =
        params.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let result = api_get("/mods/search", &str_params)?;

    let data = result["data"].as_array().cloned().unwrap_or_default();

    // Convert to Modrinth-compatible format
    let hits: Vec<serde_json::Value> = data
        .iter()
        .map(|mod_data| {
            let id = mod_data["id"].as_i64().unwrap_or(0);
            let slug = mod_data["slug"].as_str().unwrap_or("");
            let title = mod_data["name"].as_str().unwrap_or(slug);
            let summary = mod_data["summary"].as_str().unwrap_or("");
            let downloads = mod_data["downloadCount"].as_i64().unwrap_or(0);
            let icon_url = mod_data["logo"]["thumbnailUrl"]
                .as_str()
                .or_else(|| mod_data["logo"]["url"].as_str())
                .unwrap_or("");

            // Build categories list
            let categories: Vec<String> = mod_data["categories"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| c["name"].as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            serde_json::json!({
                "project_id": id.to_string(),
                "slug": slug,
                "title": title,
                "description": summary,
                "downloads": downloads,
                "categories": categories,
                "icon_url": icon_url,
                "source": "curseforge",
                "author": mod_data["authors"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|a| a["name"].as_str())
                    .unwrap_or("?"),
            })
        })
        .collect();

    Ok(hits)
}

/// Get a single mod project by ID (CurseForge uses integer IDs, not slugs).
pub fn get_project(mod_id: &str) -> Result<serde_json::Value, String> {
    let result = api_get(&format!("/mods/{}", mod_id), &[])?;
    let data = result["data"].clone();

    // Convert to Modrinth-compatible format
    let title = data["name"].as_str().unwrap_or(mod_id);
    let slug = data["slug"].as_str().unwrap_or(mod_id);
    Ok(serde_json::json!({
        "id": data["id"].as_i64().map(|i| i.to_string()).unwrap_or_else(|| mod_id.to_string()),
        "slug": slug,
        "title": title,
        "description": data["summary"].as_str().unwrap_or(""),
        "source": "curseforge",
    }))
}

/// Get mod files (versions) for a project.
/// Returns Modrinth-compatible version array.
pub fn get_project_files(
    mod_id: &str,
    game_version: Option<&str>,
    loader: Option<&str>,
) -> Result<Vec<serde_json::Value>, String> {
    let result = api_get(
        &format!("/mods/{}/files", mod_id),
        &[("pageSize", "10000")],
    )?;

    let files = result["data"].as_array().cloned().unwrap_or_default();

    // Filter by game version and loader
    let mut versions: Vec<serde_json::Value> = files
        .iter()
        .filter(|f| {
            // Game version filter
            if let Some(gv) = game_version {
                let game_versions: Vec<&str> = f["gameVersions"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                    .unwrap_or_default();
                if !game_versions.contains(&gv) {
                    return false;
                }
            }
            // Loader filter (CurseForge embeds loader names in gameVersions)
            if let Some(ld) = loader {
                let game_versions: Vec<String> = f["gameVersions"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(|s| s.to_lowercase())
                            .collect()
                    })
                    .unwrap_or_default();
                if !game_versions.contains(&ld.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .map(|f| {
            let file_id = f["id"].as_i64().unwrap_or(0);
            let filename = f["fileName"].as_str().unwrap_or("unknown.jar");
            let download_url = f["downloadUrl"].as_str().unwrap_or("");
            // Fallback CDN URL if downloadUrl is null (distribution disabled)
            let url = if download_url.is_empty() {
                format!(
                    "https://edge.forgecdn.net/files/{}/{}",
                    file_id / 1000,
                    file_id % 1000
                )
            } else {
                download_url.to_string()
            };

            // Extract SHA-1 hash if available
            let sha1 = f["hashes"]
                .as_array()
                .and_then(|arr| {
                    arr.iter().find(|h| h["algo"].as_i64() == Some(1)) // 1 = SHA1
                })
                .and_then(|h| h["value"].as_str())
                .unwrap_or("");

            let game_versions: Vec<String> = f["gameVersions"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();

            // Extract loaders from gameVersions
            let loaders: Vec<String> = game_versions
                .iter()
                .filter(|gv| {
                    let lv = gv.to_lowercase();
                    matches!(lv.as_str(), "fabric" | "forge" | "quilt" | "neoforge")
                })
                .cloned()
                .collect();

            let release_type = f["releaseType"].as_i64().unwrap_or(1);
            let version_type = match release_type {
                1 => "release",
                2 => "beta",
                3 => "alpha",
                _ => "release",
            };

            serde_json::json!({
                "id": file_id.to_string(),
                "version_number": f["displayName"].as_str().unwrap_or(filename),
                "name": f["displayName"].as_str().unwrap_or(filename),
                "date_published": f["fileDate"].as_str().unwrap_or(""),
                "version_type": version_type,
                "game_versions": game_versions,
                "loaders": loaders,
                "files": [{
                    "filename": filename,
                    "url": url,
                    "primary": true,
                    "hashes": if sha1.is_empty() {
                        serde_json::json!({})
                    } else {
                        serde_json::json!({"sha1": sha1})
                    },
                }],
                "source": "curseforge",
            })
        })
        .collect();

    // Sort by date descending (newest first)
    versions.sort_by(|a, b| {
        b["date_published"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date_published"].as_str().unwrap_or(""))
    });

    Ok(versions)
}

/// Download files from a version object into a directory.
/// Reuses modrinth::download_version_files logic.
pub fn download_version_files(
    version_data: &serde_json::Value,
    dest_dir: &Path,
    label: &str,
) -> Vec<PathBuf> {
    // The version data is already in Modrinth-compatible format,
    // so we can reuse the same download logic.
    crate::modrinth::download_version_files(version_data, dest_dir, label)
}
