use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use uuid::Uuid;

pub const APP_SERVICE: &str = "nexus-launcher";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccountKind {
    Offline,
    Microsoft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub uuid: String,
    pub kind: AccountKind,
    #[serde(default)]
    pub xuid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    #[serde(default)]
    pub loader_version: String,
    pub game_dir: String,
    pub nickname: String,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_memory")]
    pub memory_gb: u8,
    #[serde(default = "default_java")]
    pub java_path: String,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub selected_profile: Option<String>,
}

fn default_memory() -> u8 { 4 }
fn default_java() -> String { "auto".to_owned() }

impl Default for Settings {
    fn default() -> Self {
        Self {
            memory_gb: default_memory(),
            java_path: default_java(),
            profiles: Vec::new(),
            accounts: Vec::new(),
            selected_profile: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub base: PathBuf,
    pub instances: PathBuf,
    pub shared: PathBuf,
    pub config_file: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("com", "Nexus", "NexusLauncher")
            .context("Не удалось определить папку данных приложения")?;
        let base = project.data_local_dir().to_path_buf();
        let paths = Self {
            instances: base.join("instances"),
            shared: base.join("shared"),
            config_file: base.join("launcher.json"),
            base,
        };
        fs::create_dir_all(&paths.instances)?;
        fs::create_dir_all(paths.shared.join("versions"))?;
        fs::create_dir_all(paths.shared.join("libraries"))?;
        fs::create_dir_all(paths.shared.join("assets"))?;
        Ok(paths)
    }

    pub fn load_settings(&self) -> Result<Settings> {
        if !self.config_file.exists() {
            return Ok(Settings::default());
        }
        let data = fs::read(&self.config_file).context("Не удалось прочитать настройки")?;
        Ok(serde_json::from_slice(&data).context("Файл настроек повреждён")?)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        fs::create_dir_all(&self.base)?;
        let temp = self.config_file.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(settings)?;
        fs::write(&temp, data)?;
        if self.config_file.exists() {
            let _ = fs::remove_file(&self.config_file);
        }
        fs::rename(temp, &self.config_file).context("Не удалось сохранить настройки")?;
        Ok(())
    }

    pub fn make_profile(&self, name: &str, version: &str, nickname: &str) -> Result<Profile> {
        let id = Uuid::new_v4().to_string();
        let profile_root = self.instances.join(&id);
        let game_dir = profile_root.join("minecraft");
        fs::create_dir_all(game_dir.join("mods"))?;
        fs::create_dir_all(game_dir.join("saves"))?;
        fs::create_dir_all(game_dir.join("resourcepacks"))?;
        fs::create_dir_all(game_dir.join("shaderpacks"))?;
        let profile = Profile {
            id: id.clone(),
            name: clean_label(name, "Моя сборка"),
            mc_version: clean_version(version),
            loader: "Vanilla".to_owned(),
            loader_version: String::new(),
            game_dir: game_dir.to_string_lossy().into_owned(),
            nickname: clean_nickname(nickname),
            account_id: None,
        };
        let instance_json = profile_root.join("instance.json");
        fs::write(instance_json, serde_json::to_vec_pretty(&profile)?)?;
        Ok(profile)
    }
}

pub fn clean_label(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() { return fallback.to_owned(); }
    value.chars().filter(|c| !c.is_control()).take(48).collect()
}

pub fn clean_version(value: &str) -> String {
    let v = value.trim();
    if v.is_empty() { "latest-release".to_owned() }
    else if v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c)) { v.to_owned() }
    else { "latest-release".to_owned() }
}

pub fn clean_nickname(value: &str) -> String {
    let filtered: String = value.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').take(16).collect();
    if filtered.is_empty() { "Player".to_owned() } else { filtered }
}
