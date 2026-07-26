//! Fabric loader installation.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::http;

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";

pub struct FabricManager {
    #[allow(dead_code)]
    pub game_dir: PathBuf,
    pub lib_dir: PathBuf,
}

impl FabricManager {
    pub fn new(game_dir: &Path) -> Self {
        FabricManager {
            game_dir: game_dir.to_path_buf(),
            lib_dir: game_dir.join("libraries"),
        }
    }

    /// Get available Fabric loader versions for a Minecraft version.
    pub fn get_available_versions(
        &self,
        mc_version: Option<&str>,
    ) -> AppResult<serde_json::Value> {
        let url = if let Some(mcv) = mc_version {
            format!("{}/versions/loader/{}", FABRIC_META, mcv)
        } else {
            format!("{}/versions/loader", FABRIC_META)
        };

        let (status, body) = http::http_get(&url)
            .map_err(|e| AppError::Http(format!("Fabric Meta API error: {}", e)))?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "Fabric Meta API returned {}",
                status
            )));
        }
        Ok(serde_json::from_slice(&body)?)
    }

    /// Fetch the Fabric profile JSON.
    fn fetch_profile(
        &self,
        mc_version: &str,
        loader_version: &str,
    ) -> AppResult<serde_json::Value> {
        let url = format!(
            "{}/versions/loader/{}/{}/profile/json",
            FABRIC_META, mc_version, loader_version
        );
        let (status, body) = http::http_get(&url)
            .map_err(|e| AppError::Http(format!("Fabric profile fetch error: {}", e)))?;
        if status != 200 {
            return Err(AppError::Loader(format!(
                "Fabric profile fetch failed ({}) for {}/{}",
                status, mc_version, loader_version
            )));
        }
        Ok(serde_json::from_slice(&body)?)
    }

    /// Convert a Maven name to a relative path.
    fn maven_path(name: &str) -> String {
        let parts: Vec<&str> = name.split(':').collect();
        let (g, a, v) = (parts[0], parts[1], parts[2]);
        format!("{}/{}/{}/{}-{}.jar", g.replace('.', "/"), a, v, a, v)
    }

    /// Install Fabric loader.
    pub fn install(
        &self,
        mc_version: &str,
        loader_version_id: Option<&str>,
    ) -> AppResult<(Vec<PathBuf>, serde_json::Value)> {
        let versions = self.get_available_versions(Some(mc_version))?;
        let arr = versions.as_array();

        if arr.is_none_or(|a| a.is_empty()) {
            return Err(AppError::Loader(format!(
                "No Fabric loader found for Minecraft {}",
                mc_version
            )));
        }
        let arr = arr.unwrap();

        let target = if let Some(lv) = loader_version_id {
            arr.iter()
                .find(|v| v["loader"]["version"].as_str() == Some(lv))
                .cloned()
                .ok_or_else(|| {
                    AppError::Loader(format!(
                        "Fabric loader version '{}' not found for MC {}",
                        lv, mc_version
                    ))
                })?
        } else {
            arr[0].clone()
        };

        let loader_ver = target["loader"]["version"]
            .as_str()
            .unwrap_or("")
            .to_string();
        crate::info!(
            "Installing Fabric Loader {} for Minecraft {}...",
            loader_ver,
            mc_version
        );

        let profile = self.fetch_profile(mc_version, &loader_ver)?;
        let libraries = profile["libraries"].as_array();
        let mut all_jars = Vec::new();

        if let Some(libs) = libraries {
            crate::info!("Profile: {} libraries to download", libs.len());

            for lib in libs {
                let name = lib["name"].as_str().unwrap_or("");
                let url_base = lib["url"]
                    .as_str()
                    .unwrap_or("https://maven.fabricmc.net/");
                let rel_path = Self::maven_path(name);
                let jar_path = self.lib_dir.join(&rel_path);

                if !jar_path.exists() {
                    let full_url =
                        format!("{}/{}", url_base.trim_end_matches('/'), rel_path);
                    let label = name.split(':').nth(1).unwrap_or("unknown");
                    if let Err(e) = http::download_file(
                        &full_url,
                        &jar_path,
                        label,
                        None,
                        3,
                        false,
                    ) {
                        return Err(AppError::Http(format!(
                            "Failed to download Fabric library {}: {}",
                            label, e
                        )));
                    }
                }
                all_jars.push(jar_path);
            }
        }

        crate::success!(
            "Fabric {} installed ({} jars)",
            loader_ver,
            all_jars.len()
        );

        // Save profile
        let profile_path = self
            .lib_dir
            .join("fabric")
            .join(format!("fabric-profile-{}.json", mc_version));
        if let Some(parent) = profile_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(
            &profile_path,
            serde_json::to_string_pretty(&profile).unwrap_or_default(),
        )
        .ok();

        Ok((all_jars, profile))
    }
}