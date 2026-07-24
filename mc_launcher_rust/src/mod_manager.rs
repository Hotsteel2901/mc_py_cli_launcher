//! Mod management — search, install, list, disable, enable, uninstall mods.

use std::path::{Path, PathBuf};

use crate::modrinth;

pub struct ModManager {
    pub game_dir: PathBuf,
}

impl ModManager {
    pub fn new(game_dir: &Path) -> Self {
        ModManager {
            game_dir: game_dir.to_path_buf(),
        }
    }

    /// Get the mods directory for a Minecraft version.
    pub fn mods_dir(&self, mc_version: &str) -> PathBuf {
        let d = self
            .game_dir
            .join("versions")
            .join(mc_version)
            .join("mods");
        std::fs::create_dir_all(&d).ok();
        d
    }

    /// List installed Minecraft versions.
    pub fn list_installed_versions(game_dir: &Path) -> Vec<String> {
        let versions_dir = game_dir.join("versions");
        if !versions_dir.exists() {
            return Vec::new();
        }
        let mut vers: Vec<String> = std::fs::read_dir(&versions_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                e.path().join(format!("{}.json", name)).exists()
            })
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();

        // Sort by version number
        vers.sort_by(|a, b| {
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
        vers
    }

    /// Search for mods on Modrinth.
    pub fn search(
        &self,
        query: &str,
        limit: u32,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let mut facets: Vec<serde_json::Value> =
            vec![serde_json::json!(["project_type:mod"])];

        if let Some(l) = loader {
            facets.push(serde_json::json!([format!("categories:{}", l.to_lowercase())]));
        }
        if let Some(gv) = game_version {
            facets.push(serde_json::json!([format!("versions:{}", gv)]));
        }

        let result = modrinth::search_projects(
            query,
            "relevance",
            limit,
            0,
            Some(&serde_json::json!(facets)),
        );

        let mut hits: Vec<serde_json::Value> = result["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for h in &mut hits {
            h["source"] = serde_json::json!("modrinth");
        }
        hits
    }

    /// Get loader support info for a project.
    pub fn loader_support(
        &self,
        project_id: &str,
    ) -> std::collections::BTreeMap<String, (String, String)> {
        let versions = modrinth::get_project_versions(project_id, None, None);
        Self::summarize_loader_support(&versions)
    }

    /// Summarize loader support from version data.
    /// Returns map: loader -> (highest MC version, mod version).
    pub fn summarize_loader_support(
        versions: &serde_json::Value,
    ) -> std::collections::BTreeMap<String, (String, String)> {
        let mc_re = regex::Regex::new(r"^\d+\.\d+(\.\d+)?$").unwrap();
        let mut support: std::collections::BTreeMap<String, (String, String)> =
            std::collections::BTreeMap::new();

        let arr = versions.as_array();
        if arr.is_none() {
            return support;
        }

        for v in arr.unwrap() {
            let game_versions: Vec<&str> = v["game_versions"]
                .as_array()
                .map(|gv| {
                    gv.iter()
                        .filter_map(|s| s.as_str())
                        .filter(|s| mc_re.is_match(s))
                        .collect()
                })
                .unwrap_or_default();

            let top = game_versions
                .iter()
                .max_by(|a, b| {
                    let na: Vec<u32> = regex::Regex::new(r"\d+")
                        .unwrap()
                        .find_iter(a)
                        .filter_map(|m| m.as_str().parse().ok())
                        .collect();
                    let nb: Vec<u32> = regex::Regex::new(r"\d+")
                        .unwrap()
                        .find_iter(b)
                        .filter_map(|m| m.as_str().parse().ok())
                        .collect();
                    na.cmp(&nb)
                })
                .map(|s| s.to_string());

            if let Some(top_mc) = top {
                let mod_ver = v["version_number"]
                    .as_str()
                    .unwrap_or(v["id"].as_str().unwrap_or("?"))
                    .to_string();

                for loader in v["loaders"].as_array().into_iter().flatten() {
                    let l = loader.as_str().unwrap_or("").to_lowercase();
                    let cur = support.get(&l);
                    let insert = match cur {
                        Some((cur_mc, _)) => {
                            let na: Vec<u32> = regex::Regex::new(r"\d+")
                                .unwrap()
                                .find_iter(&top_mc)
                                .filter_map(|m| m.as_str().parse().ok())
                                .collect();
                            let nb: Vec<u32> = regex::Regex::new(r"\d+")
                                .unwrap()
                                .find_iter(cur_mc)
                                .filter_map(|m| m.as_str().parse().ok())
                                .collect();
                            na > nb
                        }
                        None => true,
                    };
                    if insert {
                        support
                            .insert(l.clone(), (top_mc.clone(), mod_ver.clone()));
                    }
                }
            }
        }

        support
    }

    /// Format loader support for display.
    pub fn format_loader_support(
        support: &std::collections::BTreeMap<String, (String, String)>,
    ) -> String {
        let loaders = ["fabric", "forge", "neoforge"];
        let parts: Vec<String> = loaders
            .iter()
            .map(|l| {
                if let Some((mc, modver)) = support.get(*l) {
                    format!("{} <= MC {} ({})", l, mc, modver)
                } else {
                    format!("{} \u{2014}", l)
                }
            })
            .collect();
        parts.join("  |  ")
    }

    /// Detect installed loaders for a game version.
    fn detect_loaders(&self, mc_version: &str) -> Vec<String> {
        let mut found = Vec::new();
        for loader in &["fabric", "neoforge", "forge"] {
            let p = self
                .game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{}-profile-{}.json", loader, mc_version));
            if p.exists() {
                found.push(loader.to_string());
            }
        }
        found
    }

    /// Pick the appropriate loader.
    fn pick_loader(&self, mc_version: &str, preferred: Option<&str>) -> Option<String> {
        if let Some(p) = preferred {
            return Some(p.to_string());
        }
        let detected = self.detect_loaders(mc_version);
        if detected.is_empty() {
            return None;
        }
        if detected.len() == 1 {
            return Some(detected[0].clone());
        }
        let order = ["fabric", "neoforge", "forge"];
        for loader in &order {
            if detected.contains(&loader.to_string()) {
                crate::info!(
                    "Multiple loaders detected; picked {}. Use --loader to override.",
                    loader
                );
                return Some(loader.to_string());
            }
        }
        Some(detected[0].clone())
    }

    /// Install a mod.
    pub fn install(
        &self,
        slug: &str,
        mc_version: &str,
        loader: Option<&str>,
        version_id: Option<&str>,
    ) -> (Vec<PathBuf>, serde_json::Value, serde_json::Value) {
        crate::info!("Resolving mod: {}...", slug);
        let loader = self.pick_loader(mc_version, loader);
        if let Some(ref l) = loader {
            crate::info!("Using loader: {}", l);
        }

        let project = modrinth::get_project(slug);
        let project_id = project["id"].as_str().unwrap_or(slug);

        let versions = modrinth::get_project_versions(
            project_id,
            loader.as_ref().map(|l| vec![l.clone()]).as_deref(),
            Some(&[mc_version.to_string()]),
        );

        let proj_title = project["title"].as_str().unwrap_or(slug);

        if versions.as_array().map_or(true, |a| a.is_empty()) {
            let extra = format!(
                " for MC {}{}",
                mc_version,
                loader
                    .as_ref()
                    .map(|l| format!(" ({})", l))
                    .unwrap_or_default()
            );
            crate::error_msg!("No versions found for {}{}", proj_title, extra);

            let support = self.loader_support(project_id);
            if !support.is_empty() {
                println!(
                    "\n  Available support for '{}':",
                    proj_title
                );
                println!("    {}", Self::format_loader_support(&support));
                println!();
            }
            if loader.is_none() {
                println!("  Hint: install a loader first, or specify --loader manually");
            }
            std::process::exit(1);
        }

        let mut versions_arr = versions.as_array().unwrap().clone();

        let target = if let Some(vid) = version_id {
            versions_arr
                .iter()
                .find(|v| v["id"].as_str().map_or(false, |id| id == vid))
                .cloned()
                .unwrap_or_else(|| {
                    crate::die!(format!(
                        "Version '{}' not found for {}",
                        vid, proj_title
                    ));
                })
        } else {
            // Sort: loader match first, then by date
            versions_arr.sort_by(|a, b| {
                let a_match = loader.as_ref().map_or(true, |l| {
                    a["loaders"]
                        .as_array()
                        .map_or(false, |arr| {
                            arr.iter().any(|v| {
                                v.as_str().map_or(false, |s| {
                                    s.to_lowercase() == l.to_lowercase()
                                })
                            })
                        })
                });
                let b_match = loader.as_ref().map_or(true, |l| {
                    b["loaders"]
                        .as_array()
                        .map_or(false, |arr| {
                            arr.iter().any(|v| {
                                v.as_str().map_or(false, |s| {
                                    s.to_lowercase() == l.to_lowercase()
                                })
                            })
                        })
                });
                if a_match != b_match {
                    if a_match {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                } else {
                    b["date_published"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(a["date_published"].as_str().unwrap_or(""))
                }
            });
            versions_arr[0].clone()
        };

        let ver_num = target["version_number"]
            .as_str()
            .unwrap_or(target["id"].as_str().unwrap_or("?"));
        let mc_str = target["game_versions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "?".to_string());
        let loaders_str = target["loaders"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| "?".to_string());

        if let Some(ref l) = loader {
            let has_loader = target["loaders"].as_array().map_or(false, |arr| {
                arr.iter().any(|v| {
                    v.as_str()
                        .map_or(false, |s| s.to_lowercase() == l.to_lowercase())
                })
            });
            if !has_loader {
                crate::warn_msg!(
                    "Note: selected version uses loader '{}', not '{}'",
                    loaders_str,
                    l
                );
            }
        }

        crate::info!("Installing {}...", proj_title);
        println!("    Mod version:   {}", ver_num);
        println!("    Game versions: {}", mc_str);
        println!("    Loaders:       {}", loaders_str);

        // Check dependencies
        self.check_dependencies(&target, mc_version, loader.as_deref());

        let dest_dir = self.mods_dir(mc_version);
        let l = if loader.is_some() {
            format!("{} {}", proj_title, ver_num)
        } else {
            proj_title.to_string()
        };
        let paths = modrinth::download_version_files(&target, &dest_dir, &l);
        let total_size: u64 = paths
            .iter()
            .filter_map(|p| p.metadata().ok().map(|m| m.len()))
            .sum();
        crate::success!(
            "Installed {} file(s) ({:.1} KB) -> {}",
            paths.len(),
            total_size as f64 / 1024.0,
            dest_dir.display()
        );

        (paths, target, project)
    }

    /// Check and display mod dependencies.
    fn check_dependencies(
        &self,
        version_data: &serde_json::Value,
        mc_version: &str,
        loader: Option<&str>,
    ) {
        let deps: Vec<&serde_json::Value> = version_data["dependencies"]
            .as_array()
            .map(|arr| arr.iter().filter(|d| d["project_id"].is_string()).collect())
            .unwrap_or_default();

        if deps.is_empty() {
            return;
        }

        let dest_dir = self.mods_dir(mc_version);
        let existing_jars: Vec<String> = std::fs::read_dir(&dest_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name.ends_with(".jar")
                    && !name.ends_with("-sources.jar")
                    && !name.ends_with("-javadoc.jar")
                    && !name.ends_with(".disabled")
            })
            .map(|e| e.file_name().to_string_lossy().to_lowercase())
            .collect();

        let mut required_missing: Vec<(String, String, String, String)> = Vec::new();
        let mut optional_list: Vec<(String, String, String, String, bool)> = Vec::new();

        for dep in &deps {
            let dep_pid = dep["project_id"].as_str().unwrap_or("");
            let dep_type = dep["dependency_type"].as_str().unwrap_or("required");

            let dep_proj = modrinth::get_project(dep_pid);
            let dep_title = dep_proj["title"].as_str().unwrap_or(dep_pid);
            let dep_slug = dep_proj["slug"].as_str().unwrap_or(dep_pid);

            let dep_versions = modrinth::get_project_versions(
                dep_pid,
                loader.map(|l| vec![l.to_string()]).as_deref(),
                Some(&[mc_version.to_string()]),
            );

            let (dep_ver_num, dep_mc) = if let Some(arr) = dep_versions.as_array() {
                if arr.is_empty() {
                    ("(no matching version)".to_string(), "?".to_string())
                } else {
                    let best = &arr[0];
                    (
                        best["version_number"]
                            .as_str()
                            .unwrap_or(best["id"].as_str().unwrap_or("?"))
                            .to_string(),
                        best["game_versions"]
                            .as_array()
                            .map(|gv| {
                                gv.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|| "?".to_string()),
                    )
                }
            } else {
                ("(no matching version)".to_string(), "?".to_string())
            };

            let dep_key_lower = dep_slug.to_lowercase();
            let dep_title_lower = dep_title.to_lowercase().replace(' ', "_");
            let present = existing_jars.iter().any(|n| {
                n.contains(&dep_key_lower) || n.contains(&dep_title_lower)
            });

            match dep_type {
                "required" => {
                    if present {
                        crate::info!(
                            "[dep: required] {} -- already installed",
                            dep_title
                        );
                    } else {
                        required_missing.push((
                            dep_title.to_string(),
                            dep_slug.to_string(),
                            dep_ver_num,
                            dep_mc,
                        ));
                    }
                }
                _ => {
                    if present {
                        crate::info!(
                            "[dep: optional] {} -- already installed",
                            dep_title
                        );
                    }
                    optional_list.push((
                        dep_title.to_string(),
                        dep_slug.to_string(),
                        dep_ver_num,
                        dep_mc,
                        present,
                    ));
                }
            }
        }

        if required_missing.is_empty() && optional_list.is_empty() {
            return;
        }

        println!();
        crate::log::header("Dependencies");

        if !required_missing.is_empty() {
            println!(
                "  \x1b[1m\x1b[91m[ MUST INSTALL ] Required dependencies:\x1b[0m"
            );
            for (title, slug, ver, mc) in &required_missing {
                println!("    \x1b[91m- {}\x1b[0m", title);
                println!("      slug:    {}", slug);
                println!("      version: {}  (MC: {})", ver, mc);
                print!(
                    "      install: mc-launcher install-mod {} -v {}",
                    slug, mc_version
                );
                if let Some(l) = loader {
                    print!(" --loader {}", l);
                }
                println!();
            }
        }

        if !optional_list.is_empty() {
            println!(
                "  \x1b[1m\x1b[93m[ RECOMMENDED ] Optional dependencies:\x1b[0m"
            );
            for (title, slug, ver, mc, present) in &optional_list {
                let mark = if *present {
                    "\x1b[92m[installed]\x1b[0m "
                } else {
                    ""
                };
                println!("    {}{}", mark, title);
                println!("      slug:    {}", slug);
                println!("      version: {}  (MC: {})", ver, mc);
                print!(
                    "      install: mc-launcher install-mod {} -v {}",
                    slug, mc_version
                );
                if let Some(l) = loader {
                    print!(" --loader {}", l);
                }
                println!();
            }
        }

        println!();
        if !required_missing.is_empty() {
            crate::warn_msg!(
                "{} required dependency(ies) missing. Install them first, or the mod may not load.",
                required_missing.len()
            );
        }
        println!();
    }

    /// List installed mods for a version.
    pub fn list_mods(&self, mc_version: &str) -> Vec<(String, bool, u64)> {
        let mods_dir = self.mods_dir(mc_version);
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&mods_dir) {
            let mut files: Vec<_> = entries.flatten().collect();
            files.sort_by_key(|e| e.file_name());
            for f in files {
                let name = f.file_name().to_string_lossy().to_string();
                let size = f.metadata().map(|m| m.len()).unwrap_or(0);
                if name.ends_with(".disabled") {
                    results.push((name[..name.len() - 9].to_string(), false, size));
                } else if name.ends_with(".jar") {
                    results.push((name, true, size));
                }
            }
        }
        results
    }

    /// Disable a mod.
    pub fn disable_mod(&self, slug: &str, mc_version: &str) -> bool {
        let mods_dir = self.mods_dir(mc_version);
        let target = mods_dir.join(format!("{}.jar", slug));
        if target.exists() {
            let new_name = format!("{}.jar.disabled", slug);
            std::fs::rename(&target, mods_dir.join(&new_name)).is_ok()
        } else {
            // Fuzzy match
            if let Ok(entries) = std::fs::read_dir(&mods_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.ends_with(".jar")
                        && name.contains(&slug.to_lowercase())
                    {
                        let new_name =
                            format!("{}.disabled", entry.file_name().to_string_lossy());
                        return std::fs::rename(
                            entry.path(),
                            mods_dir.join(&new_name),
                        )
                        .is_ok();
                    }
                }
            }
            false
        }
    }

    /// Enable a disabled mod.
    pub fn enable_mod(&self, slug: &str, mc_version: &str) -> bool {
        let mods_dir = self.mods_dir(mc_version);
        let target = mods_dir.join(format!("{}.jar.disabled", slug));
        if target.exists() {
            let new_name = &target.file_name().unwrap().to_string_lossy().to_string();
            std::fs::rename(
                &target,
                mods_dir.join(&new_name[..new_name.len() - 9]),
            )
            .is_ok()
        } else {
            // Fuzzy match
            if let Ok(entries) = std::fs::read_dir(&mods_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if name.ends_with(".jar.disabled")
                        && name.contains(&slug.to_lowercase())
                    {
                        let old_name =
                            entry.file_name().to_string_lossy().to_string();
                        return std::fs::rename(
                            entry.path(),
                            mods_dir.join(&old_name[..old_name.len() - 9]),
                        )
                        .is_ok();
                    }
                }
            }
            false
        }
    }

    /// Uninstall a mod.
    pub fn uninstall_mod(&self, slug: &str, mc_version: &str) -> Vec<String> {
        let mods_dir = self.mods_dir(mc_version);
        let mut deleted = Vec::new();

        for pattern in &[
            format!("{}.jar", slug),
            format!("{}.jar.disabled", slug),
        ] {
            let target = mods_dir.join(pattern);
            if target.exists() {
                std::fs::remove_file(&target).ok();
                deleted.push(
                    target
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }

        if deleted.is_empty() {
            // Fuzzy match
            if let Ok(entries) = std::fs::read_dir(&mods_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    if (name.ends_with(".jar") || name.ends_with(".jar.disabled"))
                        && name.contains(&slug.to_lowercase())
                    {
                        let fname = entry
                            .file_name()
                            .to_string_lossy()
                            .to_string();
                        std::fs::remove_file(entry.path()).ok();
                        deleted.push(fname);
                    }
                }
            }
        }

        deleted
    }
}
