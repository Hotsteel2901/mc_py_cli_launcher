//! Microsoft OAuth2 authentication — browser login, device code, auth chain.

use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use url::form_urlencoded;

use crate::error::{AppError, AppResult};
use crate::http;
use crate::log;

const MS_CLIENT_ID: &str = "00000000402b5328";
const MS_REDIRECT: &str = "https://login.live.com/oauth20_desktop.srf";
const MS_SCOPE: &str = "service::user.auth.xboxlive.com::MBI_SSL";
const MS_AUTH_URL: &str = "https://login.live.com/oauth20_authorize.srf";
const MS_TOKEN_URL: &str = "https://login.live.com/oauth20_token.srf";
const MS_DEVICE_AUTH: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const MS_DEVICE_TOKEN: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str =
    "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

pub struct MicrosoftAuth {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: f64,
    pub mc_token: String,
    pub username: String,
    pub uuid: String,
}

impl MicrosoftAuth {
    pub fn new() -> Self {
        MicrosoftAuth {
            access_token: String::new(),
            refresh_token: String::new(),
            expires_at: 0.0,
            mc_token: String::new(),
            username: String::new(),
            uuid: String::new(),
        }
    }

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    /// Browser-based Microsoft login. Opens browser, user pastes redirect URL.
    pub fn login(&mut self) -> AppResult<bool> {
        let params = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", MS_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("scope", MS_SCOPE)
            .append_pair("redirect_uri", MS_REDIRECT)
            .append_pair("prompt", "select_account")
            .append_pair("lw", "1")
            .append_pair("fl", "dob,easi2")
            .append_pair("xsup", "1")
            .append_pair("nopa", "2")
            .finish();
        let auth_url = format!("{}?{}", MS_AUTH_URL, params);

        println!(
            "\n  {}\n",
            log::clr(
                "\x1b[96m",
                "+-------------------------------------------------------------+"
            )
        );
        println!("  |  [1/6] Microsoft Login                                      |");
        println!("  |  A browser will open. Sign in with your Microsoft account.  |");
        println!(
            "  |  After login, copy the FULL URL from the address bar.       |"
        );
        println!("  |  If browser does not open, go to:                           |");
        println!("  |    {}", auth_url);
        println!(
            "  {}\n",
            log::clr(
                "\x1b[96m",
                "+-------------------------------------------------------------+"
            )
        );

        webbrowser::open(&auth_url).ok();

        print!(
            "{}",
            log::clr("\x1b[96m", "  Paste redirect URL here: ")
        );
        io::stdout().flush().ok();
        let mut redirect_url = String::new();
        if io::stdin().read_line(&mut redirect_url).is_err() {
            return Err(AppError::Generic("Cancelled.".into()));
        }
        let redirect_url = redirect_url.trim().to_string();
        if redirect_url.is_empty() {
            return Err(AppError::Generic("No URL provided.".into()));
        }

        let auth_code = url::Url::parse(&redirect_url)
            .ok()
            .and_then(|u| {
                u.query_pairs()
                    .find(|(k, _)| k == "code")
                    .map(|(_, v)| v.to_string())
            })
            .unwrap_or_default();

        if auth_code.is_empty() {
            return Err(AppError::Generic(
                "Could not find 'code' in the URL. Make sure you copied the FULL URL.".into()
            ));
        }

        crate::log::step(2, 6, "Exchanging auth code for Microsoft token...");
        let token_body = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", MS_CLIENT_ID)
            .append_pair("code", &auth_code)
            .append_pair("grant_type", "authorization_code")
            .append_pair("redirect_uri", MS_REDIRECT)
            .append_pair("scope", MS_SCOPE)
            .finish();

        let (status, body) =
            http::http_post_form(MS_TOKEN_URL, token_body.as_bytes())
                .map_err(|e| AppError::Generic(format!("Token exchange failed: {}", e)))?;
        if status != 200 {
            let hint =
                String::from_utf8_lossy(&body).chars().take(500).collect::<String>();
            return Err(AppError::Generic(format!("Token exchange failed ({}) -- {}", status, hint)));
        }

        let ms_data: serde_json::Value = serde_json::from_slice(&body).unwrap();
        self.access_token =
            ms_data["access_token"].as_str().unwrap_or("").into();
        self.refresh_token =
            ms_data["refresh_token"].as_str().unwrap_or("").into();
        self.expires_at =
            Self::now() + ms_data["expires_in"].as_f64().unwrap_or(3600.0);

        match Self::do_full_auth_chain(&self.access_token) {
            Ok((mc, name, id, exp)) => {
                self.mc_token = mc;
                self.username = name;
                self.uuid = id;
                self.expires_at = exp;
            }
            Err(e) => return Err(AppError::Generic(e)),
        }

        crate::success!("Logged in as: {} ({})", self.username, self.uuid);
        Ok(true)
    }

    /// Device-code login (no browser available).
    pub fn device_code_login(&mut self) -> AppResult<bool> {
        println!(
            "\n  {}\n",
            log::clr(
                "\x1b[96m",
                "+-------------------------------------------------------------+"
            )
        );
        println!("  |  [1/2] Microsoft Device Code Login                          |");
        println!(
            "  {}\n",
            log::clr(
                "\x1b[96m",
                "+-------------------------------------------------------------+"
            )
        );

        let dev_body = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", MS_CLIENT_ID)
            .append_pair("scope", MS_SCOPE)
            .finish();

        let (status, body) =
            http::http_post_form(MS_DEVICE_AUTH, dev_body.as_bytes())
                .map_err(|e| AppError::Generic(format!("Device code request failed: {}", e)))?;
        if status != 200 {
            let hint =
                String::from_utf8_lossy(&body).chars().take(500).collect::<String>();
            return Err(AppError::Generic(format!("Device code request failed ({}) -- {}", status, hint)));
        }

        let dev_data: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let user_code = dev_data["user_code"].as_str().unwrap_or("");
        let device_code_val = dev_data["device_code"].as_str().unwrap_or("");
        let interval = dev_data["interval"].as_u64().unwrap_or(5);
        let expires_in = dev_data["expires_in"].as_u64().unwrap_or(900);

        println!("  +------------------------------------------------------+");
        println!("  |   Open:  https://microsoft.com/link                  |");
        println!(
            "  |   Enter this code:  {: <33}|",
            user_code
        );
        println!(
            "  |   Code expires in {:>2} minutes.                    |",
            expires_in / 60
        );
        println!("  +------------------------------------------------------+");
        println!("\n  Waiting for you to complete login...");

        let deadline = Self::now() + expires_in as f64;
        let mut poll_interval = interval;

        while Self::now() < deadline {
            std::thread::sleep(std::time::Duration::from_secs(poll_interval));
            let poll_body = form_urlencoded::Serializer::new(String::new())
                .append_pair("client_id", MS_CLIENT_ID)
                .append_pair(
                    "grant_type",
                    "urn:ietf:params:oauth:grant-type:device_code",
                )
                .append_pair("device_code", device_code_val)
                .finish();

            match http::http_post_form(MS_DEVICE_TOKEN, poll_body.as_bytes()) {
                Ok((200, body_bytes)) => {
                    let ms_data: serde_json::Value =
                        serde_json::from_slice(&body_bytes).unwrap();
                    self.access_token =
                        ms_data["access_token"].as_str().unwrap_or("").into();
                    self.refresh_token =
                        ms_data["refresh_token"].as_str().unwrap_or("").into();
                    self.expires_at = Self::now()
                        + ms_data["expires_in"].as_f64().unwrap_or(3600.0);
                    break;
                }
                Ok((_, body_bytes)) => {
                    let err: serde_json::Value =
                        serde_json::from_slice(&body_bytes).unwrap_or_default();
                    match err["error"].as_str() {
                        Some("authorization_pending") => {
                            print!(
                                "\r  Waiting... ({:.0}s left)",
                                deadline - Self::now()
                            );
                            io::stdout().flush().ok();
                        }
                        Some("slow_down") => {
                            poll_interval += 5;
                        }
                        _ => {
                            let desc = err["error_description"]
                                .as_str()
                                .unwrap_or("unknown error");
                            return Err(AppError::Generic(format!("Device login failed: {}", desc)));
                        }
                    }
                }
                Err(e) => {
                    return Err(AppError::Generic(format!("Device token request failed: {}", e)));
                }
            }
        }

        if self.access_token.is_empty() {
            return Err(AppError::Generic("Timed out waiting for device code login.".into()));
        }

        crate::success!("Authenticated with Microsoft!");
        crate::log::step(2, 2, "Completing Minecraft authentication...");
        match Self::do_full_auth_chain(&self.access_token) {
            Ok((mc, name, id, exp)) => {
                self.mc_token = mc;
                self.username = name;
                self.uuid = id;
                self.expires_at = exp;
            }
            Err(e) => return Err(AppError::Generic(e)),
        }

        crate::success!("Logged in as: {} ({})", self.username, self.uuid);
        Ok(true)
    }

    /// XBL -> XSTS -> Minecraft login chain.
    pub fn do_full_auth_chain(
        ms_access_token: &str,
    ) -> Result<(String, String, String, f64), String> {
        // XBL auth -- try both with and without "d=" prefix
        let mut xbl_token = String::new();
        let mut _uhs = String::new();

        let tickets = [
            ms_access_token.to_string(),
            format!("d={}", ms_access_token),
        ];
        for ticket in &tickets {
            let xbl_json = serde_json::json!({
                "Properties": {
                    "AuthMethod": "RPS",
                    "SiteName": "user.auth.xboxlive.com",
                    "RpsTicket": ticket,
                },
                "RelyingParty": "http://auth.xboxlive.com",
                "TokenType": "JWT",
            });

            match http::http_post_json(XBL_AUTH_URL, &xbl_json) {
                Ok((200, body)) => {
                    let xbl: serde_json::Value =
                        serde_json::from_slice(&body).map_err(|e| {
                            format!("XBL parse error: {}", e)
                        })?;
                    xbl_token =
                        xbl["Token"].as_str().unwrap_or("").to_string();
                    _uhs = xbl["DisplayClaims"]["xui"][0]["uhs"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    break;
                }
                _ => continue,
            }
        }

        if xbl_token.is_empty() {
            return Err("Xbox Live auth failed".into());
        }

        // XSTS auth
        let xsts_json = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbl_token],
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });

        let (xsts_status, xsts_body) =
            http::http_post_json(XSTS_AUTH_URL, &xsts_json)
                .map_err(|e| format!("XSTS request failed: {}", e))?;

        if xsts_status != 200 {
            let err: serde_json::Value =
                serde_json::from_slice(&xsts_body).unwrap_or_default();
            let xerr = err["XErr"].as_i64().unwrap_or(0);
            let msg = match xerr {
                2148916233 => {
                    "No Xbox Live profile. Create one at https://www.xbox.com/"
                }
                2148916235 => {
                    "Xbox Live is not available in your country/region."
                }
                2148916236 => {
                    "Adult verification required (South Korea age-gating)."
                }
                2148916237 => {
                    "Adult verification required (South Korea age-gating)."
                }
                2148916238 => {
                    "Child account -- must be added to an Xbox Family by an adult."
                }
                _ => "XSTS error",
            };
            return Err(format!("XSTS auth failed: {} (XErr {})", msg, xerr));
        }

        let xsts: serde_json::Value = serde_json::from_slice(&xsts_body)
            .map_err(|e| format!("XSTS parse error: {}", e))?;
        let xsts_token = xsts["Token"].as_str().unwrap_or("");
        let xsts_uhs = xsts["DisplayClaims"]["xui"][0]["uhs"]
            .as_str()
            .unwrap_or("");

        // Minecraft login
        let mc_json = serde_json::json!({
            "identityToken": format!("XBL3.0 x={};{}", xsts_uhs, xsts_token),
        });

        let (mc_status, mc_body) =
            http::http_post_json(MC_LOGIN_URL, &mc_json)
                .map_err(|e| format!("Minecraft login request failed: {}", e))?;

        if mc_status != 200 {
            return Err(format!(
                "Minecraft login failed ({})",
                mc_status
            ));
        }

        let mc_auth: serde_json::Value = serde_json::from_slice(&mc_body)
            .map_err(|e| format!("MC auth parse error: {}", e))?;
        let mc_token =
            mc_auth["access_token"].as_str().unwrap_or("").to_string();
        let expires =
            Self::now() + mc_auth["expires_in"].as_f64().unwrap_or(86400.0);

        // Minecraft profile
        let (prof_status, prof_body) = http::http_get_hdrs(
            MC_PROFILE_URL,
            &[("Authorization", &format!("Bearer {}", mc_token))],
        )
        .map_err(|e| format!("Profile request failed: {}", e))?;

        if prof_status != 200 {
            return Err(format!(
                "Minecraft profile fetch failed ({})",
                prof_status
            ));
        }

        let profile: serde_json::Value = serde_json::from_slice(&prof_body)
            .map_err(|e| format!("Profile parse error: {}", e))?;
        let username = profile["name"].as_str().unwrap_or("?").to_string();
        let uuid = profile["id"].as_str().unwrap_or("?").to_string();

        Ok((mc_token, username, uuid, expires))
    }

    /// Attempt to refresh an expired Microsoft token.
    pub fn try_refresh(&mut self, refresh_token: &str) -> bool {
        let body = form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", MS_CLIENT_ID)
            .append_pair("refresh_token", refresh_token)
            .append_pair("grant_type", "refresh_token")
            .append_pair("scope", MS_SCOPE)
            .finish();

        match http::http_post_form(MS_TOKEN_URL, body.as_bytes()) {
            Ok((200, data_bytes)) => {
                let data: serde_json::Value =
                    serde_json::from_slice(&data_bytes).unwrap();
                self.access_token =
                    data["access_token"].as_str().unwrap_or("").to_string();
                self.refresh_token = data["refresh_token"]
                    .as_str()
                    .unwrap_or(refresh_token)
                    .to_string();
                self.expires_at =
                    Self::now() + data["expires_in"].as_f64().unwrap_or(3600.0);

                match Self::do_full_auth_chain(&self.access_token) {
                    Ok((mc, name, id, exp)) => {
                        self.mc_token = mc;
                        self.username = name;
                        self.uuid = id;
                        self.expires_at = exp;
                        true
                    }
                    Err(_) => false,
                }
            }
            Ok((status, _)) => {
                crate::warn_msg!(
                    "Microsoft token refresh failed ({})",
                    status
                );
                false
            }
            Err(e) => {
                crate::warn_msg!("Microsoft token refresh failed: {}", e);
                false
            }
        }
    }
}
