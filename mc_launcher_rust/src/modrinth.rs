//! Modrinth API client — search, project info, versions, download.

use std::path::{Path, PathBuf};

use crate::http;

const MODRINTH_API: &str = "https://api.modrinth.com/v2";
#[allow(dead_code)]
const LAUNCHER_NAME: &str = "simple-mc-cli";
#[allow(dead_code)]
const LAUNCHER_VER: &str = env!("CARGO_PKG_VERSION");

fn api_get(path: &str, params: &[(&str, &str)]) -> serde_json::Value {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in params {
        serializer.append_pair(k, v);
    }
    let qs = serializer.finish();

    let url = if qs.is_empty() {
        format!("{}{}", MODRINTH_API, path)
    } else {
        format!("{}{}?{}", MODRINTH_API, path, qs)
    };

    let (status, body) = http::http_get(&url).unwrap_or_else(|e| {
        crate::die!(format!("Modrinth API error: {}", e));
    });

    if status != 200 {
        let hint = String::from_utf8_lossy(&body)
            .chars()
            .take(300)
            .collect::<String>();
        crate::die!(
            format!("Modrinth API returned {} for {}", status, path),
            &hint
        );
    }

    serde_json::from_slice(&body).unwrap_or_else(|_| {
        crate::die!("Failed to parse Modrinth API response");
    })
}

/// List all game versions from Modrinth.
pub fn list_game_versions() -> serde_json::Value {
    api_get("/tag/game_version", &[])
}

/// List all mod loaders from Modrinth.
pub fn list_loaders() -> Vec<String> {
    let data = api_get("/tag/loader", &[]);
    data.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Search for projects on Modrinth.
pub fn search_projects(
    query: &str,
    index: &str,
    limit: u32,
    offset: u32,
    facets: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut params = vec![
        ("query".to_string(), query.to_string()),
        ("index".to_string(), index.to_string()),
        ("limit".to_string(), limit.to_string()),
        ("offset".to_string(), offset.to_string()),
    ];

    if let Some(f) = facets {
        params.push(("facets".to_string(), f.to_string()));
    }

    let str_params: Vec<(&str, &str)> =
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    api_get("/search", &str_params)
}

/// Get a single project by slug or ID.
pub fn get_project(slug_or_id: &str) -> serde_json::Value {
    api_get(&format!("/project/{}", slug_or_id), &[])
}

/// Get versions for a project.
pub fn get_project_versions(
    slug_or_id: &str,
    loaders: Option<&[String]>,
    game_versions: Option<&[String]>,
) -> serde_json::Value {
    let mut params: Vec<(String, String)> = Vec::new();

    if let Some(l) = loaders {
        let json_str = serde_json::to_string(l).unwrap_or_default();
        params.push(("loaders".to_string(), json_str));
    }
    if let Some(gv) = game_versions {
        let json_str = serde_json::to_string(gv).unwrap_or_default();
        params.push(("game_versions".to_string(), json_str));
    }

    let str_params: Vec<(&str, &str)> =
        params.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    api_get(
        &format!("/project/{}/version", slug_or_id),
        &str_params,
    )
}

/// Get a single version by ID.
#[allow(dead_code)]
pub fn get_version(version_id: &str) -> serde_json::Value {
    api_get(&format!("/version/{}", version_id), &[])
}

/// Get multiple versions by ID.
#[allow(dead_code)]
pub fn get_versions(version_ids: &[&str]) -> serde_json::Value {
    let json_str = serde_json::to_string(version_ids).unwrap_or_default();
    api_get("/versions", &[("ids", &json_str)])
}

/// Download files from a version object into a directory.
/// Returns paths to downloaded files.
pub fn download_version_files(
    version_data: &serde_json::Value,
    dest_dir: &Path,
    label: &str,
) -> Vec<PathBuf> {
    let dest = dest_dir;
    std::fs::create_dir_all(dest).ok();
    let mut paths = Vec::new();

    let files = version_data["files"].as_array();
    if files.is_none() {
        return paths;
    }
    let files = files.unwrap();

    // Prefer primary files, then filter out sources/javadoc
    let primary_files: Vec<&serde_json::Value> = files
        .iter()
        .filter(|f| f["primary"].as_bool().unwrap_or(false))
        .collect();

    let selected: Vec<&serde_json::Value> = if !primary_files.is_empty() {
        primary_files
    } else {
        files
            .iter()
            .filter(|f| {
                let name = f["filename"].as_str().unwrap_or("");
                !name.ends_with("-sources.jar") && !name.ends_with("-javadoc.jar")
            })
            .collect()
    };

    let mut downloaded_names = std::collections::HashSet::new();

    for f in &selected {
        let filename = f["filename"].as_str().unwrap_or("unknown.jar");
        let file_path = dest.join(filename);
        let url = f["url"].as_str().unwrap_or("");

        if !file_path.exists() {
            if let Err(e) = http::download_file(
                url,
                &file_path,
                if label.is_empty() { filename } else { label },
                None,
                3,
                true,
            ) {
                crate::warn_msg!("Failed to download {}: {}", filename, e);
                continue;
            }
        } else {
            crate::info!("{} -- cached", filename);
        }
        paths.push(file_path);
        downloaded_names.insert(filename.to_string());
    }

    // Clean up stale sources/javadoc jars if a primary jar was downloaded
    if !downloaded_names.is_empty() {
        for suffix in &["-sources.jar", "-javadoc.jar"] {
            if let Ok(entries) = std::fs::read_dir(dest) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.ends_with(suffix) {
                        let base = format!(
                            "{}.jar",
                            &name[..name.len() - suffix.len()]
                        );
                        if downloaded_names.contains(&base)
                            && entry.path().exists()
                        {
                            std::fs::remove_file(entry.path()).ok();
                            crate::info!(
                                "Removed stale {} jar: {}",
                                &suffix[1..suffix.len() - 4],
                                name
                            );
                        }
                    }
                }
            }
        }
    }

    paths
}
