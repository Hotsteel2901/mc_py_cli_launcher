//! MC CLI Launcher -- Rust rewrite
//!
//! Simple Minecraft CLI launcher: Microsoft login, offline mode,
//! Fabric/Forge/NeoForge loaders, and Modrinth mods.

mod account;
mod auth;
mod error;
mod fabric;
mod forge;
mod http;
mod java;
pub mod log;
mod launcher;
mod mod_manager;
mod modrinth;
mod neoforge;
mod util;
mod version;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{arg, value_parser, ArgAction, Command};
use rayon::prelude::*;

use crate::auth::MicrosoftAuth;
use crate::fabric::FabricManager;
use crate::forge::ForgeManager;
use crate::http::SourceMode;
use crate::launcher::MinecraftLauncher;
use crate::mod_manager::ModManager;
use crate::modrinth as mr;
use crate::neoforge::NeoForgeManager;
use crate::version::VersionManager;

/// Parse a RAM string like "4G", "2048M", or plain integer in MB.
fn parse_ram(value: &str) -> Result<u32, String> {
    let value = value.trim().to_uppercase();
    let re = regex::Regex::new(r"^(\d+(?:\.\d+)?)\s*(G|GB|M|MB)?$").unwrap();
    let caps = re.captures(&value).ok_or_else(|| {
        format!(
            "Invalid RAM value: '{}'. Use format like 4G, 2048M, or a plain number.",
            value
        )
    })?;
    let num: f64 = caps[1].parse().unwrap_or(0.0);
    let unit = caps.get(2).map(|m| m.as_str()).unwrap_or("M");
    if unit.starts_with('G') {
        Ok((num * 1024.0) as u32)
    } else {
        Ok(num as u32)
    }
}

fn main() {
    let matches = Command::new("mc-launcher")
        .disable_version_flag(true)
        .version(env!("CARGO_PKG_VERSION"))
        .about(
            "Simple Minecraft CLI Launcher -- Microsoft + offline + \
             Fabric/Forge/NeoForge + Modrinth mods",
        )
        .after_help(
            "Examples:\n  \
             mc-launcher login                       # Microsoft login (browser)\n  \
             mc-launcher login --device-code         # Device code login\n  \
             mc-launcher offline Steve               # Offline mode\n  \
             mc-launcher play                        # Launch with saved account\n  \
             mc-launcher play -v 1.21.4              # Launch specific version\n  \
             mc-launcher play -v 1.21.4 --ram 4G     # Allocate 4 GB RAM\n  \
             mc-launcher play --fabric               # Launch with Fabric\n  \
             mc-launcher play --forge                # Launch with Forge\n  \
             mc-launcher play --neoforge             # Launch with NeoForge\n  \
             mc-launcher accounts                    # Show saved accounts\n  \
             mc-launcher download                    # Download latest version\n  \
             mc-launcher download -v 1.20.1 --no-assets\n  \
             mc-launcher list-versions               # List all MC versions\n  \
             mc-launcher list-loaders                # List all mod loaders\n  \
             mc-launcher search sodium               # Search mods on Modrinth\n  \
             mc-launcher search-more sodium          # Detailed mod info\n  \
             mc-launcher install-fabric -v 1.21.4    # Install Fabric\n  \
             mc-launcher install-forge -v 1.20.1     # Install Forge\n  \
             mc-launcher install-neoforge -v 1.21.4  # Install NeoForge\n  \
             mc-launcher install-mod sodium -v 1.21.4\n  \
             mc-launcher list-installed              # List installed versions\n  \
             mc-launcher list-mods -v 1.21.4         # List mods\n  \
             mc-launcher disable-mod sodium -v 1.21.4\n  \
             mc-launcher enable-mod sodium -v 1.21.4\n  \
             mc-launcher uninstall-mod sodium -v 1.21.4\n  \
             mc-launcher logout                      # Clear saved session",
        )
        // Global options
        .arg(
            arg!(-d --dir <DIR> "Game directory"),
        )
        .arg(
            arg!(-t --threads <N> "Parallel download threads")
                .value_parser(value_parser!(usize))
                .default_value("4"),
        )
        .arg(arg!(-j --java <PATH> "Path to Java executable"))
        .arg(arg!(-v --version <VERSION> "Minecraft version"))
        .arg(
            arg!(-l --loader <LOADER> "Mod loader filter (fabric, forge, neoforge)"),
        )
        .arg(arg!(--"loader-version" <VERSION> "Specific loader version ID"))
        .arg(arg!(--"mod-version" <ID> "Specific mod version ID"))
        .arg(
            arg!(-r --ram <RAM> "RAM allocation (e.g. 4G, 2048M)")
                .value_parser(parse_ram)
                .default_value("4096"),
        )
        .arg(
            arg!(--width <PIXELS> "Game window width")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            arg!(--height <PIXELS> "Game window height")
                .value_parser(value_parser!(u32)),
        )
        .arg(
            arg!(--fabric "Launch with Fabric loader")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--forge "Launch with Forge loader")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--neoforge "Launch with NeoForge loader")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--"no-assets" "Skip asset downloads")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--official "Force use official Mojang/Forge URLs (no mirror)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--bmcl "Force use BMCLAPI mirror (大陆用户推荐)")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--"device-code" "Use device code login")
                .action(ArgAction::SetTrue),
        )
        .arg(
            arg!(--limit <N> "Max search results")
                .value_parser(value_parser!(u32))
                .default_value("10"),
        )
        // Positional: action
        .arg(
            arg!([ACTION] "Action to perform")
                .default_value("play")
                .value_parser([
                    "login", "offline", "play", "launch", "download",
                    "logout", "accounts", "list-versions", "list-loaders",
                    "list-installed", "list-mods", "search", "search-more",
                    "install-fabric", "install-forge", "install-neoforge",
                    "install-mod", "disable-mod", "enable-mod", "uninstall-mod",
                ]),
        )
        // Positional: query
        .arg(arg!([QUERY] "Username (offline) or mod search query / mod slug"))
        .get_matches();

    // Extract options
    let game_dir = PathBuf::from(
        matches
            .get_one::<String>("dir")
            .cloned()
            .unwrap_or_else(|| crate::util::default_game_dir().to_string_lossy().to_string()),
    );
    let threads = *matches.get_one::<usize>("threads").unwrap();
    let mut action = matches.get_one::<String>("ACTION").unwrap().clone();
    let query = matches.get_one::<String>("QUERY").cloned();
    let mc_version = matches.get_one::<String>("version").cloned();
    let loader_arg = matches.get_one::<String>("loader").cloned();
    let loader_version = matches.get_one::<String>("loader-version").cloned();
    let mod_version = matches.get_one::<String>("mod-version").cloned();
    let ram_mb = *matches.get_one::<u32>("ram").unwrap();
    let java_path = matches.get_one::<String>("java").cloned();
    let width = matches.get_one::<u32>("width").copied();
    let height = matches.get_one::<u32>("height").copied();
    let no_assets = matches.get_flag("no-assets");
    let device_code = matches.get_flag("device-code");
    let official_flag = matches.get_flag("official");
    let bmcl_flag = matches.get_flag("bmcl");
    let fabric_flag = matches.get_flag("fabric");
    let forge_flag = matches.get_flag("forge");
    let neoforge_flag = matches.get_flag("neoforge");
    let limit = *matches.get_one::<u32>("limit").unwrap();

    // Resolve source mode: manual > auto-detect
    let manual_mode = if official_flag && bmcl_flag {
        crate::die!("Cannot use both --official and --bmcl.");
    } else if official_flag {
        Some(SourceMode::Official)
    } else if bmcl_flag {
        Some(SourceMode::BmclApi)
    } else {
        None
    };

    let resolved_mode = http::resolve_source_mode(manual_mode);
    http::set_source_mode(resolved_mode);
    let mode_label = match resolved_mode {
        SourceMode::Official => "official",
        SourceMode::BmclApi => "BMCLAPI mirror",
        _ => "auto",
    };
    crate::info!("Download source: {} (fallback enabled)", mode_label);

    // Normalize alias
    if action == "launch" {
        action = "play".to_string();
    }

    // Resolve loader from flags
    let loader = resolve_loader(loader_arg, fabric_flag, forge_flag, neoforge_flag);

    let mut launcher = MinecraftLauncher::new(&game_dir, threads);
    if let Some(ref j) = java_path {
        launcher.set_java(j);
    }

    // --- Dispatch ---
    match action.as_str() {
        "logout" => cmd_logout(&mut launcher),
        "accounts" => cmd_accounts(&launcher),
        "list-versions" => cmd_list_versions(),
        "list-loaders" => cmd_list_loaders(),
        "list-installed" => cmd_list_installed(&game_dir, &launcher),
        "list-mods" => cmd_list_mods(&game_dir, mc_version),
        "search" => cmd_search(&game_dir, query, limit, mc_version, loader.as_deref()),
        "search-more" => cmd_search_more(&game_dir, query),
        "install-fabric" => {
            cmd_install_fabric(&game_dir, mc_version, loader_version.as_deref())
        }
        "install-forge" => {
            cmd_install_forge(&game_dir, mc_version, loader_version.as_deref())
        }
        "install-neoforge" => {
            cmd_install_neoforge(&game_dir, mc_version, loader_version.as_deref())
        }
        "install-mod" => cmd_install_mod(
            &game_dir, query, mc_version, loader.as_deref(), mod_version.as_deref(),
        ),
        "disable-mod" => cmd_disable_mod(&game_dir, query, mc_version),
        "enable-mod" => cmd_enable_mod(&game_dir, query, mc_version),
        "uninstall-mod" => cmd_uninstall_mod(&game_dir, query, mc_version),
        "login" => cmd_login(&mut launcher, device_code),
        "offline" => cmd_offline(&mut launcher, query.as_deref().unwrap_or("Steve")),
        "download" => cmd_download(&mut launcher, mc_version.as_deref(), no_assets),
        "play" => cmd_play(
            &mut launcher, mc_version.as_deref(), ram_mb, loader.as_deref(), width, height,
        ),
        _ => {
            eprintln!("Unknown action: {}", action);
            std::process::exit(1);
        }
    }
}

fn resolve_loader(
    loader_arg: Option<String>,
    fabric: bool,
    forge: bool,
    neoforge: bool,
) -> Option<String> {
    if forge && neoforge {
        crate::die!("Cannot use both --forge and --neoforge.");
    }
    if let Some(l) = loader_arg {
        return Some(l);
    }
    if neoforge {
        return Some("neoforge".into());
    }
    if forge {
        return Some("forge".into());
    }
    if fabric {
        return Some("fabric".into());
    }
    None
}

// ─── command implementations ──────────────────────────────────────

fn cmd_logout(launcher: &mut MinecraftLauncher) {
    launcher.accounts.clear();
    println!("  Cleared all saved accounts.");
}

fn cmd_accounts(launcher: &MinecraftLauncher) {
    crate::log::header("Saved Accounts");
    let accs = launcher.accounts.accounts();
    if accs.as_object().is_none_or(|o| o.is_empty()) {
        crate::warn_msg!("No accounts saved.");
        crate::info!("Login:  mc-launcher login");
        crate::info!("Offline: mc-launcher offline <username>");
        return;
    }

    let default_key = launcher.accounts.default_key();
    for (key, acc) in accs.as_object().unwrap() {
        let is_default = if Some(key) == default_key.as_ref() {
            " (default)"
        } else {
            ""
        };
        let acc_type = acc["type"].as_str().unwrap_or("?");
        let username = acc["username"].as_str().unwrap_or("?");
        println!("  [{}] {}{}", acc_type, username, is_default);

        if acc_type == "msa" {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let expires = acc["expires_at"].as_f64().unwrap_or(0.0);
            if now > expires {
                println!("         Session expired -- will auto-refresh on next launch");
            } else {
                let remaining = expires - now;
                let hours = (remaining / 3600.0) as u32;
                println!("         Session valid (~{}h remaining)", hours);
            }
        }
    }
}

fn cmd_list_versions() {
    crate::log::header("Minecraft Versions (from Modrinth)");
    let versions = mr::list_game_versions();
    let mut arr: Vec<_> = versions.as_array().cloned().unwrap_or_default();
    arr.sort_by(|a, b| {
        b["date"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date"].as_str().unwrap_or(""))
    });

    for v in &arr {
        let marker = if v["major"].as_bool().unwrap_or(false) {
            " *"
        } else {
            ""
        };
        println!(
            "  {: <12} {: <10} {}{}",
            v["version"].as_str().unwrap_or("?"),
            v["version_type"].as_str().unwrap_or("?"),
            v["date"]
                .as_str()
                .unwrap_or("?")
                .chars()
                .take(10)
                .collect::<String>(),
            marker
        );
    }
    crate::info!("Total: {} versions", arr.len());
}

fn cmd_list_loaders() {
    crate::log::header("Mod Loaders");
    let loaders = mr::list_loaders();
    println!("  Modrinth loaders:");
    for l in &loaders {
        println!("  - {}", l);
    }
    println!("  Built-in loaders:");
    for l in &["fabric", "forge", "neoforge", "quilt"] {
        println!("  - {}", l);
    }
    crate::info!(
        "Total: {} loaders from Modrinth + built-in loaders",
        loaders.len()
    );
}

fn cmd_list_installed(
    game_dir: &std::path::Path,
    launcher: &MinecraftLauncher,
) {
    crate::log::header("Locally Installed Minecraft Versions");
    let versions = ModManager::list_installed_versions(game_dir);
    if versions.is_empty() {
        crate::warn_msg!("No versions installed.");
        crate::info!("Download one: mc-launcher download -v <version>");
        return;
    }

    for v in &versions {
        let vdir = game_dir.join("versions").join(v);
        let jar = vdir.join(format!("{}.jar", v));
        let jar_size = if jar.exists() {
            let size =
                jar.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
            format!("  ({:.1} MB)", size)
        } else {
            String::new()
        };

        let mods_dir = vdir.join("mods");
        let mod_count = count_jars(&mods_dir, ".jar");
        let dis_count = count_jars(&mods_dir, ".jar.disabled");

        let mut tags: Vec<String> = Vec::new();
        for loader in &["fabric", "forge", "neoforge"] {
            if game_dir
                .join("libraries")
                .join(loader)
                .join(format!("{}-profile-{}.json", loader, v))
                .exists()
            {
                tags.push(match *loader {
                    "fabric" => "Fabric",
                    "forge" => "Forge",
                    "neoforge" => "NeoForge",
                    _ => loader,
                }.to_string());
            }
        }
        if mod_count > 0 {
            tags.push(format!("{} mods", mod_count));
        }
        if dis_count > 0 {
            tags.push(format!("{} disabled", dis_count));
        }

        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", tags.join(", "))
        };
        println!("  {: <12}{}{}", v, jar_size, tag_str);
    }

    println!("\n  Total: {} version(s)", versions.len());
    if let Some(acc) = launcher.accounts.get_default() {
        println!(
            "  Account: {} ({})",
            acc["username"].as_str().unwrap_or("?"),
            acc["type"].as_str().unwrap_or("?")
        );
    } else {
        println!("  No account saved. Run: mc-launcher login");
    }
}

fn count_jars(dir: &std::path::Path, suffix: &str) -> usize {
    if !dir.exists() {
        return 0;
    }
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(suffix))
                .count()
        })
        .unwrap_or(0)
}

fn cmd_list_mods(game_dir: &std::path::Path, mc_version: Option<String>) {
    let mc_version = resolve_mc_version(mc_version, game_dir);
    crate::log::header(&format!("Mods for Minecraft {}", mc_version));

    let mm = ModManager::new(game_dir);
    let mods = mm.list_mods(&mc_version);
    if mods.is_empty() {
        crate::warn_msg!("No mods installed for {}.", mc_version);
        crate::info!("Install: mc-launcher install-mod <slug> -v {}", mc_version);
        return;
    }

    for (name, enabled, size) in &mods {
        let status = if *enabled { "[enabled] " } else { "[DISABLED]" };
        println!(
            "  {} {: <50} {:>8.1} KB",
            status,
            name,
            *size as f64 / 1024.0
        );
    }
    let enabled_count = mods.iter().filter(|(_, e, _)| *e).count();
    let disabled_count = mods.iter().filter(|(_, e, _)| !*e).count();
    crate::info!("{} enabled, {} disabled", enabled_count, disabled_count);
}

fn cmd_search(
    game_dir: &std::path::Path,
    query: Option<String>,
    limit: u32,
    mc_version: Option<String>,
    loader: Option<&str>,
) {
    let query = query.unwrap_or_else(|| {
        crate::die!("Please provide a search query.\n  Example: mc-launcher search sodium");
    });

    crate::log::header(&format!("Searching for: {} (source: modrinth)", query));

    let mm = ModManager::new(game_dir);
    let hits = mm.search(&query, limit, mc_version.as_deref(), loader);

    if hits.is_empty() {
        crate::warn_msg!("No results found.");
        return;
    }

    // Parallel loader support queries
    let support_map: std::collections::HashMap<
        String,
        std::collections::BTreeMap<String, (String, String)>,
    > = hits
        .par_iter()
        .filter_map(|h| {
            let pid = h["project_id"].as_str()?;
            let support = mm.loader_support(pid);
            Some((pid.to_string(), support))
        })
        .collect();

    for (i, h) in hits.iter().enumerate() {
        let title =
            h["title"].as_str().unwrap_or(h["slug"].as_str().unwrap_or("?"));
        let desc = h["description"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(100)
            .collect::<String>();
        let author = h["author"].as_str().unwrap_or("?");
        let downloads = h["downloads"].as_u64().unwrap_or(0);
        let categories = h["categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let source = h["source"].as_str().unwrap_or("?");

        println!("  [{}.] {}", i + 1, title);
        println!(
            "      slug: {}  |  source: {}",
            h["slug"].as_str().unwrap_or("?"),
            source
        );
        println!(
            "      by: {}  |  downloads: {}",
            author,
            format_num(downloads)
        );
        println!("      categories: {}", categories);

        if let Some(support) =
            support_map.get(h["project_id"].as_str().unwrap_or(""))
        {
            if !support.is_empty() {
                println!(
                    "      support: {}",
                    ModManager::format_loader_support(support)
                );
            }
        }
        if !desc.is_empty() {
            println!("      {}...", desc);
        }
        println!();
    }

    crate::info!("Found {} result(s).", hits.len());
    crate::info!("Install with: mc-launcher install-mod <slug> -v <version>");
    crate::info!("Details with: mc-launcher search-more <slug>");
}

fn cmd_search_more(_game_dir: &std::path::Path, query: Option<String>) {
    let slug = query.unwrap_or_else(|| {
        crate::die!(
            "Please provide the exact mod slug.\n  Example: mc-launcher search-more sodium"
        );
    });

    let project = mr::get_project(&slug);
    let project_id = project["id"].as_str().unwrap_or(&slug);
    let versions = mr::get_project_versions(project_id, None, None);
    let support = ModManager::summarize_loader_support(&versions);

    let title = project["title"].as_str().unwrap_or(&slug);
    crate::log::header(title);

    println!(
        "  slug:        {}",
        project["slug"].as_str().unwrap_or("?")
    );
    println!(
        "  project id:  {}",
        project["id"].as_str().unwrap_or("?")
    );
    println!(
        "  downloads:   {}",
        format_num(project["downloads"].as_u64().unwrap_or(0))
    );
    println!(
        "  followers:   {}",
        format_num(project["followers"].as_u64().unwrap_or(0))
    );
    println!(
        "  client/server: {} / {}",
        project["client_side"].as_str().unwrap_or("?"),
        project["server_side"].as_str().unwrap_or("?")
    );
    let lic = &project["license"];
    println!("  license:     {}", lic["id"].as_str().unwrap_or("?"));
    println!(
        "  categories:  {}",
        project["categories"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    );
    println!(
        "  updated:     {}",
        project["updated"]
            .as_str()
            .unwrap_or("?")
            .chars()
            .take(10)
            .collect::<String>()
    );
    if let Some(src) = project["source_url"].as_str() {
        println!("  source:      {}", src);
    }
    if let Some(issues) = project["issues_url"].as_str() {
        println!("  issues:      {}", issues);
    }
    if let Some(desc) = project["description"].as_str() {
        println!("\n  {}", desc);
    }

    println!("\n  Loader support (highest game version):");
    let shown_loaders = ["fabric", "forge", "neoforge"];
    for l in &shown_loaders {
        if let Some((mc, modver)) = support.get(*l) {
            println!(
                "    {: <10} <= MC {: <10} latest mod version: {}",
                l, mc, modver
            );
        } else {
            println!("    {: <10} not supported", l);
        }
    }
    for (l, (mc, modver)) in &support {
        if !shown_loaders.contains(&l.as_str()) {
            println!(
                "    {: <10} <= MC {: <10} latest mod version: {}",
                l, mc, modver
            );
        }
    }

    let mc_re = regex::Regex::new(r"^\d+\.\d+(\.\d+)?$").unwrap();
    let all_mc: std::collections::BTreeSet<String> = versions
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .flat_map(|v| {
            v["game_versions"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|gv| {
                    let s = gv.as_str()?;
                    if mc_re.is_match(s) {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    if !all_mc.is_empty() {
        let mc_sorted: Vec<_> = all_mc.iter().collect();
        let shown: Vec<_> = mc_sorted.iter().rev().take(12).rev().collect();
        let prefix = if mc_sorted.len() > 12 { "..., " } else { "" };
        println!(
            "\n  Supported MC releases: {}{}",
            prefix,
            shown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let mut versions_arr = versions.as_array().cloned().unwrap_or_default();
    versions_arr.sort_by(|a, b| {
        b["date_published"]
            .as_str()
            .unwrap_or("")
            .cmp(a["date_published"].as_str().unwrap_or(""))
    });

    println!(
        "\n  Recent versions ({} of {}):",
        versions_arr.len().min(8),
        versions_arr.len()
    );
    for v in versions_arr.iter().take(8) {
        let vn = v["version_number"]
            .as_str()
            .unwrap_or(v["id"].as_str().unwrap_or("?"));
        let gv: Vec<&str> = v["game_versions"]
            .as_array()
            .map(|arr| {
                let all: Vec<&str> = arr.iter()
                    .filter_map(|x| x.as_str())
                    .collect();
                let start = if all.len() > 4 { all.len() - 4 } else { 0 };
                all[start..].to_vec()
            })
            .unwrap_or_default();
        let gv_str = gv.join(", ");
        let ld: Vec<&str> = v["loaders"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        let ld_str = ld.join(", ");
        let date = v["date_published"]
            .as_str()
            .unwrap_or("?")
            .chars()
            .take(10)
            .collect::<String>();
        let vtype = v["version_type"].as_str().unwrap_or("?");
        println!(
            "    {: <36} {: <8} MC {: <24} [{}]  {}",
            vn, vtype, gv_str, ld_str, date
        );
    }
    println!(
        "\n  Install: mc-launcher install-mod {} -v <version> --loader <loader>",
        project["slug"].as_str().unwrap_or(&slug)
    );
}

fn cmd_install_fabric(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install Fabric Loader");
    crate::info!("Target MC version: {}", mc_version);

    let fm = FabricManager::new(game_dir);
    let (all_jars, profile) = fm.install(&mc_version, loader_version)
        .unwrap_or_else(|e| crate::die!(format!("Fabric install failed: {}", e)));

    crate::success!("Fabric Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Profile:    {}", profile["id"].as_str().unwrap_or("?"));
    println!(
        "    Libraries:  {} jars -> {}",
        all_jars.len(),
        game_dir.join("libraries").display()
    );
    println!(
        "\n  Launch with: mc-launcher play -v {} --fabric",
        mc_version
    );
}

fn cmd_install_forge(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install Forge Loader");
    crate::info!("Target MC version: {}", mc_version);

    let fm = ForgeManager::new(game_dir);
    let (installed_id, profile) = fm.install(&mc_version, loader_version)
        .unwrap_or_else(|e| crate::die!(format!("Forge install failed: {}", e)));

    crate::success!("Forge Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Version ID: {}", installed_id);
    println!(
        "    Main Class: {}",
        profile["mainClass"].as_str().unwrap_or("?")
    );
    println!(
        "\n  Launch with: mc-launcher play -v {} --forge",
        mc_version
    );
}

fn cmd_install_neoforge(
    game_dir: &std::path::Path,
    mc_version: Option<String>,
    loader_version: Option<&str>,
) {
    let mc_version = resolve_mc_version_or_latest(mc_version, game_dir);
    crate::log::header("Install NeoForge Loader");
    crate::info!("Target MC version: {}", mc_version);

    let nm = NeoForgeManager::new(game_dir);
    let (installed_id, profile) = nm.install(&mc_version, loader_version)
        .unwrap_or_else(|e| crate::die!(format!("NeoForge install failed: {}", e)));

    crate::success!("NeoForge Loader installed successfully!");
    println!("    MC Version: {}", mc_version);
    println!("    Version ID: {}", installed_id);
    println!(
        "    Main Class: {}",
        profile["mainClass"].as_str().unwrap_or("?")
    );
    println!(
        "\n  Launch with: mc-launcher play -v {} --neoforge",
        mc_version
    );
}

fn cmd_install_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
    loader: Option<&str>,
    mod_version: Option<&str>,
) {
    let slug = query.unwrap_or_else(|| {
        crate::die!("Please provide a mod slug.\n  Example: mc-launcher install-mod sodium -v 1.21.4");
    });
    let mc_version = resolve_mc_version(mc_version, game_dir);

    crate::log::header(&format!("Install Mod for MC {}", mc_version));

    let mm = ModManager::new(game_dir);
    let (_paths, _version_data, project) =
        mm.install(&slug, &mc_version, loader, mod_version);

    let title = project["title"].as_str().unwrap_or(&slug);
    let source = project["source"].as_str().unwrap_or("?");
    crate::success!(
        "{} installed for Minecraft {} (source: {})!",
        title,
        mc_version,
        source
    );

    // Check which loader is installed for launch hint
    for l in &["fabric", "neoforge", "forge"] {
        if game_dir
            .join("libraries")
            .join(l)
            .join(format!("{}-profile-{}.json", l, mc_version))
            .exists()
        {
            println!(
                "  Launch with: mc-launcher play -v {} --{}",
                mc_version, l
            );
            return;
        }
    }
    println!(
        "  Install a loader first, e.g.: mc-launcher install-fabric -v {}",
        mc_version
    );
    println!(
        "  Then launch: mc-launcher play -v {} --fabric",
        mc_version
    );
}

fn cmd_disable_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
) {
    let slug = query.unwrap_or_else(|| crate::die!("Provide a mod slug/name to disable."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    if mm.disable_mod(&slug, &mc_version) {
        crate::success!("Disabled '{}' for Minecraft {}", slug, mc_version);
    } else {
        crate::die!(format!(
            "No mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_enable_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
) {
    let slug =
        query.unwrap_or_else(|| crate::die!("Provide a mod slug/name to enable."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    if mm.enable_mod(&slug, &mc_version) {
        crate::success!("Enabled '{}' for Minecraft {}", slug, mc_version);
    } else {
        crate::die!(format!(
            "No disabled mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_uninstall_mod(
    game_dir: &std::path::Path,
    query: Option<String>,
    mc_version: Option<String>,
) {
    let slug = query
        .unwrap_or_else(|| crate::die!("Provide a mod slug/name to uninstall."));
    let mc_version = resolve_mc_version(mc_version, game_dir);
    let mm = ModManager::new(game_dir);
    let deleted = mm.uninstall_mod(&slug, &mc_version);
    if !deleted.is_empty() {
        for d in &deleted {
            crate::success!("Deleted: {}", d);
        }
    } else {
        crate::die!(format!(
            "No mod matching '{}' found for {}",
            slug, mc_version
        ));
    }
}

fn cmd_login(launcher: &mut MinecraftLauncher, device_code: bool) {
    let mut auth = MicrosoftAuth::new();

    if device_code {
        auth.device_code_login().unwrap_or_else(|e| {
            crate::die!(format!("Login failed: {}", e));
        });
    } else {
        crate::log::header("Microsoft Login (Browser)");
        crate::info!("A browser window will open. Log in with your Microsoft account.");
        crate::info!("Make sure your Microsoft account owns Minecraft!\n");
        crate::info!("Tip: use --device-code for device code login.\n");
        auth.login().unwrap_or_else(|e| {
            crate::die!(format!("Login failed: {}", e));
        });
    }

    let uid = crate::util::format_uuid(&auth.uuid);
    launcher.accounts.set_msa(
        &auth.username,
        &uid,
        &auth.mc_token,
        &auth.refresh_token,
        auth.expires_at,
    );

    crate::success!("Logged in as: {}", auth.username);
    crate::info!("Credentials saved. Next steps:");
    crate::info!("  mc-launcher download -v <version>   # download a Minecraft version");
    crate::info!("  mc-launcher play -v <version>       # launch the game");
}

fn cmd_offline(launcher: &mut MinecraftLauncher, username: &str) {
    crate::log::header(&format!("Offline Mode: {}", username));
    launcher.accounts.set_offline(username);
    crate::success!("Offline account saved: {}", username);
    crate::info!("Next steps:");
    crate::info!("  mc-launcher download -v <version>   # download a Minecraft version");
    crate::info!("  mc-launcher play -v <version>       # launch the game");
}

fn cmd_download(
    launcher: &mut MinecraftLauncher,
    mc_version: Option<&str>,
    no_assets: bool,
) {
    crate::log::header("Download Only");
    launcher
        .download_version(mc_version, no_assets)
        .unwrap_or_else(|e| {
            crate::die!(format!("Download failed: {}", e));
        });
}

fn cmd_play(
    launcher: &mut MinecraftLauncher,
    mc_version: Option<&str>,
    ram_mb: u32,
    loader: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
) {
    let account = launcher.accounts.get_default().unwrap_or_else(|| {
        crate::die!(
            "No saved account. Run 'login' or 'offline <name>' first.",
            "  mc-launcher login        # Microsoft login\n  mc-launcher offline Steve # Offline mode"
        );
    });

    let exit_code = launcher
        .launch(mc_version, Some(account), ram_mb, loader, width, height)
        .unwrap_or_else(|e| {
            crate::die!(format!("Launch failed: {}", e));
        });
    std::process::exit(exit_code);
}

// ─── helpers ───────────────────────────────────────────────────────

fn resolve_mc_version(
    mc_version: Option<String>,
    game_dir: &std::path::Path,
) -> String {
    if let Some(v) = mc_version {
        return v;
    }
    let installed = ModManager::list_installed_versions(game_dir);
    if installed.is_empty() {
        crate::die!(
            "No versions installed. Specify --version.\n  Download first: mc-launcher download -v <version>"
        );
    }
    let v = installed.last().unwrap().clone();
    crate::info!("Auto-detected version: {}", v);
    v
}

fn resolve_mc_version_or_latest(
    mc_version: Option<String>,
    game_dir: &std::path::Path,
) -> String {
    if let Some(v) = mc_version {
        let vm = VersionManager::new(game_dir);
        let manifest = vm.fetch_manifest().unwrap_or_else(|e| {
            crate::die!(format!("Cannot fetch version manifest: {}", e));
        });
        let known: std::collections::HashSet<String> = manifest["versions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v["id"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if !known.contains(&v) {
            let mut close: Vec<_> = known
                .iter()
                .filter(|kv| kv.starts_with(&v[..v.len().min(4)]))
                .cloned()
                .collect();
            close.sort();
            let hint = if close.is_empty() {
                String::new()
            } else {
                let items: Vec<_> = close.iter().rev().take(8).rev().cloned().collect();
                format!("Did you mean: {}", items.join(", "))
            };
            crate::die!(
                format!("Minecraft version '{}' does not exist.", v),
                &hint
            );
        }
        return v;
    }

    let vm = VersionManager::new(game_dir);
    let manifest = vm.fetch_manifest().unwrap_or_else(|e| {
        crate::die!(format!("Cannot fetch version manifest: {}", e));
    });
    manifest["latest"]["release"]
        .as_str()
        .unwrap_or("1.21.4")
        .to_string()
}

fn format_num(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
