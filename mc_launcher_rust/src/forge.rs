//! Forge loader installation.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::error::{AppError, AppResult};
use crate::http;
use crate::java;
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
    pub fn get_available_versions(&self, mc_version: &str) -> AppResult<Vec<String>> {
        let (status, body) = http::http_get(FORGE_MAVEN_META).map_err(|e| {
            AppError::Loader(format!("Forge maven metadata fetch failed: {}", e))
        })?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "Forge maven metadata fetch failed ({})",
                status
            )));
        }

        let text = String::from_utf8_lossy(&body);
        let prefix = format!("{}-", mc_version);

        let metadata: Metadata = quick_xml::de::from_str(&text).map_err(|e| {
            AppError::Xml(format!("Failed to parse Forge maven metadata: {}", e))
        })?;

        let mut versions: Vec<String> = metadata
            .versioning
            .versions
            .version
            .into_iter()
            .filter(|v| v.starts_with(&prefix))
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

    /// Install Forge loader.
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

        crate::info!(
            "Installing Forge {} for Minecraft {}...",
            loader_ver,
            mc_version
        );
        self.ensure_base_game(mc_version)?;

        let installer_url = self.installer_url(mc_version, &loader_ver);
        let installer_path = self.lib_dir.join("forge").join(format!(
            "forge-{}-{}-installer.jar",
            mc_version, loader_ver
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

        let java_path = java::check_java(None).ok_or_else(|| {
            AppError::Loader("Java not found. Cannot run Forge installer.".into())
        })?;

        crate::info!("Running Forge installer (this may take a while)...");
        let status = Command::new(&java_path)
            .arg("-jar")
            .arg(&installer_path)
            .arg("--installClient")
            .arg(&self.game_dir)
            .status()
            .map_err(|e| {
                AppError::Loader(format!("Failed to run Forge installer: {}", e))
            })?;

        if !status.success() {
            return Err(AppError::Loader(format!(
                "Forge installer failed (exit {})",
                status.code().unwrap_or(-1)
            )));
        }

        // Find the installed version JSON
        let installed_id = format!("{}-forge-{}", mc_version, loader_ver);
        let mut version_json_path = self
            .game_dir
            .join("versions")
            .join(&installed_id)
            .join(format!("{}.json", installed_id));

        if !version_json_path.exists() {
            if let Ok(candidates) = std::fs::read_dir(
                self.game_dir.join("versions"),
            ) {
                let mut matches: Vec<_> = candidates
                    .flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with(&format!("{}-forge-", mc_version))
                    })
                    .collect();
                matches.sort_by_key(|e| e.file_name());
                if let Some(last) = matches.last() {
                    let id = last.file_name().to_string_lossy().to_string();
                    version_json_path =
                        last.path().join(format!("{}.json", id));
                }
            }
        }

        if !version_json_path.exists() {
            return Err(AppError::Loader(
                "Forge installer finished but version.json was not found.".into(),
            ));
        }

        let installed_id = version_json_path
            .parent()
            .ok_or_else(|| {
                AppError::Loader("Version JSON path has no parent directory".into())
            })?
            .file_name()
            .ok_or_else(|| {
                AppError::Loader("Version JSON path has no file name".into())
            })?
            .to_string_lossy()
            .to_string();

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