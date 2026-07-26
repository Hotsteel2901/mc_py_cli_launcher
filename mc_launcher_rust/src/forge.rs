//! Forge loader installation — manual extraction-based, no Java installer needed.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::http;
use crate::version::VersionManager;

const FORGE_MAVEN: &str = "https://maven.minecraftforge.net/net/minecraftforge/forge";
const FORGE_PROMOTIONS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const FORGE_MAVEN_META: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";

#[derive(Debug, Deserialize)]
struct Metadata {
    versioning: Versioning,
}

#[derive(Debug, Deserialize)]
struct Versions {
    version: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Versioning {
    versions: Versions,
}

pub struct ForgeManager {
    pub game_dir: PathBuf,
    pub lib_dir: PathBuf,
}

impl ForgeManager {
    pub fn new(game_dir: &Path) -> Self {
        ForgeManager {
            game_dir: game_dir.to_path_buf(),
            lib_dir: game_dir.join("libraries"),
        }
    }

    /// Get available Forge versions for a Minecraft version from Maven metadata.
    /// Tries BMCLAPI mirror first, falls back to official Maven if mirror data is incomplete.
    pub fn get_available_versions(&self, mc_version: &str) -> AppResult<Vec<String>> {
        let prefix = format!("{}-", mc_version);

        // Try BMCLAPI mirror first
        let versions = Self::fetch_versions_from_url(FORGE_MAVEN_META, &prefix, true);
        if let Ok(ref v) = versions {
            if !v.is_empty() {
                return Ok(v.clone());
            }
        }

        // Fall back to official Maven URL (bypass mirror)
        crate::warn_msg!(
            "Forge: no {} versions in BMCLAPI mirror, trying official Maven...",
            mc_version
        );
        Self::fetch_versions_from_url(FORGE_MAVEN_META, &prefix, false)
    }

    fn fetch_versions_from_url(
        url: &str,
        prefix: &str,
        use_mirror: bool,
    ) -> AppResult<Vec<String>> {
        let result = if use_mirror {
            http::http_get(url)
        } else {
            http::http_get_no_mirror(url)
        };

        let (status, body) = result.map_err(|e| {
            AppError::Loader(format!("Forge maven metadata fetch failed: {}", e))
        })?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "Forge maven metadata fetch failed ({})",
                status
            )));
        }

        let text = String::from_utf8_lossy(&body);

        let metadata: Metadata = quick_xml::de::from_str(&text).map_err(|e| {
            AppError::Xml(format!("Failed to parse Forge maven metadata: {}", e))
        })?;

        let mut versions: Vec<String> = metadata
            .versioning
            .versions
            .version
            .into_iter()
            .filter(|v| v.starts_with(prefix))
            .map(|v| v[prefix.len()..].to_string())
            .collect();

        // Sort by numeric components
        let num_re = regex::Regex::new(r"(\d+)")?;
        versions.sort_by(|a, b| {
            let nums_a: Vec<u32> = num_re
                .find_iter(a)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            let nums_b: Vec<u32> = num_re
                .find_iter(b)
                .filter_map(|m| m.as_str().parse().ok())
                .collect();
            nums_a.cmp(&nums_b)
        });

        Ok(versions)
    }

    /// Get recommended Forge version from promotions.
    pub fn get_recommended_version(&self, mc_version: &str) -> Option<String> {
        let (status, body) =
            http::http_get(FORGE_PROMOTIONS).unwrap_or_else(|_| (0, Vec::new()));
        if status != 200 {
            return None;
        }
        let data: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let promos = data["promos"].as_object()?;
        promos
            .get(&format!("{}-recommended", mc_version))
            .or_else(|| promos.get(&format!("{}-latest", mc_version)))
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    /// Build the Forge installer URL.
    fn installer_url(&self, mc_version: &str, loader_version: &str) -> String {
        format!(
            "{}/{}-{}/forge-{}-{}-installer.jar",
            FORGE_MAVEN, mc_version, loader_version, mc_version, loader_version
        )
    }

    /// Ensure base game files exist.
    fn ensure_base_game(&self, mc_version: &str) -> AppResult<()> {
        let vm = VersionManager::new(&self.game_dir);
        let (version_id, version_data) = vm.get_version_info(Some(mc_version))?;
        vm.download_client_jar(&version_id, &version_data)?;
        let profile_path = self.game_dir.join("launcher_profiles.json");
        if !profile_path.exists() {
            std::fs::write(&profile_path, "{}").ok();
        }
        Ok(())
    }

    /// Extract a file from a ZIP/JAR archive into a byte vector.
    fn extract_zip_entry(zip_path: &Path, entry_name: &str) -> AppResult<Vec<u8>> {
        let file = std::fs::File::open(zip_path).map_err(|e| {
            AppError::Loader(format!("Cannot open installer: {}", e))
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            AppError::Loader(format!("Cannot read installer zip: {}", e))
        })?;
        let mut entry = archive.by_name(entry_name).map_err(|_| {
            AppError::Loader(format!(
                "Entry '{}' not found in installer",
                entry_name
            ))
        })?;
        let mut buf = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut buf).map_err(|e| {
            AppError::Loader(format!("Failed to read '{}': {}", entry_name, e))
        })?;
        Ok(buf)
    }

    /// Extract a file from the installer JAR to a destination path.
    fn extract_from_installer(
        installer_path: &Path,
        entry_name: &str,
        dest: &Path,
    ) -> AppResult<()> {
        if dest.exists() {
            return Ok(());
        }
        let data = Self::extract_zip_entry(installer_path, entry_name)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(dest, &data).map_err(|e| {
            AppError::Loader(format!("Cannot write {}: {}", dest.display(), e))
        })?;
        Ok(())
    }

    /// Install Forge loader — manual approach: extract version.json from the
    /// installer JAR, download all libraries via BMCLAPI mirror, no Java needed.
    pub fn install(
        &self,
        mc_version: &str,
        loader_version_id: Option<&str>,
    ) -> AppResult<(String, serde_json::Value)> {
        let versions = self.get_available_versions(mc_version)?;
        if versions.is_empty() {
            return Err(AppError::Loader(format!(
                "No Forge loader found for Minecraft {}",
                mc_version
            )));
        }

        let loader_ver = if let Some(lv) = loader_version_id {
            if !versions.contains(&lv.to_string()) {
                return Err(AppError::Loader(format!(
                    "Forge loader version '{}' not found for MC {}",
                    lv, mc_version
                )));
            }
            lv.to_string()
        } else {
            let rec = self.get_recommended_version(mc_version);
            match rec {
                Some(r) if versions.contains(&r) => r,
                _ => versions
                    .last()
                    .ok_or_else(|| {
                        AppError::Loader("No Forge versions available".into())
                    })?
                    .clone(),
            }
        };

        let full_ver = format!("{}-{}", mc_version, loader_ver);
        crate::info!(
            "Installing Forge {} for Minecraft {}...",
            loader_ver,
            mc_version
        );
        self.ensure_base_game(mc_version)?;

        // ── Download the Forge installer JAR ──
        let installer_url = self.installer_url(mc_version, &loader_ver);
        let installer_path = self.lib_dir.join("forge").join(format!(
            "forge-{}-installer.jar",
            full_ver
        ));
        if let Some(parent) = installer_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        http::download_file(
            &installer_url,
            &installer_path,
            &format!("Forge {} installer", loader_ver),
            None,
            3,
            true,
        )
        .map_err(|e| AppError::Loader(format!("Forge installer download failed: {}", e)))?;

        // ── Extract version.json from the installer ──
        let version_json_bytes =
            Self::extract_zip_entry(&installer_path, "version.json")?;
        let version_data: serde_json::Value =
            serde_json::from_slice(&version_json_bytes).map_err(|e| {
                AppError::Loader(format!(
                    "Failed to parse version.json from installer: {}",
                    e
                ))
            })?;

        let installed_id = version_data["id"]
            .as_str()
            .unwrap_or(&format!("{}-forge-{}", mc_version, loader_ver))
            .to_string();

        // ── Download all Forge libraries ──
        let libs = version_data["libraries"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        crate::info!(
            "Forge: {} libraries to download",
            libs.len()
        );

        let mut failed: Vec<String> = Vec::new();

        for lib in &libs {
            let name = lib["name"].as_str().unwrap_or("");
            let url = lib["downloads"]["artifact"]["url"]
                .as_str()
                .unwrap_or("");

            if name.is_empty() {
                continue;
            }

            let parts: Vec<&str> = name.split(':').collect();
            if parts.len() < 3 {
                continue;
            }
            let (group, artifact, version) = (parts[0], parts[1], parts[2]);
            let classifier = parts.get(3).copied().unwrap_or("");

            let group_path = group.replace('.', "/");
            let jar_name = if classifier.is_empty() {
                format!("{}-{}.jar", artifact, version)
            } else {
                format!("{}-{}-{}.jar", artifact, version, classifier)
            };
            let lib_dir = self
                .lib_dir
                .join(&group_path)
                .join(artifact)
                .join(version);
            let jar_path = lib_dir.join(&jar_name);

            if jar_path.exists() {
                continue;
            }

            if !url.is_empty() {
                // Download from the provided URL (mirrored automatically)
                if let Err(e) = http::download_file(
                    url,
                    &jar_path,
                    &format!("{}:{}", artifact, version),
                    None,
                    2,
                    false,
                ) {
                    failed.push(format!("{}: {}", name, e));
                }
            } else {
                // Empty URL — this is the Forge universal JAR embedded in the installer
                // It lives at maven/net/minecraftforge/forge/{ver}/forge-{ver}.jar
                let maven_entry = format!(
                    "maven/net/minecraftforge/forge/{}/forge-{}.jar",
                    full_ver, full_ver
                );
                if let Err(e) = Self::extract_from_installer(
                    &installer_path,
                    &maven_entry,
                    &jar_path,
                ) {
                    // Fallback: try downloading from Maven
                    let fallback_url = format!(
                        "https://maven.minecraftforge.net/net/minecraftforge/forge/{}/forge-{}.jar",
                        full_ver, full_ver
                    );
                    if let Err(e2) = http::download_file(
                        &fallback_url,
                        &jar_path,
                        &format!("{}:{}", artifact, version),
                        None,
                        2,
                        false,
                    ) {
                        failed.push(format!("{}: extract({}), download({})", name, e, e2));
                    }
                }
            }
        }

        if !failed.is_empty() {
            crate::warn_msg!(
                "Failed to download {} Forge libraries:",
                failed.len()
            );
            for f in &failed {
                crate::warn_msg!("  {}", f);
            }
        }

        // ── Save version.json ──
        let version_dir = self.game_dir.join("versions").join(&installed_id);
        std::fs::create_dir_all(&version_dir).ok();
        let version_json_path = version_dir.join(format!("{}.json", installed_id));
        std::fs::write(
            &version_json_path,
            serde_json::to_string_pretty(&version_data).unwrap_or_default(),
        )
        .map_err(|e| {
            AppError::Loader(format!("Cannot write version.json: {}", e))
        })?;

        // ── Build loader profile ──
        let profile =
            self.build_profile(mc_version, &installed_id, &version_json_path)?;

        let profile_path = self
            .lib_dir
            .join("forge")
            .join(format!("forge-profile-{}.json", mc_version));
        if let Some(parent) = profile_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(
            &profile_path,
            serde_json::to_string_pretty(&profile).unwrap_or_default(),
        )
        .ok();

        crate::success!(
            "Forge {} installed for Minecraft {}",
            loader_ver,
            mc_version
        );
        Ok((installed_id, profile))
    }

    fn build_profile(
        &self,
        mc_version: &str,
        version_id: &str,
        version_json_path: &Path,
    ) -> AppResult<serde_json::Value> {
        let version_data: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(version_json_path)?)?;
        let merged = self.resolve_inherits(
            &version_data,
            &self.game_dir.join("versions"),
        )?;

        Ok(serde_json::json!({
            "loader": "forge",
            "mc_version": mc_version,
            "version_id": version_id,
            "mainClass": merged["mainClass"],
            "libraries": merged["libraries"],
            "arguments": merged["arguments"],
        }))
    }

    fn resolve_inherits(
        &self,
        version_data: &serde_json::Value,
        versions_dir: &Path,
    ) -> AppResult<serde_json::Value> {
        let parent_id = version_data["inheritsFrom"].as_str();
        if parent_id.is_none() {
            return Ok(version_data.clone());
        }
        let parent_id = parent_id.unwrap();
        let parent_path = versions_dir
            .join(parent_id)
            .join(format!("{}.json", parent_id));

        if !parent_path.exists() {
            crate::warn_msg!(
                "Parent version {} not found; Forge may not launch.",
                parent_id
            );
            return Ok(version_data.clone());
        }

        let parent: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&parent_path)?)?;
        let merged_parent = self.resolve_inherits(&parent, versions_dir)?;

        let mut merged = merged_parent.clone();
        if let Some(obj) = version_data.as_object() {
            if let Some(merged_obj) = merged.as_object_mut() {
                for (k, v) in obj {
                    if k != "libraries" {
                        merged_obj.insert(k.clone(), v.clone());
                    }
                }
            }
        }

        // Merge libraries
        let parent_libs = merged_parent["libraries"].as_array().cloned();
        let child_libs = version_data["libraries"].as_array().cloned();
        let mut all_libs: Vec<serde_json::Value> = parent_libs.unwrap_or_default();
        if let Some(child) = child_libs {
            all_libs.extend(child);
        }
        merged["libraries"] = serde_json::Value::Array(all_libs);

        Ok(merged)
    }
}