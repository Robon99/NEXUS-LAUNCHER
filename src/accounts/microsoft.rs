use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use keyring::Entry;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpListener,
    io::ErrorKind,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;
use uuid::Uuid;

use crate::config::{Account, AccountKind, APP_SERVICE};

const AUTHORIZE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBOX_USER_AUTH: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XBOX_XSTS: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_ENTITLEMENTS: &str = "https://api.minecraftservices.com/entitlements/mcstore";
const MC_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct MinecraftProfile {
    id: String,
    name: String,
}

pub struct LoginResult {
    pub account: Account,
}

/// Opens the system browser for Microsoft sign-in (Authorization Code + PKCE),
/// receives the redirect on an ephemeral localhost port, then validates the
/// Microsoft -> Xbox Live -> XSTS -> Minecraft Services chain.
/// The application must be registered as a public desktop client with the
/// `http://localhost` redirect URI. No password is collected by NEXUS.
pub fn sign_in<F>(client_id: &str, status: F) -> Result<LoginResult>
where
    F: Fn(String) + Send + Sync + 'static,
{
    if client_id.trim().is_empty() {
        bail!("Не задан NEXUS_MICROSOFT_CLIENT_ID");
    }

    let client = Client::builder()
        .user_agent("NEXUS-Launcher/0.2.0")
        .timeout(Duration::from_secs(45))
        .build()?;

    status("Открываю официальный вход Microsoft в браузере…".into());
    let listener = TcpListener::bind(("localhost", 0))
        .context("Не удалось открыть локальный порт для возврата из Microsoft")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    // Microsoft treats localhost redirect ports as ephemeral for native apps;
    // register `http://localhost` in the Mobile and desktop application platform.
    let redirect_uri = format!("http://localhost:{port}");

    // Two UUIDs give a high-entropy PKCE verifier in the required 43..128 range.
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let state = Uuid::new_v4().simple().to_string();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

    let mut authorize = Url::parse(AUTHORIZE_URL)?;
    authorize.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", "XboxLive.signin offline_access")
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("prompt", "select_account");

    open::that(authorize.as_str()).context("Не удалось открыть браузер. Проверь браузер по умолчанию.")?;
    status("Войди в Microsoft в открывшемся браузере и подтверди доступ для NEXUS…".into());

    let (code, returned_state) = receive_oauth_redirect(&listener)?;
    if returned_state != state {
        bail!("Ответ Microsoft не прошёл проверку state (возможна подмена OAuth-запроса)");
    }

    status("Обмениваю код Microsoft на токены…".into());
    let response = client.post(TOKEN_URL).form(&[
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
        ("scope", "XboxLive.signin offline_access"),
    ]).send()?;
    let response_status = response.status();
    let token_json: Value = response.json().context("Microsoft вернула некорректный ответ OAuth")?;
    if !response_status.is_success() {
        bail!("Microsoft OAuth: {}", token_json.get("error_description").and_then(Value::as_str)
            .or_else(|| token_json.get("error").and_then(Value::as_str)).unwrap_or("неизвестная ошибка"));
    }
    let oauth: TokenResponse = serde_json::from_value(token_json)
        .context("Microsoft вернула неполный ответ OAuth")?;

    status("Проверяю Xbox Live…".into());
    let xbox_response = client.post(XBOX_USER_AUTH).json(&json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={}", oauth.access_token)
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    })).send()?;
    let xbox_status = xbox_response.status();
    let xbox: Value = xbox_response.json().context("Xbox Live вернул некорректный ответ")?;
    if !xbox_status.is_success() {
        bail!("Xbox Live: {}", xbox.pointer("XErr").and_then(Value::as_u64)
            .map(|v| v.to_string()).unwrap_or_else(|| xbox_status.to_string()));
    }
    let xbox_token = xbox.get("Token").and_then(Value::as_str).context("Xbox Live не вернул токен")?;
    let user_hash = xbox.pointer("DisplayClaims.xui.0.uhs").and_then(Value::as_str)
        .context("Xbox Live не вернул user hash")?;

    status("Проверяю XSTS…".into());
    let xsts_response = client.post(XBOX_XSTS).json(&json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbox_token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    })).send()?;
    let xsts_status = xsts_response.status();
    let xsts: Value = xsts_response.json().context("XSTS вернул некорректный ответ")?;
    if !xsts_status.is_success() {
        let xerr = xsts.get("XErr").and_then(Value::as_u64).map(|v| format!("код {v}"))
            .unwrap_or_else(|| xsts_status.to_string());
        bail!("Xbox XSTS не разрешил вход ({xerr}). Возможно, аккаунту нужен возрастной/региональный доступ или Xbox-профиль.");
    }
    let xsts_token = xsts.get("Token").and_then(Value::as_str).context("XSTS не вернул токен")?;
    let xsts_hash = xsts.pointer("DisplayClaims.xui.0.uhs").and_then(Value::as_str).unwrap_or(user_hash);
    let xuid = xsts.pointer("DisplayClaims.xui.0.xid")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    status("Проверяю Minecraft Java Edition…".into());
    let mc_response = client.post(MC_LOGIN).json(&json!({
        "identityToken": format!("XBL3.0 x={};{}", xsts_hash, xsts_token)
    })).send()?;
    let mc_status = mc_response.status();
    let mc: Value = mc_response.json().context("Minecraft Services вернул некорректный ответ")?;
    if !mc_status.is_success() {
        bail!("Minecraft Services не принял Xbox-токен: {}", mc_status);
    }
    let mc_token = mc.get("access_token").and_then(Value::as_str)
        .context("Minecraft Services не вернул access_token")?;

    let entitlements_response = client.get(MC_ENTITLEMENTS).bearer_auth(mc_token).send()?;
    let entitlement_status = entitlements_response.status();
    let entitlements: Value = entitlements_response.json().context("Не удалось прочитать лицензию Minecraft")?;
    if !entitlement_status.is_success() {
        bail!("Не удалось проверить лицензию Minecraft: {}", entitlement_status);
    }
    let items = entitlements.get("items").and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Minecraft не вернул сведения о лицензии"))?;
    let owns_java = items.iter().any(|item| {
        matches!(item.get("name").and_then(Value::as_str), Some("product_minecraft" | "game_minecraft"))
    });
    if !owns_java {
        bail!("У этого Microsoft-аккаунта не найдена лицензия Minecraft Java Edition. Купи Java Edition или выбери офлайн-режим.");
    }

    let profile: MinecraftProfile = client.get(MC_PROFILE).bearer_auth(mc_token).send()?
        .error_for_status()?.json().context("Не удалось получить игровой профиль Minecraft")?;
    let account = Account {
        id: profile.id.clone(),
        name: profile.name,
        uuid: profile.id,
        kind: AccountKind::Microsoft,
        // Minecraft's modern launch arguments include the Xbox user ID.
        xuid,
    };

    // Secrets go to Windows Credential Manager, never to launcher.json.
    Entry::new(APP_SERVICE, &format!("{}:minecraft", account.id))?
        .set_password(mc_token).context("Не удалось сохранить Minecraft-токен в Windows Credential Manager")?;
    if !oauth.refresh_token.is_empty() {
        Entry::new(APP_SERVICE, &format!("{}:refresh", account.id))?
            .set_password(&oauth.refresh_token).context("Не удалось сохранить refresh-токен")?;
    }
    let expires_at = unix_now()?.saturating_add(oauth.expires_in.max(1800));
    Entry::new(APP_SERVICE, &format!("{}:expires", account.id))?
        .set_password(&expires_at.to_string()).context("Не удалось сохранить срок действия токена")?;

    status(format!("Вход выполнен: {}. Лицензия подтверждена.", account.name));
    Ok(LoginResult { account })
}

fn receive_oauth_redirect(listener: &TcpListener) -> Result<(String, String)> {
    let started = Instant::now();
    let (mut stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if started.elapsed() > Duration::from_secs(300) {
                    bail!("Время ожидания ответа браузера Microsoft истекло. Попробуй войти снова.");
                }
                thread::sleep(Duration::from_millis(150));
            }
            Err(error) => return Err(error).context("Не удалось принять возврат из браузера Microsoft"),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut request = Vec::with_capacity(2048);
    let mut buffer = [0u8; 1024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 { break; }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") || request.len() > 16 * 1024 { break; }
    }
    let request_text = String::from_utf8_lossy(&request);
    let first_line = request_text.lines().next().context("Браузер прислал пустой HTTP-запрос")?;
    let target = first_line.split_whitespace().nth(1).context("Некорректный localhost callback")?;
    let callback = Url::parse(&format!("http://localhost{target}"))?;
    let mut code = None;
    let mut state = None;
    let mut oauth_error = None;
    for (key, value) in callback.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => oauth_error = Some(value.into_owned()),
            "error_description" => oauth_error = Some(value.into_owned()),
            _ => {}
        }
    }

    let (status_line, page) = if code.is_some() {
        ("200 OK", "<h2>Microsoft-вход подтверждён</h2><p>Можно закрыть эту вкладку и вернуться в NEXUS Launcher.</p>")
    } else {
        ("400 Bad Request", "<h2>Не удалось завершить вход</h2><p>Вернитесь в NEXUS Launcher и попробуйте снова.</p>")
    };
    let body = format!("<!doctype html><html><meta charset=\"utf-8\"><title>NEXUS Launcher</title><body style=\"font:16px Segoe UI,Arial;background:#101724;color:#eef;padding:48px\"><h1>NEXUS Launcher</h1>{page}</body></html>");
    let response = format!(
        "HTTP/1.1 {status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(), body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    if let Some(error) = oauth_error { bail!("Microsoft отказала во входе: {error}"); }
    Ok((code.context("Microsoft не вернула authorization code")?, state.context("Microsoft не вернула state")?))
}

pub fn minecraft_token(account_id: &str) -> Result<String> {
    let token_entry = Entry::new(APP_SERVICE, &format!("{}:minecraft", account_id))?;
    let token = token_entry.get_password()
        .context("Токен Microsoft не найден. Войди в аккаунт ещё раз.")?;
    let expiry_entry = Entry::new(APP_SERVICE, &format!("{}:expires", account_id))?;
    let expires_at = expiry_entry.get_password().ok().and_then(|value| value.parse::<u64>().ok());
    if let Some(expiry) = expires_at {
        if unix_now()?.saturating_add(120) < expiry { return Ok(token); }
    }

    let refresh_token = Entry::new(APP_SERVICE, &format!("{}:refresh", account_id))?
        .get_password().context("Срок Microsoft-токена истёк. Войди в аккаунт ещё раз.")?;
    let client_id = std::env::var("NEXUS_MICROSOFT_CLIENT_ID")
        .context("Для обновления Microsoft-токена задай NEXUS_MICROSOFT_CLIENT_ID и войди в аккаунт заново.")?;
    let client = Client::builder().user_agent("NEXUS-Launcher/0.2.0")
        .timeout(Duration::from_secs(45)).build()?;
    let response = client.post(TOKEN_URL).form(&[
        ("client_id", client_id.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("scope", "XboxLive.signin offline_access"),
    ]).send()?;
    let response_status = response.status();
    let payload: Value = response.json().context("Microsoft вернула некорректный ответ при обновлении токена")?;
    if !response_status.is_success() {
        bail!("Microsoft не смогла обновить токен: {}. Войди в аккаунт снова.",
            payload.get("error_description").and_then(Value::as_str)
                .or_else(|| payload.get("error").and_then(Value::as_str)).unwrap_or("неизвестная ошибка"));
    }
    let renewed: TokenResponse = serde_json::from_value(payload)
        .context("Microsoft не вернула новый access token")?;
    let minecraft_access = exchange_access_token(&client, &renewed.access_token)?;
    token_entry.set_password(&minecraft_access)
        .context("Не удалось сохранить обновлённый Minecraft-токен")?;
    if !renewed.refresh_token.is_empty() {
        Entry::new(APP_SERVICE, &format!("{}:refresh", account_id))?.set_password(&renewed.refresh_token)?;
    }
    let expires_at = unix_now()?.saturating_add(renewed.expires_in.max(1800));
    expiry_entry.set_password(&expires_at.to_string())?;
    Ok(minecraft_access)
}

fn unix_now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)
        .context("Системное время раньше Unix epoch")?.as_secs())
}

/// Exchanges a renewed Microsoft OAuth token through Xbox/XSTS into a Minecraft token.
fn exchange_access_token(client: &Client, microsoft_access_token: &str) -> Result<String> {
    let xbox_response = client.post(XBOX_USER_AUTH).json(&json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={microsoft_access_token}")
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    })).send()?;
    let xbox_status = xbox_response.status();
    let xbox: Value = xbox_response.json().context("Xbox Live вернул некорректный ответ при обновлении")?;
    if !xbox_status.is_success() {
        bail!("Xbox Live не принял обновлённый токен: {}", xbox.pointer("XErr")
            .and_then(Value::as_u64).map(|value| value.to_string()).unwrap_or_else(|| xbox_status.to_string()));
    }
    let xbox_token = xbox.get("Token").and_then(Value::as_str).context("Xbox Live не вернул токен")?;
    let user_hash = xbox.pointer("DisplayClaims.xui.0.uhs").and_then(Value::as_str)
        .context("Xbox Live не вернул user hash")?;

    let xsts_response = client.post(XBOX_XSTS).json(&json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [xbox_token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    })).send()?;
    let xsts_status = xsts_response.status();
    let xsts: Value = xsts_response.json().context("XSTS вернул некорректный ответ")?;
    if !xsts_status.is_success() {
        bail!("Xbox XSTS не разрешил обновить вход ({}). Войди в Microsoft снова.",
            xsts.get("XErr").and_then(Value::as_u64).map(|value| format!("код {value}"))
                .unwrap_or_else(|| xsts_status.to_string()));
    }
    let xsts_token = xsts.get("Token").and_then(Value::as_str).context("XSTS не вернул токен")?;
    let xsts_hash = xsts.pointer("DisplayClaims.xui.0.uhs").and_then(Value::as_str).unwrap_or(user_hash);
    let mc_response = client.post(MC_LOGIN).json(&json!({
        "identityToken": format!("XBL3.0 x={};{}", xsts_hash, xsts_token)
    })).send()?;
    let mc_status = mc_response.status();
    let mc: Value = mc_response.json().context("Minecraft Services вернул некорректный ответ")?;
    if !mc_status.is_success() { bail!("Minecraft Services отклонил обновлённый токен: {mc_status}"); }
    let minecraft_token = mc.get("access_token").and_then(Value::as_str)
        .context("Minecraft Services не вернул access_token")?.to_owned();
    Ok(minecraft_token)
}
