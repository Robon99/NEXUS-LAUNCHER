use anyhow::{anyhow, bail, Context, Result};
use md5::Md5;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use zip::ZipArchive;

use crate::config::{AppPaths, Profile};

const CURSEFORGE_API: &str = "https://api.curseforge.com/v1";

#[derive(Debug, Deserialize)]
struct MrPackIndex {
    name: String,
    #[serde(default, rename = "versionId")]
    version_id: String,
    #[serde(default)]
    dependencies: HashMap<String, String>,
    #[serde(default)]
    files: Vec<PackFile>,
}
#[derive(Debug, Deserialize)]
struct PackFile {
    path: String,
    #[serde(default)]
    hashes: HashMap<String, String>,
    #[serde(default)]
    downloads: Vec<String>,
    #[serde(default)]
    env: PackEnvironment,
}
#[derive(Debug, Deserialize)]
struct PackEnvironment {
    #[serde(default = "required")]
    client: String,
}
impl Default for PackEnvironment {
    fn default() -> Self { Self { client: required() } }
}
fn required() -> String { "required".to_owned() }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeManifest {
    name: String,
    #[serde(default)]
    version: String,
    minecraft: CurseForgeMinecraft,
    #[serde(default)]
    files: Vec<CurseForgeManifestFile>,
    #[serde(default = "default_overrides")]
    overrides: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeMinecraft {
    version: String,
    #[serde(default)]
    mod_loaders: Vec<CurseForgeLoader>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeLoader {
    id: String,
    #[serde(default)]
    primary: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeManifestFile {
    project_id: i64,
    file_id: i64,
    #[serde(default = "yes")]
    required: bool,
}
fn yes() -> bool { true }
fn default_overrides() -> String { "overrides".to_owned() }

#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> { data: T }
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurseForgeModpack {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: u64,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeModDetail {
    #[serde(default)]
    main_file_id: Option<i64>,
    #[serde(default)]
    latest_files: Vec<CurseForgeFile>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeFile {
    id: i64,
    file_name: String,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    hashes: Vec<CurseForgeHash>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CurseForgeHash {
    value: String,
    algo: u32,
}

pub fn import_mrpack<F>(archive_path: &Path, paths: &AppPaths, status: F) -> Result<Profile>
where F: Fn(String) {
    status("Читаю Modrinth-сборку…".into());
    let file = File::open(archive_path).context("Не удалось открыть архив сборки")?;
    let mut archive = ZipArchive::new(file).context("Это не ZIP/.mrpack архив")?;
    let index = {
        let mut index_file = archive.by_name("modrinth.index.json")
            .context("В архиве нет modrinth.index.json")?;
        let mut text = String::new();
        index_file.read_to_string(&mut text)?;
        serde_json::from_str::<MrPackIndex>(&text).context("Файл modrinth.index.json повреждён")?
    };
    let mc_version = index.dependencies.get("minecraft").cloned().unwrap_or_else(|| "latest-release".to_owned());
    let loader_entry = ["fabric-loader", "forge", "neoforge", "quilt-loader"]
        .iter().find_map(|key| index.dependencies.get(*key).map(|v| (*key, v.clone())));
    let (loader, loader_version) = match loader_entry {
        Some(("fabric-loader", v)) => ("Fabric".to_owned(), v),
        Some(("forge", v)) => ("Forge".to_owned(), v),
        Some(("neoforge", v)) => ("NeoForge".to_owned(), v),
        Some(("quilt-loader", v)) => ("Quilt".to_owned(), v),
        _ => ("Vanilla".to_owned(), String::new()),
    };
    let mut profile = paths.make_profile(&index.name, &mc_version, "Player")?;
    profile.loader = loader;
    profile.loader_version = loader_version;
    let game_dir = PathBuf::from(&profile.game_dir);
    let client = Client::builder().user_agent("NEXUS-Launcher/0.2.0 (Modrinth pack importer)")
        .timeout(Duration::from_secs(180)).build()?;

    let files: Vec<_> = index.files.into_iter().filter(|f| f.env.client != "unsupported" && f.env.client != "optional").collect();
    let total = files.len();
    for (i, pack_file) in files.iter().enumerate() {
        let relative = safe_relative_path(&pack_file.path)?;
        let target = game_dir.join(relative);
        if target.exists() { continue; }
        if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
        let mut last_error: Option<anyhow::Error> = None;
        let mut downloaded = false;
        for url in pack_file.downloads.iter().filter(|url| url.starts_with("https://")) {
            let attempt = (|| -> Result<()> {
                let bytes = client.get(url).send()?.error_for_status()
                    .with_context(|| format!("Не удалось скачать файл сборки {}", pack_file.path))?.bytes()?;
                verify_pack_hashes(&bytes, &pack_file.hashes, &pack_file.path)?;
                atomic_write(&target, &bytes)?;
                Ok(())
            })();
            match attempt {
                Ok(()) => { downloaded = true; break; }
                Err(error) => last_error = Some(error),
            }
        }
        if !downloaded {
            if pack_file.downloads.is_empty() {
                bail!("У файла сборки нет URL: {}", pack_file.path);
            }
            if pack_file.downloads.iter().all(|url| !url.starts_with("https://")) {
                bail!("Для файла сборки нет безопасной HTTPS-ссылки: {}", pack_file.path);
            }
            return Err(last_error.unwrap_or_else(|| anyhow!("Не удалось скачать файл сборки {}", pack_file.path)));
        }
        if (i + 1) % 10 == 0 || i + 1 == total {
            status(format!("Импортирую файлы сборки: {}/{}", i + 1, total));
        }
    }

    status("Распаковываю overrides…".into());
    extract_override_prefixes(&mut archive, &game_dir, &["overrides/", "client-overrides/"])?;
    save_profile(paths, &profile)?;
    let imported_version = if index.version_id.trim().is_empty() { "без указанной версии".to_owned() }
        else { format!("версия {}", index.version_id) };
    status(format!("Сборка «{}» ({}) импортирована. Загрузчик: {} {}.",
        index.name, imported_version, profile.loader, profile.loader_version));
    Ok(profile)
}

/// Imports CurseForge's manifest.json archive. The API key is required because the
/// manifest contains project/file IDs, not reliable public download URLs.
pub fn import_curseforge_zip<F>(archive_path: &Path, paths: &AppPaths, api_key: &str, status: F) -> Result<Profile>
where F: Fn(String) {
    if api_key.trim().is_empty() {
        bail!("Для импорта CurseForge нужен API-ключ: создай приложение в https://console.curseforge.com/ и задай NEXUS_CURSEFORGE_API_KEY.");
    }
    status("Читаю CurseForge manifest.json…".into());
    let file = File::open(archive_path).context("Не удалось открыть CurseForge ZIP")?;
    let mut archive = ZipArchive::new(file).context("Это не ZIP-архив CurseForge")?;
    let manifest = {
        let mut entry = archive.by_name("manifest.json")
            .context("В ZIP нет manifest.json — это не CurseForge modpack")?;
        let mut body = String::new();
        entry.read_to_string(&mut body)?;
        serde_json::from_str::<CurseForgeManifest>(&body).context("manifest.json повреждён или имеет неизвестную структуру")?
    };
    let primary = manifest.minecraft.mod_loaders.iter().find(|loader| loader.primary)
        .or_else(|| manifest.minecraft.mod_loaders.first());
    let (loader, loader_version) = primary.map(|loader| parse_curseforge_loader(&loader.id))
        .unwrap_or_else(|| ("Vanilla".to_owned(), String::new()));
    let mut profile = paths.make_profile(&manifest.name, &manifest.minecraft.version, "Player")?;
    profile.loader = loader;
    profile.loader_version = loader_version;
    let game_dir = PathBuf::from(&profile.game_dir);

    let client = Client::builder().user_agent("NEXUS-Launcher/0.2.0 (CurseForge modpack importer)")
        .timeout(Duration::from_secs(180)).build()?;
    let api = CurseForgeApi::new(client.clone(), api_key);
    let required_count = manifest.files.iter().filter(|file| file.required).count();
    status(format!("Загружаю моды CurseForge: 0/{}…", required_count));
    let mut completed = 0usize;
    for file in &manifest.files {
        // Optional files are intentionally left out; the user can add them later.
        // This is explicit because CurseForge marks files that may require manual selection.
        if !file.required { continue; }
        let file_info = api.get_file(file.project_id, file.file_id)
            .with_context(|| format!("Не удалось получить файл CurseForge {}/{}", file.project_id, file.file_id))?;
        let url = match file_info.download_url.clone() {
            Some(url) if url.starts_with("https://") => url,
            _ => match api.download_url(file.project_id, file.file_id) {
                Ok(url) if url.starts_with("https://") => url,
                Ok(_) => {
                    if file.required { bail!("CurseForge не предоставил HTTPS-ссылку для {}. Автор мог ограничить распространение файла.", file_info.file_name); }
                    continue;
                }
                Err(error) if !file.required => { status(format!("Пропускаю необязательный файл {}: {}", file.file_id, error)); continue; }
                Err(error) => return Err(error).with_context(|| format!("Не удалось получить ссылку на {}", file_info.file_name)),
            }
        };
        let relative = safe_relative_path(&format!("mods/{}", file_info.file_name))?;
        let target = game_dir.join(relative);
        if !target.exists() {
            let bytes = client.get(&url).send()?.error_for_status()
                .with_context(|| format!("Не удалось скачать CurseForge-файл {}", file_info.file_name))?.bytes()?;
            verify_curseforge_hashes(&bytes, &file_info.hashes, &file_info.file_name)?;
            atomic_write(&target, &bytes)?;
        }
        completed += 1;
        if completed % 5 == 0 || completed == required_count {
            status(format!("Загружаю моды CurseForge: {}/{}", completed, required_count));
        }
    }

    let overrides_prefix = format!("{}/", manifest.overrides.trim_matches('/'));
    status("Распаковываю CurseForge overrides…".into());
    extract_override_prefixes(&mut archive, &game_dir, &[overrides_prefix.as_str()])?;
    save_profile(paths, &profile)?;
    status(format!("CurseForge-сборка «{}» ({}). Импортировано {} файлов, загрузчик {} {}.",
        manifest.name, if manifest.version.trim().is_empty() { "версия не указана" } else { &manifest.version }, completed, profile.loader, profile.loader_version));
    Ok(profile)
}

fn parse_curseforge_loader(value: &str) -> (String, String) {
    let (kind, version) = value.split_once('-').unwrap_or((value, ""));
    let label = match kind.to_ascii_lowercase().as_str() {
        "fabric" => "Fabric",
        "forge" => "Forge",
        "neoforge" => "NeoForge",
        "quilt" => "Quilt",
        _ => "Vanilla",
    };
    (label.to_owned(), version.to_owned())
}

fn save_profile(paths: &AppPaths, profile: &Profile) -> Result<()> {
    let profile_root = paths.instances.join(&profile.id);
    fs::write(profile_root.join("instance.json"), serde_json::to_vec_pretty(profile)?)?;
    Ok(())
}

fn verify_pack_hashes(bytes: &[u8], hashes: &HashMap<String, String>, path: &str) -> Result<()> {
    if let Some(expected) = hashes.get("sha1") {
        let actual = hex::encode(Sha1::digest(bytes));
        if !actual.eq_ignore_ascii_case(expected) { bail!("SHA-1 не совпал для файла {}", path); }
    }
    if let Some(expected) = hashes.get("sha512") {
        let actual = hex::encode(Sha512::digest(bytes));
        if !actual.eq_ignore_ascii_case(expected) { bail!("SHA-512 не совпал для файла {}", path); }
    }
    Ok(())
}

fn verify_curseforge_hashes(bytes: &[u8], hashes: &[CurseForgeHash], file_name: &str) -> Result<()> {
    for hash in hashes {
        match hash.algo {
            1 => {
                let actual = hex::encode(Sha1::digest(bytes));
                if !actual.eq_ignore_ascii_case(&hash.value) { bail!("SHA-1 не совпал для CurseForge-файла {}", file_name); }
            }
            2 => {
                let actual = format!("{:x}", Md5::digest(bytes));
                if !actual.eq_ignore_ascii_case(&hash.value) { bail!("MD5 не совпал для CurseForge-файла {}", file_name); }
            }
            _ => {}
        }
    }
    Ok(())
}

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
    let temp = target.with_extension("nexus-download.tmp");
    fs::write(&temp, bytes)?;
    fs::rename(&temp, target).or_else(|_| {
        fs::copy(&temp, target)?;
        let _ = fs::remove_file(&temp);
        Ok::<(), std::io::Error>(())
    })?;
    Ok(())
}

fn extract_override_prefixes(archive: &mut ZipArchive<File>, game_dir: &Path, prefixes: &[&str]) -> Result<()> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let entry_name = entry.name().replace('\\', "/");
        let relative_name = prefixes.iter().find_map(|prefix| entry_name.strip_prefix(prefix));
        let Some(relative_name) = relative_name else { continue; };
        if relative_name.is_empty() { continue; }
        let relative = safe_relative_path(relative_name)?;
        let destination = game_dir.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(destination)?;
        } else {
            if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
            let mut out = File::create(destination)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}

struct CurseForgeApi {
    client: Client,
    api_key: String,
}
impl CurseForgeApi {
    fn new(client: Client, api_key: &str) -> Self { Self { client, api_key: api_key.to_owned() } }

    pub fn search_modpacks(&self, query: &str) -> Result<Vec<CurseForgeModpack>> {
        if query.trim().is_empty() { bail!("Введи поисковый запрос CurseForge"); }
        let response = self.client.get(format!("{CURSEFORGE_API}/mods/search"))
            .header("x-api-key", &self.api_key)
            .query(&[
                ("gameId", "432"),
                ("classId", "4471"),
                ("searchFilter", query),
                ("pageSize", "12"),
                ("sortField", "2"),
                ("sortOrder", "desc"),
                ("index", "0"),
            ]).send()?.error_for_status().context("CurseForge API отказал в поиске. Проверь API-ключ.")?;
        let result: ApiEnvelope<Vec<CurseForgeModpack>> = response.json().context("CurseForge вернул некорректный ответ поиска")?;
        Ok(result.data)
    }

    fn get_file(&self, project_id: i64, file_id: i64) -> Result<CurseForgeFile> {
        let response = self.client.get(format!("{CURSEFORGE_API}/mods/{project_id}/files/{file_id}"))
            .header("x-api-key", &self.api_key).send()?.error_for_status()
            .context("CurseForge API не нашёл указанный файл")?;
        let result: ApiEnvelope<CurseForgeFile> = response.json()?;
        Ok(result.data)
    }

    fn download_url(&self, project_id: i64, file_id: i64) -> Result<String> {
        let response = self.client.get(format!("{CURSEFORGE_API}/mods/{project_id}/files/{file_id}/download-url"))
            .header("x-api-key", &self.api_key).send()?.error_for_status()
            .context("CurseForge API не разрешил ссылку для скачивания")?;
        let result: ApiEnvelope<Option<String>> = response.json()?;
        result.data.context("CurseForge не предоставил ссылку на скачивание (возможно, автор запретил стороннюю загрузку)")
    }

    fn download_modpack(&self, mod_id: i64, paths: &AppPaths) -> Result<PathBuf> {
        let response = self.client.get(format!("{CURSEFORGE_API}/mods/{mod_id}"))
            .header("x-api-key", &self.api_key).send()?.error_for_status()
            .context("CurseForge не нашёл сборку")?;
        let detail: ApiEnvelope<CurseForgeModDetail> = response.json()?;
        let file_id = detail.data.main_file_id
            .or_else(|| detail.data.latest_files.first().map(|file| file.id))
            .context("У этой CurseForge-сборки нет доступного файла")?;
        let download_url = self.download_url(mod_id, file_id)?;
        if !download_url.starts_with("https://") { bail!("CurseForge вернул небезопасную ссылку скачивания"); }
        let bytes = self.client.get(&download_url).send()?.error_for_status()
            .context("Не удалось скачать ZIP-сборку с CurseForge CDN")?.bytes()?;
        let cache_dir = paths.base.join("cache").join("curseforge");
        fs::create_dir_all(&cache_dir)?;
        let target = cache_dir.join(format!("{mod_id}-{file_id}.zip"));
        atomic_write(&target, &bytes)?;
        Ok(target)
    }
}

pub fn search_curseforge_modpacks(query: &str, api_key: &str) -> Result<Vec<CurseForgeModpack>> {
    if api_key.trim().is_empty() {
        bail!("Для каталога CurseForge нужен API-ключ. Создай приложение на https://console.curseforge.com/ и задай NEXUS_CURSEFORGE_API_KEY.");
    }
    let client = Client::builder().user_agent("NEXUS-Launcher/0.2.0")
        .timeout(Duration::from_secs(45)).build()?;
    CurseForgeApi::new(client, api_key).search_modpacks(query)
}

pub fn download_curseforge_modpack<F>(mod_id: i64, paths: &AppPaths, api_key: &str, status: F) -> Result<Profile>
where F: Fn(String) {
    if api_key.trim().is_empty() { bail!("Не задан NEXUS_CURSEFORGE_API_KEY"); }
    let client = Client::builder().user_agent("NEXUS-Launcher/0.2.0")
        .timeout(Duration::from_secs(180)).build()?;
    let api = CurseForgeApi::new(client, api_key);
    status(format!("Получаю ZIP сборки CurseForge #{mod_id}…"));
    let archive = api.download_modpack(mod_id, paths)?;
    import_curseforge_zip(&archive, paths, api_key, status)
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let normalized = value.replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.trim().is_empty() || normalized.contains('\0') || path.is_absolute() {
        bail!("Недопустимый путь в архиве: {}", value);
    }
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part_text = part.to_string_lossy();
                if part_text.contains(':') { bail!("Архив содержит небезопасный путь: {}", value); }
                clean.push(part);
            }
            _ => bail!("Архив содержит небезопасный путь: {}", value),
        }
    }
    if clean.as_os_str().is_empty() { bail!("Недопустимый путь в архиве: {}", value); }
    Ok(clean)
}
