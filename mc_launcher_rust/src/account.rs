//! Account persistence — stores Microsoft / offline credentials.

use std::fs;
use std::path::PathBuf;

use crate::util;

const ACCOUNTS_FILE: &str = "launcher_accounts.json";

pub struct AccountManager {
    path: PathBuf,
    data: serde_json::Value,
}

impl AccountManager {
    pub fn new(game_dir: &std::path::Path) -> Self {
        let path = game_dir.join(ACCOUNTS_FILE);
        let data = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| {
                    serde_json::json!({"accounts": {}, "default": null})
                })
        } else {
            serde_json::json!({"accounts": {}, "default": null})
        };
        AccountManager { path, data }
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(
            &self.path,
            serde_json::to_string_pretty(&self.data).unwrap_or_default(),
        )
        .ok();
    }

    pub fn set_msa(
        &mut self,
        username: &str,
        uid: &str,
        access_token: &str,
        refresh_token: &str,
        expires_at: f64,
    ) {
        self.data["accounts"]["msa"] = serde_json::json!({
            "type": "msa",
            "username": username,
            "uuid": uid,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expires_at": expires_at,
        });
        self.data["default"] = serde_json::json!("msa");
        self.save();
    }

    pub fn set_offline(&mut self, username: &str) {
        let uid = util::offline_uuid(username);
        self.data["accounts"]["offline"] = serde_json::json!({
            "type": "offline",
            "username": username,
            "uuid": uid,
        });
        self.data["default"] = serde_json::json!("offline");
        self.save();
    }

    pub fn get_default(&self) -> Option<serde_json::Value> {
        let key = self.data["default"].as_str().unwrap_or("");
        if !key.is_empty() {
            if let Some(acc) = self.data["accounts"].get(key) {
                return Some(acc.clone());
            }
        }
        for k in &["msa", "offline"] {
            if let Some(acc) = self.data["accounts"].get(*k) {
                return Some(acc.clone());
            }
        }
        None
    }

    pub fn accounts(&self) -> &serde_json::Value {
        &self.data["accounts"]
    }

    pub fn default_key(&self) -> Option<String> {
        self.data["default"].as_str().map(String::from)
    }

    pub fn clear(&mut self) {
        self.data = serde_json::json!({"accounts": {}, "default": null});
        self.save();
    }
}
