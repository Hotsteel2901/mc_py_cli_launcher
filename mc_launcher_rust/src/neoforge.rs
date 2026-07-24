//! NeoForge loader installation.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::http;
use crate::java;
use crate::version::VersionManager;

const NEOFORGE_MAVEN: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge";
const NEOFORGE_MAVEN_META: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";

pub struct NeoForgeManager {
    pub game_dir: PathBuf,
    pub lib_dir: PathBuf,
}

impl NeoForgeManager {
    pub fn new(game_dir: &Path) -> Self {
        NeoForgeManager {
            game_dir: game_dir.to_path_buf(),
            lib_dir: game_dir.join("libraries"),
        }
    }

    /// Map MC version to NeoForge Maven version prefix.
    fn neoforge_prefix_for_mc(mc_version: &str) -> Option<String> {
        let parts: Vec<&str> = mc_version.split('.').collect();
        if parts.len() >= 3 && parts[0] == "1" && parts[1] == "20" {
            if parts[2] == "1" {
                // 1.20.1 used net.neoforged:forge, not neoforge
                return None;
            }
            return Some(format!("20.{}", parts[2]));
        }
        if parts.len() >= 2 && parts[0] == "1" {
            if let Ok(minor) = parts[1].parse::<u32>() {
                if minor >= 21 {
                    let patch = parts.get(2).copied().unwrap_or("0");
                    return Some(format!("{}.{}", minor, patch));
                }
            }
        }
        None
    }

    /// Get available NeoForge versions for a Minecraft version.
    pub fn get_available_versions(&self, mc_version: &str) -> Vec<String> {
        let (status, body) =
            http::http_get(NEOFORGE_MAVEN_META).unwrap_or_else(|e| {
                crate::die!(format!("NeoForge maven metadata fetch failed: {}", e));
            });
        if status != 200 {
            crate::die!(format!(
                "NeoForge maven metadata fetch failed ({})",
                status
            ));
        }

        let prefix = NeoForgeManager::neoforge_prefix_for_mc(mc_version);
        let prefix = match prefix {
            Some(p) => p,
            None => {
                crate::die!(format!(
                    "Cannot determine NeoForge versions for Minecraft {}. Specify --loader-version.",
                    mc_version
                ));
            }
        };

        let text = String::from_utf8_lossy(&body);
        let version_re =
            regex::Regex::new(r"<version>([^<]+)</version>").unwrap();

        let mut versions: Vec<String> = version_re
            .captures_iter(&text)
            .filter_map(|c| c.get(1))
            .map(|m| m.as_str().to_string())
            .filter(|v| {
                if v.starts_with(&prefix) {
                    let rest = &v[prefix.len()..];
                    // Must be followed by digit or period-digit for proper prefix match
                    if prefix.ends_with('.') || prefix.ends_with('-') {
                        rest.chars().next().map_or(false, |c| c.is_ascii_digit())
                    } else {
                        rest.starts_with('.')
                            && rest.len() > 1
                            && rest.chars().nth(1).map_or(false, |c| {
                                c.is_ascii_digit()
                            })
                    }
                } else {
                    false
                }
            })
            .collect();

        versions.sort_by(|a, b| {
            let nums_a: Vec<u32> =
                regex::Regex::new(r"(\d+)")
                    .unwrap()
                    .find_iter(a)
                    .filter_map(|m| m.as_str().parse().ok())
                    .collect();
            let nums_b: Vec<u32> =
                regex::Regex::new(r"(\d+)")
                    .unwrap()
                    .find_iter(b)
                    .filter_map(|m| m.as_str().parse().ok())
                    .collect();
            nums_a.cmp(&nums_b)
        });

        versions
    }

    fn installer_url(&self, loader_version: &str) -> String {
        format!(
            "{}/{}/neoforge-{}-installer.jar",
            NEOFORGE_MAVEN, loader_version, loader_version
        )
    }

    fn ensure_base_game(&self, mc_version: &str) {
        let vm = VersionManager::new(&self.game_dir);
        let (version_id, version_data) = vm.get_version_info(Some(mc_version));
        vm.download_client_jar(&version_id, &version_data);
        let profile_path = self.game_dir.join("launcher_profiles.json");
        if !profile_path.exists() {
            std::fs::write(&profile_path, "{}").ok();
        }
    }

    /// Install NeoForge loader.
    pub fn install(
        &self,
        mc_version: &str,
        loader_version_id: Option<&str>,
    ) -> (String, serde_json::Value) {
        let versions = self.get_available_versions(mc_version);
        if versions.is_empty() {
            crate::die!(format!(
                "No NeoForge loader found for Minecraft {}",
                mc_version
            ));
        }

        let loader_ver = if let Some(lv) = loader_version_id {
            if !versions.contains(&lv.to_string()) {
                crate::die!(format!(
                    "NeoForge loader version '{}' not found.",
                    lv
                ));
            }
            lv.to_string()
        } else {
            versions.last().unwrap().clone()
        };

        crate::info!(
            "Installing NeoForge {} for Minecraft {}...",
            loader_ver,
            mc_version
        );
        self.ensure_base_game(mc_version);

        let installer_url = self.installer_url(&loader_ver);
        let installer_path = self.lib_dir.join("neoforge").join(format!(
            "neoforge-{}-installer.jar",
            loader_ver
        ));
        if let Some(parent) = installer_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        http::download_file(
            &installer_url,
            &installer_path,
            &format!("NeoForge {} installer", loader_ver),
            None,
            3,
            true,
        );

        let java_path = java::check_java(None).unwrap_or_else(|| {
            crate::die!("Java not found. Cannot run NeoForge installer.");
        });

        crate::info!("Running NeoForge installer (this may take a while)...");
        let status = Command::new(&java_path)
            .arg("-jar")
            .arg(&installer_path)
            .arg("--installClient")
            .arg(&self.game_dir)
            .status()
            .unwrap_or_else(|e| {
                crate::die!(format!("Failed to run NeoForge installer: {}", e));
            });

        if !status.success() {
            crate::die!(format!(
                "NeoForge installer failed (exit {})",
                status.code().unwrap_or(-1)
            ));
        }

        // Find the installed version JSON
        let installed_id = format!("neoforge-{}", loader_ver);
        let mut version_json_path = self
            .game_dir
            .join("versions")
            .join(&installed_id)
            .join(format!("{}.json", installed_id));

        if !version_json_path.exists() {
            if let Ok(candidates) =
                std::fs::read_dir(self.game_dir.join("versions"))
            {
                let mut matches: Vec<_> = candidates
                    .flatten()
                    .filter(|e| {
                        e.file_name()
                            .to_string_lossy()
                            .starts_with("neoforge-")
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
            crate::die!(
                "NeoForge installer finished but version.json was not found."
            );
        }

        let installed_id = version_json_path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let profile = self.build_profile(
            mc_version,
            &installed_id,
            &version_json_path,
        );

        let profile_path = self
            .lib_dir
            .join("neoforge")
            .join(format!("neoforge-profile-{}.json", mc_version));
        if let Some(parent) = profile_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(
            &profile_path,
            serde_json::to_string_pretty(&profile).unwrap_or_default(),
        )
        .ok();

        crate::success!(
            "NeoForge {} installed for Minecraft {}",
            loader_ver,
            mc_version
        );
        (installed_id, profile)
    }

    fn build_profile(
        &self,
        mc_version: &str,
        version_id: &str,
        version_json_path: &Path,
    ) -> serde_json::Value {
        let version_data: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(version_json_path).unwrap(),
        )
        .unwrap();
        let merged = self.resolve_inherits(
            &version_data,
            &self.game_dir.join("versions"),
        );

        serde_json::json!({
            "loader": "neoforge",
            "mc_version": mc_version,
            "version_id": version_id,
            "mainClass": merged["mainClass"],
            "libraries": merged["libraries"],
            "arguments": merged["arguments"],
        })
    }

    fn resolve_inherits(
        &self,
        version_data: &serde_json::Value,
        versions_dir: &Path,
    ) -> serde_json::Value {
        let parent_id = version_data["inheritsFrom"].as_str();
        if parent_id.is_none() {
            return version_data.clone();
        }
        let parent_id = parent_id.unwrap();
        let parent_path = versions_dir
            .join(parent_id)
            .join(format!("{}.json", parent_id));

        if !parent_path.exists() {
            crate::warn_msg!(
                "Parent version {} not found; NeoForge may not launch.",
                parent_id
            );
            return version_data.clone();
        }

        let parent: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&parent_path).unwrap(),
        )
        .unwrap();
        let merged_parent = self.resolve_inherits(&parent, versions_dir);

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

        let parent_libs = merged_parent["libraries"].as_array().cloned();
        let child_libs = version_data["libraries"].as_array().cloned();
        let mut all_libs: Vec<serde_json::Value> = parent_libs.unwrap_or_default();
        if let Some(child) = child_libs {
            all_libs.extend(child);
        }
        merged["libraries"] = serde_json::Value::Array(all_libs);

        merged
    }
}
