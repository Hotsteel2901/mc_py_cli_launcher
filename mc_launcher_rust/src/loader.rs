//! Shared loader infrastructure for Fabric/Forge/NeoForge.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::AppError;
use crate::http;
use crate::java;
use crate::version::VersionManager;

/// Trait for mod loader managers (Fabric, Forge, NeoForge).
pub trait LoaderManager {
    /// The game directory (e.g. ~/.minecraft).
    fn game_dir(&self) -> &Path;

    /// The libraries directory (e.g. ~/.minecraft/libraries).
    fn lib_dir(&self) -> &Path;

    /// Human-readable loader name (e.g. "forge", "neoforge", "fabric").
    fn loader_name(&self) -> &'static str;

    /// Get available loader versions for a given Minecraft version.
    fn get_available_versions(&self, mc_version: &str) -> Vec<String>;

    /// Build the installer download URL for a specific MC + loader version.
    fn installer_url(&self, mc_version: &str, loader_version: &str) -> String;

    /// Install the loader with a default implementation that:
    ///   1. Gets available versions, validates
    ///   2. Calls ensure_base_game
    ///   3. Downloads installer via installer_url
    ///   4. Runs Java installer
    ///   5. Finds the installed version JSON
    ///   6. Builds profile and saves it to lib_dir/{loader}/{loader}-profile-{mc_version}.json
    fn install(
        &self,
        mc_version: &str,
        loader_version: Option<&str>,
    ) -> Result<(String, serde_json::Value), AppError> {
        let loader_name = self.loader_name();

        // 1. Get available versions, validate
        let versions = self.get_available_versions(mc_version);
        if versions.is_empty() {
            return Err(AppError::Loader(format!(
                "No {} loader found for Minecraft {}",
                loader_name, mc_version
            )));
        }

        let loader_ver = if let Some(lv) = loader_version {
            if !versions.contains(&lv.to_string()) {
                return Err(AppError::Loader(format!(
                    "{} loader version '{}' not found for MC {}",
                    loader_name, lv, mc_version
                )));
            }
            lv.to_string()
        } else {
            versions.last().cloned().unwrap()
        };

        crate::info!(
            "Installing {} {} for Minecraft {}...",
            loader_name,
            loader_ver,
            mc_version
        );

        // 2. Ensure base game files exist
        ensure_base_game(self.game_dir(), mc_version);

        // 3. Download installer
        let url = self.installer_url(mc_version, &loader_ver);
        let filename = url.rsplit('/').next().unwrap_or("installer.jar");
        let installer_path = self.lib_dir().join(loader_name).join(filename);

        // 4. Run Java installer
        run_installer(
            self.game_dir(),
            self.lib_dir(),
            &url,
            &installer_path,
            loader_name,
        )?;

        // 5. Find the installed version JSON
        // Try forge-style prefix first (e.g. "1.20.1-forge-"), then
        // neoforge-style prefix (e.g. "neoforge-")
        let prefix = format!("{}-{}-", mc_version, loader_name);
        let result = find_installed_version(self.game_dir(), &prefix);
        let (installed_id, version_json_path) = match result {
            Ok(v) => v,
            Err(_) => {
                let prefix2 = format!("{}-", loader_name);
                find_installed_version(self.game_dir(), &prefix2)?
            }
        };

        // 6. Build profile and save
        let profile = build_profile(mc_version, &installed_id, &version_json_path, loader_name)?;

        let profile_path = self
            .lib_dir()
            .join(loader_name)
            .join(format!("{}-profile-{}.json", loader_name, mc_version));
        if let Some(parent) = profile_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(
            &profile_path,
            serde_json::to_string_pretty(&profile).unwrap_or_default(),
        )
        .map_err(|e| AppError::Io(e))?;

        crate::success!(
            "{} {} installed for Minecraft {}",
            loader_name,
            loader_ver,
            mc_version
        );

        Ok((installed_id, profile))
    }
}

// ─── Shared functions ──────────────────────────────────────────────

/// Ensure base game files exist for the given Minecraft version.
/// Downloads the client jar and creates launcher_profiles.json if missing.
pub fn ensure_base_game(game_dir: &Path, mc_version: &str) {
    let vm = VersionManager::new(game_dir);
    let (version_id, version_data) = vm.get_version_info(Some(mc_version));
    vm.download_client_jar(&version_id, &version_data);
    let profile_path = game_dir.join("launcher_profiles.json");
    if !profile_path.exists() {
        std::fs::write(&profile_path, "{}").ok();
    }
}

/// Download and run a Java-based loader installer (e.g. Forge / NeoForge).
pub fn run_installer(
    game_dir: &Path,
    _lib_dir: &Path,
    installer_url: &str,
    installer_path: &Path,
    loader_name: &str,
) -> Result<(), AppError> {
    if let Some(parent) = installer_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    http::download_file(
        installer_url,
        installer_path,
        &format!("{} installer", loader_name),
        None,
        3,
        true,
    );

    let java_path = java::check_java(None).ok_or_else(|| {
        AppError::JavaNotFound(format!(
            "Java not found. Cannot run {} installer.",
            loader_name
        ))
    })?;

    crate::info!("Running {} installer (this may take a while)...", loader_name);
    let status = Command::new(&java_path)
        .arg("-jar")
        .arg(installer_path)
        .arg("--installClient")
        .arg(game_dir)
        .status()
        .map_err(|e| {
            AppError::Loader(format!(
                "Failed to run {} installer: {}",
                loader_name, e
            ))
        })?;

    if !status.success() {
        return Err(AppError::Loader(format!(
            "{} installer failed (exit {})",
            loader_name,
            status.code().unwrap_or(-1)
        )));
    }
    Ok(())
}

/// Find the installed version JSON after running the loader installer.
/// Searches `game_dir/versions/` for a directory whose name starts with `prefix`.
/// Returns `(version_id, path_to_version_json)`.
pub fn find_installed_version(
    game_dir: &Path,
    prefix: &str,
) -> Result<(String, PathBuf), AppError> {
    let versions_dir = game_dir.join("versions");

    // Collect matching directories
    let mut matches: Vec<_> = std::fs::read_dir(&versions_dir)
        .map_err(|e| AppError::Io(e))?
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(prefix)
        })
        .collect();

    matches.sort_by_key(|e| e.file_name());

    if let Some(last) = matches.last() {
        let id = last.file_name().to_string_lossy().to_string();
        let json_path = last.path().join(format!("{}.json", id));
        if json_path.exists() {
            return Ok((id, json_path));
        }
    }

    Err(AppError::Loader(format!(
        "Installer finished but version.json was not found (prefix: {}).",
        prefix
    )))
}

/// Build a loader profile JSON from the installed version data.
pub fn build_profile(
    mc_version: &str,
    version_id: &str,
    version_json_path: &Path,
    loader_name: &str,
) -> Result<serde_json::Value, AppError> {
    let version_data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(version_json_path)?)?;
    let versions_dir = version_json_path
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));
    let merged = resolve_inherits(&version_data, versions_dir);

    Ok(serde_json::json!({
        "loader": loader_name,
        "mc_version": mc_version,
        "version_id": version_id,
        "mainClass": merged["mainClass"],
        "libraries": merged["libraries"],
        "arguments": merged["arguments"],
    }))
}

/// Resolve parent version inheritance by merging `version_data` with its
/// `inheritsFrom` chain (recursively).
pub fn resolve_inherits(
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
            "Parent version {} not found; the loader may not launch.",
            parent_id
        );
        return version_data.clone();
    }

    let parent: serde_json::Value =
        match std::fs::read_to_string(&parent_path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(_) => return version_data.clone(),
            },
            Err(_) => return version_data.clone(),
        };
    let merged_parent = resolve_inherits(&parent, versions_dir);

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

    merged
}