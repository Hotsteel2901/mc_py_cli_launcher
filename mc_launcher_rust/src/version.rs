//! Minecraft version management — manifest, client jar, libraries, assets, natives.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;

use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};
use crate::http;
use crate::util;

const MC_MANIFEST: &str =
    "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";

pub struct VersionManager {
    pub game_dir: PathBuf,
    pub versions_dir: PathBuf,
    pub libraries_dir: PathBuf,
    pub assets_dir: PathBuf,
}

impl VersionManager {
    pub fn new(game_dir: &Path) -> Self {
        VersionManager {
            game_dir: game_dir.to_path_buf(),
            versions_dir: game_dir.join("versions"),
            libraries_dir: game_dir.join("libraries"),
            assets_dir: game_dir.join("assets"),
        }
    }

    /// Fetch (or use cached) Minecraft version manifest.
    pub fn fetch_manifest(&self) -> AppResult<serde_json::Value> {
        let manifest_path = self.game_dir.join("version_manifest_v2.json");

        // Use cache if less than 5 minutes old
        if manifest_path.exists() {
            if let Ok(meta) = fs::metadata(&manifest_path) {
                if let Ok(mod_time) = meta.modified() {
                    if let Ok(elapsed) = SystemTime::now().duration_since(mod_time) {
                        if elapsed.as_secs() < 300 {
                            if let Ok(data) = fs::read_to_string(&manifest_path) {
                                if let Ok(json) = serde_json::from_str(&data) {
                                    return Ok(json);
                                }
                            }
                        }
                    }
                }
            }
        }

        let (status, body) = http::http_get(MC_MANIFEST)
            .map_err(|e| AppError::Http(format!("Cannot fetch version manifest: {}", e)))?;
        if status != 200 {
            let hint = String::from_utf8_lossy(&body)
                .chars()
                .take(300)
                .collect::<String>();
            return Err(AppError::Http(format!(
                "Cannot fetch version manifest ({}) -- {}",
                status, hint
            )));
        }

        fs::create_dir_all(&self.game_dir).ok();
        fs::write(&manifest_path, &body).ok();
        Ok(serde_json::from_slice(&body)?)
    }

    /// Get version info for a specific version ID (or "latest" / "latest-snapshot").
    pub fn get_version_info(
        &self,
        version_id: Option<&str>,
    ) -> AppResult<(String, serde_json::Value)> {
        let manifest = self.fetch_manifest()?;
        let mut vid = version_id.unwrap_or("latest").to_string();

        if vid == "latest" {
            vid = manifest["latest"]["release"].as_str().unwrap_or("").to_string();
        } else if vid == "latest-snapshot" {
            vid = manifest["latest"]["snapshot"].as_str().unwrap_or("").to_string();
        }

        let entry = manifest["versions"]
            .as_array()
            .and_then(|versions| versions.iter().find(|v| v["id"].as_str() == Some(&vid)));

        let entry = match entry {
            Some(e) => e,
            None => {
                let avail: Vec<_> = manifest["versions"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .take(15)
                            .filter_map(|v| v["id"].as_str())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let hint = format!("Available (Mojang): {}...", avail.join(", "));
                return Err(AppError::Generic(format!(
                    "Version '{}' not found. {}",
                    vid, hint
                )));
            }
        };

        let json_path = self.versions_dir.join(&vid).join(format!("{}.json", vid));
        if !json_path.exists() {
            crate::info!("Downloading version manifest for {}...", vid);
            if let Err(e) = http::download_file(
                entry["url"].as_str().unwrap_or(""),
                &json_path,
                &format!("{}.json", vid),
                None,
                3,
                true,
            ) {
                return Err(AppError::Http(format!(
                    "Cannot download version manifest: {}",
                    e
                )));
            }
        }

        let version_data: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&json_path)?)?;
        Ok((vid, version_data))
    }

    /// Download the client JAR for a version.
    /// Follows HMCL's Version.getDownloadInfo() — falls back to BMCLAPI versions URL.
    pub fn download_client_jar(
        &self,
        version_id: &str,
        version_data: &serde_json::Value,
    ) -> AppResult<PathBuf> {
        let jar_path = self
            .versions_dir
            .join(version_id)
            .join(format!("{}.jar", version_id));
        if jar_path.exists() {
            return Ok(jar_path);
        }
        // HMCL: if downloads.client is null, use DEFAULT_VERSION_DOWNLOAD_URL
        let (url, sha1) = if version_data["downloads"]["client"].is_null() {
            (
                format!(
                    "https://bmclapi2.bangbang93.com/versions/{}/{}.jar",
                    version_id, version_id
                ),
                None,
            )
        } else {
            (
                version_data["downloads"]["client"]["url"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                version_data["downloads"]["client"]["sha1"]
                    .as_str()
                    .map(|s| s.to_string()),
            )
        };
        crate::info!("Downloading client jar for {}...", version_id);
        if let Err(e) = http::download_file(
            &url,
            &jar_path,
            &format!("{}.jar", version_id),
            sha1.as_deref(),
            3,
            true,
        ) {
            return Err(AppError::Http(format!("Cannot download client jar: {}", e)));
        }
        Ok(jar_path)
    }

    /// Determine if a native library classifier matches current OS/arch.
    fn needs_natives(&self, _lib_name: &str, classifiers: &[String]) -> Option<String> {
        let osn = util::os_name();
        let arch = util::os_arch();

        for c in classifiers {
            let cl = c.to_lowercase();
            match osn {
                "windows" if cl.contains("windows") => {
                    if arch == "x86_64" && cl.contains("64") {
                        return Some(c.clone());
                    }
                    if arch == "arm64" && cl.contains("arm64") {
                        return Some(c.clone());
                    }
                    if !cl.contains("64") && !cl.contains("arm64") && !cl.contains("32") {
                        return Some(c.clone());
                    }
                }
                "linux" if cl.contains("linux") => {
                    if arch == "arm64" && cl.contains("arm64") {
                        return Some(c.clone());
                    }
                    if arch == "x86_64" && !cl.contains("arm64") && !cl.contains("32") {
                        return Some(c.clone());
                    }
                }
                "osx" if cl.contains("osx") || cl.contains("macos") => {
                    if arch == "arm64" && cl.contains("arm64") {
                        return Some(c.clone());
                    }
                    if arch == "x86_64" && !cl.contains("arm64") {
                        return Some(c.clone());
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Evaluate library rules to determine if a library should be included.
    pub fn rules_allow(rules: &serde_json::Value, default: bool) -> bool {
        let arr = match rules.as_array() {
            Some(a) => a,
            None => return default,
        };
        let mut allowed = default;
        for rule in arr {
            let action = rule["action"].as_str().unwrap_or("allow");
            let mut matches = true;

            if let Some(os_rule) = rule.get("os") {
                matches = true;
                if let Some(name) = os_rule["name"].as_str() {
                    if name != util::os_name() {
                        matches = false;
                    }
                }
                if matches {
                    if let Some(arch) = os_rule["arch"].as_str() {
                        if arch != util::os_arch() {
                            matches = false;
                        }
                    }
                }
            } else if rule.get("features").is_some() {
                // Features rules — skip for now
                matches = false;
            }

            if matches {
                allowed = action == "allow";
            }
        }
        allowed
    }

    /// Download all libraries and extract native libraries.
    /// Returns paths to all JAR files for the classpath.
    /// Follows HMCL's Library.java logic: native libs download classifier JARs, not main artifact.
    pub fn download_libraries(
        &self,
        version_data: &serde_json::Value,
        natives_dir: &Path,
        max_workers: usize,
    ) -> AppResult<Vec<PathBuf>> {
        let libs = version_data["libraries"].as_array();
        if libs.is_none() {
            return Ok(Vec::new());
        }
        let libs = libs.unwrap();
        let osn = util::os_name();

        let mut all_downloads: Vec<(String, PathBuf, String, Option<String>)> = Vec::new(); // (url, dest, label, sha1)
        let mut all_jars: Vec<PathBuf> = Vec::new();

        for lib in libs {
            let name = lib["name"].as_str().unwrap_or("");
            if name.is_empty() {
                continue;
            }

            let parts: Vec<&str> = name.split(':').collect();
            if parts.len() < 3 {
                continue;
            }
            let (group, artifact, version) = (parts[0], parts[1], parts[2]);

            let group_path = group.replace('.', "/");

            // Check rules
            if let Some(rules) = lib.get("rules") {
                if !Self::rules_allow(rules, true) {
                    continue;
                }
            }

            let has_natives = lib.get("natives").is_some();
            let downloads = lib.get("downloads");

            // ── Determine what to download (following HMCL Library.java) ──
            // For native libs (has 'natives' field): download classifier JAR only, skip main artifact.
            // For regular libs: download downloads.artifact, or construct Maven URL.
            if has_natives {
                // Native library — resolve classifier for current OS
                let native_classifier = if let Some(natives) = lib.get("natives") {
                    natives[osn].as_str().map(|s| s.replace("${arch}", util::os_arch()))
                } else {
                    None
                };

                if let Some(nc) = native_classifier {
                    let lib_dir = self
                        .libraries_dir
                        .join(&group_path)
                        .join(artifact)
                        .join(version);

                    // Try downloads.classifiers[native_classifier] first
                    let (url, sha1) = if let Some(classifiers) =
                        downloads.and_then(|d| d.get("classifiers"))
                    {
                        if let Some(info) = classifiers.get(&nc) {
                            (
                                info["url"].as_str().unwrap_or("").to_string(),
                                info["sha1"].as_str().map(|s| s.to_string()),
                            )
                        } else {
                            // Try finding a matching classifier key
                            let classifier_keys: Vec<String> = classifiers
                                .as_object()
                                .map(|obj| obj.keys().cloned().collect())
                                .unwrap_or_default();
                            let match_key = self.needs_natives(name, &classifier_keys);
                            if let Some(mk) = match_key {
                                if let Some(info) = classifiers.get(&mk) {
                                    (
                                        info["url"].as_str().unwrap_or("").to_string(),
                                        info["sha1"].as_str().map(|s| s.to_string()),
                                    )
                                } else {
                                    (String::new(), None)
                                }
                            } else {
                                (String::new(), None)
                            }
                        }
                    } else {
                        (String::new(), None)
                    };

                    let jar_name = format!("{}-{}-{}.jar", artifact, version, nc);
                    let jar_path = lib_dir.join(&jar_name);

                    if !jar_path.exists() && !url.is_empty() {
                        all_downloads.push((
                            url,
                            jar_path.clone(),
                            format!("{}:{} [native]", group, artifact),
                            sha1,
                        ));
                    }
                    all_jars.push(jar_path);
                }
            } else {
                // Regular (non-native) library
                let lib_dir = self
                    .libraries_dir
                    .join(&group_path)
                    .join(artifact)
                    .join(version);
                let jar_name = format!("{}-{}.jar", artifact, version);
                let jar_path = lib_dir.join(&jar_name);

                if !jar_path.exists() {
                    // Try downloads.artifact first (HMCL: getRawDownloadInfo → downloads.artifact)
                    let (url, sha1) = if let Some(artifact_info) =
                        downloads.and_then(|d| d.get("artifact"))
                    {
                        let url = artifact_info["url"].as_str().unwrap_or("");
                        let sha1 = artifact_info["sha1"].as_str().map(|s| s.to_string());
                        (url.to_string(), sha1)
                    } else {
                        // Fallback: construct URL from library.url or default libraries.minecraft.net
                        // (HMCL: computePath → repo + path)
                        let repo = lib["url"]
                            .as_str()
                            .unwrap_or("https://libraries.minecraft.net/");
                        let repo = if repo.ends_with('/') {
                            repo.to_string()
                        } else {
                            format!("{}/", repo)
                        };
                        let path = format!(
                            "{}/{}/{}/{}",
                            group_path, artifact, version, jar_name
                        );
                        (format!("{}{}", repo, path), None)
                    };

                    if !url.is_empty() {
                        all_downloads.push((
                            url,
                            jar_path.clone(),
                            format!("{}:{}", group, artifact),
                            sha1,
                        ));
                    }
                }
                all_jars.push(jar_path);
            }
        }

        let new_count = all_downloads.len();
        crate::info!(
            "Libraries: {} total, {} to download [{} threads]...",
            all_jars.len(),
            new_count,
            max_workers
        );

        if !all_downloads.is_empty() {
            let total = all_downloads.len();
            let pb = ProgressBar::new(total as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{msg:40} [{bar:25}] {pos}/{len}")
                    .unwrap(),
            );
            pb.set_message("Libraries");

            let failed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            all_downloads
                .par_iter()
                .for_each(|(url, dest, label, sha1)| {
                    if dest.exists() {
                        pb.inc(1);
                        return;
                    }
                    if let Err(e) = http::download_file(
                        url,
                        dest,
                        label,
                        sha1.as_deref(),
                        2,
                        false,
                    ) {
                        let mut failed = failed.lock().unwrap();
                        failed.push(format!("{}: {}", label, e));
                    }
                    pb.inc(1);
                });

            pb.finish_with_message("Libraries done");

            let failed = Arc::try_unwrap(failed).unwrap().into_inner().unwrap();
            if !failed.is_empty() {
                return Err(AppError::Http(format!(
                    "Failed to download {} library(s):\n    {}",
                    failed.len(),
                    failed.join("\n    ")
                )));
            }
        }

        // Extract natives from downloaded jars
        for (_url, dest, label, _sha1) in &all_downloads {
            if label.contains("[native]") && dest.exists() {
                self.extract_natives(dest, natives_dir);
            }
        }

        // Also check any cached native jars
        let mut seen: std::collections::HashSet<PathBuf> =
            all_downloads.iter().map(|(_, d, _, _)| d.clone()).collect();

        for lib in libs {
            if let Some(rules) = lib.get("rules") {
                if !Self::rules_allow(rules, true) {
                    continue;
                }
            }
            let name = lib["name"].as_str().unwrap_or("");
            let parts: Vec<&str> = name.split(':').collect();
            if parts.len() < 3 {
                continue;
            }
            let (group, artifact, version) = (parts[0], parts[1], parts[2]);
            let group_path = group.replace('.', "/");
            let lib_dir = self
                .libraries_dir
                .join(&group_path)
                .join(artifact)
                .join(version);

            if let Some(natives) = lib.get("natives") {
                if let Some(native_key) = natives[util::os_name()].as_str() {
                    let native_key = native_key.replace("${arch}", util::os_arch());
                    if let Some(classifiers) =
                        lib["downloads"].get("classifiers")
                    {
                        let classifier_keys: Vec<String> = classifiers
                            .as_object()
                            .map(|obj| obj.keys().cloned().collect())
                            .unwrap_or_default();
                        let match_key =
                            self.needs_natives(name, &classifier_keys);
                        let mk = match_key.unwrap_or(native_key);
                        if classifiers.get(&mk).is_some() {
                            let cached = lib_dir.join(format!(
                                "{}-{}-{}.jar",
                                artifact, version, mk
                            ));
                            if !seen.contains(&cached) && cached.exists() {
                                self.extract_natives(&cached, natives_dir);
                                seen.insert(cached);
                            }
                        }
                    }
                }
            }
        }

        Ok(all_jars)
    }

    /// Extract native libraries (dll, so, dylib) from a JAR file.
    pub fn extract_natives(&self, jar_path: &Path, natives_dir: &Path) {
        fs::create_dir_all(natives_dir).ok();

        // Check if this jar was already extracted
        let marker_path = natives_dir.join(".natives_extracted");
        let jar_name = jar_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let marker_content = if marker_path.exists() {
            std::fs::read_to_string(&marker_path).unwrap_or_default()
        } else {
            String::new()
        };
        if marker_content.contains(jar_name.as_ref()) {
            return;
        }

        let file = match fs::File::open(jar_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut archive = match zip::ZipArchive::new(file) {
            Ok(a) => a,
            Err(e) => {
                crate::warn_msg!(
                    "Failed to extract natives from {}: {}",
                    jar_path.display(),
                    e
                );
                return;
            }
        };

        let mut extracted = false;
        for i in 0..archive.len() {
            let mut entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let name = entry
                .name()
                .split('/')
                .next_back()
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                continue;
            }
            let ext = name
                .rsplit('.')
                .next()
                .unwrap_or("")
                .to_lowercase();
            if matches!(ext.as_str(), "dll" | "so" | "dylib" | "jnilib") {
                let target = natives_dir.join(&name);
                if !target.exists() {
                    let mut buf = Vec::new();
                    if entry.read_to_end(&mut buf).is_ok() {
                        fs::write(&target, &buf).ok();
                        extracted = true;
                    }
                }
            }
        }

        // Mark this jar as extracted
        if extracted {
            let mut marker = marker_content;
            if !marker.is_empty() {
                marker.push('\n');
            }
            marker.push_str(&jar_name);
            fs::write(&marker_path, &marker).ok();
        }
    }

    /// Download game assets.
    /// Follows HMCL's Version.getAssetIndex() logic with hardcoded hashes for old versions.
    pub fn download_assets(
        &self,
        version_data: &serde_json::Value,
        max_workers: usize,
    ) -> AppResult<String> {
        let asset_index = &version_data["assetIndex"];

        // If no assetIndex field, construct one from hardcoded hashes (HMCL: Version.getAssetIndex)
        let (index_id, index_url) = if asset_index.is_null() {
            let assets_id = version_data["assets"]
                .as_str()
                .unwrap_or("legacy")
                .to_string();
            let hash = match assets_id.as_str() {
                "1.8" => "f6ad102bcaa53b1a58358f16e376d548d44933ec",
                "14w31a" => "10a2a0e75b03cfb5a7196abbdf43b54f7fa61deb",
                "14w25a" => "32ff354a3be1c4dd83027111e6d79ee4d701d2c0",
                "1.7.4" => "545510a60f526b9aa8a38f9c0bc7a74235d21675",
                "1.7.10" => "1863782e33ce7b584fc45b037325a1964e095d3e",
                "1.7.3" => "f6cf726f4747128d13887010c2cbc44ba83504d9",
                "pre-1.6" => "3d8e55480977e32acd9844e545177e69a52f594b",
                _ => "770572e819335b6c0a053f8378ad88eda189fc14",
            };
            let assets_id = if matches!(assets_id.as_str(), "1.8" | "14w31a" | "14w25a" | "1.7.4" | "1.7.10" | "1.7.3" | "pre-1.6") {
                assets_id
            } else {
                "legacy".to_string()
            };
            let url = format!(
                "https://launchermeta.mojang.com/v1/packages/{}/{}.json",
                hash, assets_id
            );
            (assets_id, url)
        } else {
            (
                asset_index["id"].as_str().unwrap_or("").to_string(),
                asset_index["url"].as_str().unwrap_or("").to_string(),
            )
        };

        let index_path = self
            .assets_dir
            .join("indexes")
            .join(format!("{}.json", index_id));

        if !index_path.exists() {
            if let Err(e) = http::download_file(
                &index_url,
                &index_path,
                &format!("Asset index {}", index_id),
                None,
                3,
                true,
            ) {
                return Err(AppError::Http(format!("Cannot download asset index: {}", e)));
            }
        }

        let index_data: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&index_path).unwrap())
                .unwrap();
        let objects = index_data["objects"].as_object();
        if objects.is_none() {
            return Ok(index_id);
        }
        let objects = objects.unwrap();
        let total = objects.len();

        let mut missing: Vec<(String, PathBuf)> = Vec::new();
        for (_name, obj) in objects {
            let h = obj["hash"].as_str().unwrap_or("");
            if h.len() < 2 {
                continue;
            }
            let sub_dir = &h[..2];
            let obj_path = self.assets_dir.join("objects").join(sub_dir).join(h);
            if !obj_path.exists() {
                let url = format!(
                    "https://resources.download.minecraft.net/{}/{}",
                    sub_dir, h
                );
                missing.push((url, obj_path));
            }
        }

        if missing.is_empty() {
            crate::info!("Assets: all {} up to date.", total);
            return Ok(index_id);
        }

        crate::info!(
            "Assets: {}/{} to download [{} threads]...",
            missing.len(),
            total,
            max_workers
        );

        let total_missing = missing.len();
        let pb = ProgressBar::new(total_missing as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{msg:40} [{bar:25}] {pos}/{len}")
                .unwrap(),
        );
        pb.set_message("Assets");

        let failed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        missing.par_iter().for_each(|(url, dest)| {
            if let Err(e) = http::download_file(url, dest, "", None, 2, false) {
                let mut failed = failed.lock().unwrap();
                failed.push(format!("{}: {}", url, e));
            }
            pb.inc(1);
        });

        pb.finish_with_message("Assets done");

        let failed = Arc::try_unwrap(failed).unwrap().into_inner().unwrap();
        if !failed.is_empty() {
            return Err(AppError::Http(format!(
                "Failed to download {} asset(s):\n    {}",
                failed.len(),
                failed.join("\n    ")
            )));
        }

        Ok(index_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rules_allow_empty() {
        let rules = serde_json::json!([]);
        assert!(VersionManager::rules_allow(&rules, true));
        assert!(!VersionManager::rules_allow(&rules, false));
    }

    #[test]
    fn test_rules_allow_os_windows() {
        let rules = serde_json::json!([
            {"action": "allow", "os": {"name": "windows"}},
            {"action": "disallow", "os": {"name": "linux"}},
            {"action": "disallow", "os": {"name": "osx"}}
        ]);
        let result = VersionManager::rules_allow(&rules, false);
        // On Windows, this should be true; on Linux/macOS, false
        let expected = cfg!(windows);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_rules_allow_disallow_override() {
        let rules = serde_json::json!([
            {"action": "allow", "os": {"name": "linux"}},
            {"action": "disallow", "os": {"name": "linux"}}
        ]);
        // Last matching rule should win
        let result = VersionManager::rules_allow(&rules, true);
        let expected = !cfg!(target_os = "linux");
        assert_eq!(result, expected);
    }

    #[test]
    fn test_rules_allow_with_arch() {
        let rules = serde_json::json!([
            {"action": "allow", "os": {"name": "linux", "arch": "x86_64"}}
        ]);
        let result = VersionManager::rules_allow(&rules, false);
        let expected = cfg!(target_os = "linux") && cfg!(target_arch = "x86_64");
        assert_eq!(result, expected);
    }

    #[test]
    fn test_rules_allow_with_features() {
        // Features rules should be skipped (match = false)
        let rules = serde_json::json!([
            {"action": "allow", "features": {"is_demo_user": true}}
        ]);
        let result = VersionManager::rules_allow(&rules, false);
        // Features rules don't match, so default applies
        assert!(!result);
    }
}
