use anyhow::{anyhow, bail, Context, Result};
use md5::Md5;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::{AtomicUsize, Ordering}, Arc, Mutex},
    thread,
    time::Duration,
};
use uuid::Uuid;
use url::Url;
use zip::ZipArchive;

use crate::config::{AccountKind, AppPaths, Profile, Settings};
use crate::accounts::microsoft;

const VERSION_MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const USER_AGENT: &str = "NEXUS-Launcher/0.2.0 (Minecraft Java launcher)";

#[derive(Debug, Deserialize)]
struct Manifest {
    latest: LatestVersions,
    versions: Vec<ManifestVersion>,
}
#[derive(Debug, Deserialize)]
struct LatestVersions { release: String, snapshot: String }
#[derive(Debug, Deserialize)]
struct ManifestVersion { id: String, url: String, #[serde(default, rename = "type")] version_type: String }

#[derive(Debug, Deserialize)]
struct VersionJson {
    id: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    #[serde(default)]
    arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments", default)]
    minecraft_arguments: Option<String>,
    downloads: VersionDownloads,
    #[serde(rename = "assetIndex", default)]
    asset_index: Option<AssetIndex>,
    #[serde(default)]
    assets: Option<String>,
    #[serde(default)]
    libraries: Vec<Library>,
    #[serde(default)]
    logging: Option<LoggingInfo>,
    #[serde(rename = "javaVersion", default)]
    java_version: Option<JavaVersion>,
}
#[derive(Debug, Deserialize)]
struct LoggingInfo { client: Option<ClientLogging> }
#[derive(Debug, Deserialize)]
struct ClientLogging { argument: String, file: LoggingFile }
#[derive(Debug, Deserialize)]
struct LoggingFile {
    id: String,
    url: String,
    #[serde(default)] sha1: Option<String>,
    #[serde(default)] size: Option<u64>,
}
#[derive(Debug, Deserialize)]
struct JavaVersion { #[serde(rename = "majorVersion")] major_version: Option<u32> }
#[derive(Debug, Deserialize)]
struct VersionDownloads { client: DownloadFile }
#[derive(Debug, Clone, Deserialize)]
struct DownloadFile {
    url: String,
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    path: Option<String>,
}
#[derive(Debug, Deserialize)]
struct AssetIndex {
    id: String,
    url: String,
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}
#[derive(Debug, Deserialize)]
struct AssetIndexContent {
    objects: HashMap<String, AssetObject>,
    #[serde(default, rename = "virtual")]
    virtual_assets: bool,
    #[serde(default, rename = "map_to_resources")]
    map_to_resources: bool,
}
#[derive(Debug, Clone, Deserialize)]
struct AssetObject { hash: String, #[serde(default)] size: u64 }

#[derive(Debug, Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<ArgumentItem>,
    #[serde(default)]
    jvm: Vec<ArgumentItem>,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentItem {
    Plain(String),
    Conditional(RuleArgument),
}
#[derive(Debug, Deserialize)]
struct RuleArgument { rules: Vec<Rule>, value: ArgumentValue }
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ArgumentValue { One(String), Many(Vec<String>) }
#[derive(Debug, Deserialize)]
struct Rule {
    action: String,
    #[serde(default)]
    os: Option<OsRule>,
    #[serde(default)]
    features: Option<HashMap<String, bool>>,
}
#[derive(Debug, Deserialize)]
struct OsRule { name: Option<String> }

#[derive(Debug, Deserialize)]
struct Library {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    rules: Option<Vec<Rule>>,
    #[serde(default)]
    downloads: Option<LibraryDownloads>,
    #[serde(default)]
    natives: Option<HashMap<String, String>>,
}
#[derive(Debug, Deserialize)]
struct LibraryDownloads {
    #[serde(default)]
    artifact: Option<DownloadFile>,
    #[serde(default)]
    classifiers: Option<HashMap<String, DownloadFile>>,
}

pub fn install_and_launch<F>(profile: &Profile, settings: &Settings, paths: &AppPaths, status: F) -> Result<()>
where F: Fn(String) + Send + Sync + 'static {
    let status: Arc<dyn Fn(String) + Send + Sync> = Arc::new(status);
    let client = Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(240)).build()?;
    (status)("Получаю официальный список версий Minecraft…".into());

    let manifest: Manifest = client.get(VERSION_MANIFEST).send()?.error_for_status()?.json()
        .context("Не удалось загрузить официальный манифест Minecraft")?;
    let wanted = match profile.mc_version.as_str() {
        "latest-release" => manifest.latest.release.clone(),
        "latest-snapshot" => manifest.latest.snapshot.clone(),
        other => other.to_owned(),
    };
    let version_meta = manifest.versions.iter().find(|v| v.id == wanted)
        .ok_or_else(|| anyhow!("Версия Minecraft {} отсутствует в официальном манифесте", wanted))?;
    let version_bytes = client.get(&version_meta.url).send()?.error_for_status()?.bytes()
        .context("Не удалось загрузить метаданные версии")?;
    let mut version: VersionJson = serde_json::from_slice(&version_bytes)
        .context("Метаданные версии Minecraft имеют неизвестный формат")?;

    let required_java = version.java_version.as_ref().and_then(|v| v.major_version).unwrap_or(8);
    let java_path = resolve_java(&settings.java_path, required_java, paths, status.clone())?;

    let game_dir = PathBuf::from(&profile.game_dir);
    fs::create_dir_all(&game_dir)?;
    let loader_json_path = if profile.loader.eq_ignore_ascii_case("Vanilla") {
        None
    } else {
        (status)(format!("Подготавливаю {} {} для Minecraft {}…", profile.loader, profile.loader_version, wanted));
        Some(install_or_fetch_loader(
            &client, profile, &wanted, &java_path, &game_dir, paths, status.clone()
        )?)
    };
    if let Some(loader_path) = loader_json_path {
        let loader_value: serde_json::Value = serde_json::from_slice(&fs::read(&loader_path)?)
            .context("Метаданные модлоадера повреждены")?;
        merge_loader_metadata(&mut version, &loader_value, &wanted)?;
    }

    let version_dir = paths.shared.join("versions").join(&wanted);
    fs::create_dir_all(&version_dir)?;
    let version_json_path = version_dir.join(format!("{}.json", wanted));
    fs::write(&version_json_path, &version_bytes)?;

    let client_jar = version_dir.join(format!("{}.jar", wanted));
    (status)(format!("Проверяю клиент Minecraft {}…", wanted));
    download_verified_with_size(
        &client,
        &version.downloads.client.url,
        &client_jar,
        version.downloads.client.sha1.as_deref(),
        version.downloads.client.size,
    )?;

    let natives_dir = paths.shared.join("natives").join(&version.id);
    fs::create_dir_all(&natives_dir)?;
    let mut classpath = Vec::<PathBuf>::new();
    for library in &version.libraries {
        if !rules_allow(library.rules.as_deref()) { continue; }
        let Some(relative_path) = library_relative_path(library)? else { continue; };
        let local_candidate = game_dir.join("libraries").join(&relative_path);
        let shared_candidate = paths.shared.join("libraries").join(&relative_path);
        let target = if local_candidate.exists() { local_candidate.clone() }
            else { shared_candidate.clone() };

        if let Some(downloads) = &library.downloads {
            if let Some(artifact) = &downloads.artifact {
                let actual_target = if target.exists() { target.clone() } else { shared_candidate.clone() };
                download_verified_with_size(&client, &artifact.url, &actual_target, artifact.sha1.as_deref(), artifact.size)?;
            }
            if let (Some(natives), Some(classifiers)) = (&library.natives, &downloads.classifiers) {
                if let Some(template) = natives.get(platform_key()) {
                    let classifier_name = template.replace("${arch}", if usize::BITS == 64 { "64" } else { "32" });
                    if let Some(native_file) = classifiers.get(&classifier_name) {
                        let rel = safe_library_path(native_file.path.as_deref().unwrap_or("native.jar"))?;
                        let native_target = paths.shared.join("libraries").join(rel);
                        download_verified_with_size(&client, &native_file.url, &native_target, native_file.sha1.as_deref(), native_file.size)?;
                        extract_natives(&native_target, &natives_dir)?;
                    }
                }
            }
        }
        // Loader metadata from Fabric/Quilt can describe Maven artifacts with a
        // repository URL but without a `downloads.artifact` object. Fetch those too.
        if !local_candidate.exists() && !shared_candidate.exists() {
            let base_url = library.url.as_deref().unwrap_or("https://libraries.minecraft.net/");
            let url = format!("{}/{}", base_url.trim_end_matches('/'), relative_path.to_string_lossy().replace('\\', "/"));
            if !url.starts_with("https://") { bail!("Для библиотеки {} указана небезопасная ссылка", relative_path.display()); }
            download_verified(&client, &url, &shared_candidate, None)?;
        }

        let resolved = if local_candidate.exists() { local_candidate }
            else if shared_candidate.exists() { shared_candidate }
            else { target };
        if resolved.exists() && !classpath.contains(&resolved) { classpath.push(resolved); }
    }
    classpath.push(client_jar.clone());

    let assets_dir = paths.shared.join("assets");
    let mut asset_index_id = version.assets.clone().unwrap_or_else(|| "legacy".to_owned());
    if let Some(index) = &version.asset_index {
        asset_index_id = index.id.clone();
        (status)(format!("Загружаю индекс ресурсов {}…", index.id));
        let index_path = assets_dir.join("indexes").join(format!("{}.json", index.id));
        download_verified_with_size(&client, &index.url, &index_path, index.sha1.as_deref(), index.size)?;
        let index_content: AssetIndexContent = serde_json::from_slice(&fs::read(&index_path)?)
            .context("Индекс ресурсов Minecraft повреждён")?;
        download_assets(&client, &assets_dir, &game_dir, &index.id, index_content, status.clone())?;
    }
    let account = settings.accounts.iter().find(|a| profile.account_id.as_deref() == Some(a.id.as_str()));
    let (player_name, player_uuid, access_token, user_type, xuid) = match account {
        Some(account) if account.kind == AccountKind::Microsoft => (
            account.name.clone(), account.uuid.replace('-', ""), microsoft::minecraft_token(&account.id)?, "msa".to_owned(), account.xuid.clone().unwrap_or_default()
        ),
        _ => {
            let name = crate::config::clean_nickname(&profile.nickname);
            (name.clone(), offline_uuid(&name), "0".to_owned(), "legacy".to_owned(), String::new())
        }
    };

    let cp = classpath.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join(if cfg!(windows) { ";" } else { ":" });
    let logging = if let Some(client_logging) = version.logging.as_ref().and_then(|l| l.client.as_ref()) {
        let path = assets_dir.join("log_configs").join(&client_logging.file.id);
        download_verified_with_size(&client, &client_logging.file.url, &path, client_logging.file.sha1.as_deref(), client_logging.file.size)?;
        Some((client_logging.argument.clone(), path))
    } else { None };
    let mut replacements = build_replacements(
        &player_name, &player_uuid, &access_token, &user_type, &xuid,
        &version.id, &version_meta.version_type, &game_dir, &assets_dir, &asset_index_id, &natives_dir, &paths.shared, &cp,
    );
    if let Some((_, path)) = &logging {
        replacements.insert("${path}".to_owned(), path.to_string_lossy().into_owned());
    }
    let mut jvm_args = Vec::<String>::new();
    let mut game_args = Vec::<String>::new();
    if let Some(arguments) = &version.arguments {
        jvm_args = flatten_arguments(&arguments.jvm);
        game_args = flatten_arguments(&arguments.game);
    } else if let Some(old_args) = &version.minecraft_arguments {
        game_args = old_args.split_whitespace().map(ToOwned::to_owned).collect();
    }
    jvm_args = jvm_args.into_iter().map(|s| replace_tokens(&s, &replacements)).collect();
    if let Some((argument, _)) = &logging {
        let resolved = replace_tokens(argument, &replacements);
        if !jvm_args.contains(&resolved) { jvm_args.push(resolved); }
    }
    game_args = game_args.into_iter().map(|s| replace_tokens(&s, &replacements)).collect();

    let memory = settings.memory_gb.clamp(2, 32);
    if !jvm_args.iter().any(|a| a.starts_with("-Xmx")) { jvm_args.insert(0, format!("-Xmx{}G", memory)); }
    if !jvm_args.iter().any(|a| a.starts_with("-Xms")) { jvm_args.insert(0, "-Xms512M".to_owned()); }
    if !jvm_args.iter().any(|a| a.starts_with("-Djava.library.path=")) {
        jvm_args.push(format!("-Djava.library.path={}", natives_dir.to_string_lossy()));
    }
    if !jvm_args.iter().any(|a| a == "-cp" || a == "-classpath") {
        jvm_args.push("-cp".to_owned());
        jvm_args.push(cp);
    }
    if !game_args.iter().any(|a| a == "--gameDir") {
        game_args.push("--gameDir".to_owned());
        game_args.push(game_dir.to_string_lossy().to_string());
    }

    (status)(format!("Запускаю Minecraft {} ({})…", wanted, profile.loader));
    let log_path = game_dir.join("nexus-launch.log");
    let stdout = File::create(&log_path).context("Не удалось создать лог запуска")?;
    let stderr = stdout.try_clone()?;
    let mut command = Command::new(&java_path);
    command.args(&jvm_args)
        .arg(&version.main_class)
        .args(&game_args)
        .current_dir(&game_dir)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    command.spawn().with_context(|| format!("Не удалось запустить Java через '{}'. Проверь путь к Java.", java_path.display()))?;
    fs::write(game_dir.join(".nexus-installed-version"), format!("{}\n{}\n{}", wanted, profile.loader, profile.loader_version))?;
    (status)(format!("Minecraft {} ({}) запущен. Лог: {}", wanted, profile.loader, log_path.display()));
    Ok(())
}

fn install_or_fetch_loader(
    client: &Client,
    profile: &Profile,
    minecraft_version: &str,
    java_path: &Path,
    game_dir: &Path,
    paths: &AppPaths,
    status: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<PathBuf> {
    match profile.loader.to_ascii_lowercase().as_str() {
        "fabric" => fetch_loader_profile(client, "fabric", minecraft_version, &profile.loader_version, game_dir),
        "quilt" => fetch_loader_profile(client, "quilt", minecraft_version, &profile.loader_version, game_dir),
        "forge" | "neoforge" => {
            install_forge_family(client, profile, minecraft_version, java_path, game_dir, paths, status)?
        }
        other => bail!("Неизвестный загрузчик Minecraft: {other}"),
    }
}

fn validate_loader_version(loader_version: &str) -> Result<()> {
    if loader_version.trim().is_empty() {
        bail!("В профиле не сохранена версия загрузчика. Импортируй modpack заново.");
    }
    if loader_version.len() > 80 || !loader_version.chars().all(|ch| ch.is_ascii_alphanumeric() || ".-_+".contains(ch)) {
        bail!("Недопустимая версия загрузчика: {}", loader_version);
    }
    Ok(())
}

fn fetch_loader_profile(client: &Client, loader: &str, mc_version: &str, loader_version: &str, game_dir: &Path) -> Result<PathBuf> {
    validate_loader_version(loader_version)?;
    let (host, api_version) = if loader == "fabric" {
        ("https://meta.fabricmc.net/", "v2")
    } else {
        ("https://meta.quiltmc.org/", "v3")
    };
    let mut endpoint = Url::parse(host)?;
    endpoint.path_segments_mut().map_err(|_| anyhow!("Некорректный URL API загрузчика"))?
        .extend([api_version, "versions", "loader", mc_version, loader_version, "profile", "json"]);
    let response = client.get(endpoint).send()?.error_for_status()
        .with_context(|| format!("Не удалось получить профиль {} для Minecraft {}", loader, mc_version))?;
    let data: serde_json::Value = response.json().context("API модлоадера вернул неправильный JSON")?;
    let id = data.get("id").and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{loader}-{loader_version}-{mc_version}"));
    let directory = game_dir.join("versions").join(&id);
    fs::create_dir_all(&directory)?;
    let target = directory.join(format!("{id}.json"));
    fs::write(&target, serde_json::to_vec_pretty(&data)?)?;
    Ok(target)
}

fn install_forge_family(
    client: &Client,
    profile: &Profile,
    mc_version: &str,
    java_path: &Path,
    game_dir: &Path,
    paths: &AppPaths,
    status: Arc<dyn Fn(String) + Send + Sync>,
) -> Result<PathBuf> {
    validate_loader_version(&profile.loader_version)
        .context("В профиле не сохранена корректная версия загрузчика. Импортируй modpack заново.")?;
    if let Ok(existing_profile) = find_installed_loader_json(game_dir, mc_version, &profile.loader, &profile.loader_version) {
        (status)(format!("Использую уже установленный профиль {} {}…", profile.loader, profile.loader_version));
        return Ok(existing_profile);
    }
    let (repository, artifact_path) = if profile.loader.eq_ignore_ascii_case("Forge") {
        let coordinate = if profile.loader_version.starts_with(&format!("{mc_version}-")) {
            profile.loader_version.clone()
        } else {
            format!("{mc_version}-{}", profile.loader_version)
        };
        ("https://maven.minecraftforge.net/", format!(
            "net/minecraftforge/forge/{coordinate}/forge-{coordinate}-installer.jar"))
    } else if mc_version == "1.20.1" || profile.loader_version.starts_with("47.") {
        let coordinate = if profile.loader_version.starts_with(&format!("{mc_version}-")) {
            profile.loader_version.clone()
        } else {
            format!("{mc_version}-{}", profile.loader_version)
        };
        ("https://maven.neoforged.net/releases/", format!(
            "net/neoforged/forge/{coordinate}/forge-{coordinate}-installer.jar"))
    } else {
        let coordinate = profile.loader_version.clone();
        ("https://maven.neoforged.net/releases/", format!(
            "net/neoforged/neoforge/{coordinate}/neoforge-{coordinate}-installer.jar"))
    };
    let installer_url = format!("{}{}", repository, artifact_path);
    let installer_path = paths.shared.join("installers").join(format!("{}-{}-installer.jar", profile.loader.to_ascii_lowercase(), profile.loader_version));
    (status)(format!("Скачиваю установщик {} {}…", profile.loader, profile.loader_version));
    download_verified(client, &installer_url, &installer_path, None)?;

    (status)(format!("Устанавливаю {} в отдельную папку сборки…", profile.loader));
    let output = Command::new(java_path)
        .arg("-jar")
        .arg(&installer_path)
        .arg("--installClient")
        .arg(game_dir)
        .current_dir(game_dir)
        .output()
        .with_context(|| format!("Не удалось выполнить установщик {}", profile.loader))?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Установщик {} завершился с кодом {:?}. {} {}",
            profile.loader, output.status.code(), tail_text(&stdout), tail_text(&stderr));
    }

    find_installed_loader_json(game_dir, mc_version, &profile.loader, &profile.loader_version)
}

fn tail_text(text: &str) -> String {
    let trimmed = text.trim();
    let start = trimmed.char_indices().rev().nth(1800).map(|(i, _)| i).unwrap_or(0);
    trimmed[start..].to_owned()
}

fn find_installed_loader_json(game_dir: &Path, mc_version: &str, loader: &str, loader_version: &str) -> Result<PathBuf> {
    let versions_dir = game_dir.join("versions");
    let mut candidates = Vec::new();
    collect_loader_jsons(&versions_dir, mc_version, loader, loader_version, &mut candidates)?;
    candidates.into_iter().next().ok_or_else(|| anyhow!(
        "Установщик {} завершился, но не создал launch JSON для Minecraft {} в {}",
        loader, mc_version, versions_dir.display()))
}

fn collect_loader_jsons(root: &Path, mc_version: &str, loader: &str, loader_version: &str, result: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() { return Ok(()); }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_loader_jsons(&path, mc_version, loader, loader_version, result)?;
        } else if path.extension().and_then(|v| v.to_str()).is_some_and(|v| v.eq_ignore_ascii_case("json")) {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&fs::read(&path)?) else { continue; };
            let inherits = value.get("inheritsFrom").and_then(serde_json::Value::as_str).unwrap_or("");
            let id = value.get("id").and_then(serde_json::Value::as_str).unwrap_or("").to_ascii_lowercase();
            let (loader_token, loader_match) = if loader.eq_ignore_ascii_case("NeoForge") {
                ("neoforge", id.contains("neoforge"))
            } else {
                ("forge", id.contains("forge") && !id.contains("neoforge"))
            };
            let version_key = loader_version.strip_prefix(&format!("{mc_version}-")).unwrap_or(loader_version);
            let expected_suffix = format!("{loader_token}-{version_key}").to_ascii_lowercase();
            let version_match = !version_key.is_empty() && id.ends_with(&expected_suffix);
            if inherits == mc_version && loader_match && version_match && value.get("mainClass").is_some() {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(())
}

fn merge_loader_metadata(base: &mut VersionJson, loader: &serde_json::Value, mc_version: &str) -> Result<()> {
    if let Some(inherits) = loader.get("inheritsFrom").and_then(serde_json::Value::as_str) {
        if inherits != mc_version {
            bail!("Модлоадер рассчитан на Minecraft {}, а профиль использует {}", inherits, mc_version);
        }
    }
    if let Some(id) = loader.get("id").and_then(serde_json::Value::as_str) { base.id = id.to_owned(); }
    if let Some(main) = loader.get("mainClass") {
        let selected = main.as_str().or_else(|| main.get("client").and_then(serde_json::Value::as_str));
        if let Some(selected) = selected { base.main_class = selected.to_owned(); }
    }
    if let Some(libraries) = loader.get("libraries") {
        let mut parsed: Vec<Library> = serde_json::from_value(libraries.clone())
            .context("Список библиотек модлоадера имеет неизвестный формат")?;
        base.libraries.append(&mut parsed);
    }
    // Convert legacy Mojang arguments before appending a loader's arguments.
    // This keeps older Forge/NeoForge profiles launchable alongside modern JSON.
    if base.arguments.is_none() {
        if let Some(legacy) = base.minecraft_arguments.take() {
            base.arguments = Some(Arguments {
                game: legacy.split_whitespace().map(|value| ArgumentItem::Plain(value.to_owned())).collect(),
                jvm: Vec::new(),
            });
        }
    }
    if let Some(arguments_value) = loader.get("arguments") {
        let mut extra: Arguments = serde_json::from_value(arguments_value.clone())
            .context("Аргументы модлоадера имеют неизвестный формат")?;
        let base_args = base.arguments.get_or_insert_with(|| Arguments { game: Vec::new(), jvm: Vec::new() });
        base_args.game.append(&mut extra.game);
        base_args.jvm.append(&mut extra.jvm);
    }
    if let Some(args) = loader.get("minecraftArguments").and_then(serde_json::Value::as_str) {
        base.arguments.get_or_insert_with(|| Arguments { game: Vec::new(), jvm: Vec::new() }).game =
            args.split_whitespace().map(|value| ArgumentItem::Plain(value.to_owned())).collect();
        base.minecraft_arguments = None;
    }
    Ok(())
}

fn library_relative_path(library: &Library) -> Result<Option<PathBuf>> {
    if let Some(path) = library.downloads.as_ref()
        .and_then(|downloads| downloads.artifact.as_ref())
        .and_then(|artifact| artifact.path.as_deref()) {
        return safe_library_path(path).map(Some);
    }
    if let Some(name) = library.name.as_deref() {
        return safe_library_path(&maven_coordinate_path(name)?.to_string_lossy()).map(Some);
    }
    Ok(None)
}

fn safe_library_path(value: &str) -> Result<PathBuf> {
    let normalized = value.replace('\\', "/");
    if normalized.trim().is_empty() || normalized.starts_with('/') || normalized.contains('\0') {
        bail!("Недопустимый путь библиотеки Minecraft: {value}");
    }
    let mut clean = PathBuf::new();
    for part in normalized.split('/') {
        if part.is_empty() { continue; }
        if part == "." || part == ".." || part.contains(':') {
            bail!("Небезопасный путь библиотеки Minecraft: {value}");
        }
        clean.push(part);
    }
    if clean.as_os_str().is_empty() {
        bail!("Пустой путь библиотеки Minecraft");
    }
    Ok(clean)
}

fn maven_coordinate_path(name: &str) -> Result<PathBuf> {
    let (coordinate, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = coordinate.split(':').collect();
    if parts.len() < 3 { bail!("Некорректная Maven-координата библиотеки: {name}"); }
    if parts.iter().any(|part| part.is_empty() || part.contains('/') || part.contains('\\'))
        || parts[0].split('.').any(|segment| segment.is_empty())
        || extension.is_empty()
        || !extension.chars().all(|ch| ch.is_ascii_alphanumeric() || ".-_".contains(ch))
    {
        bail!("Небезопасная Maven-координата библиотеки: {name}");
    }
    let group_path = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    let classifier = parts.get(3).map(|v| format!("-{v}")).unwrap_or_default();
    Ok(PathBuf::from(format!("{group_path}/{artifact}/{version}/{artifact}-{version}{classifier}.{extension}")))
}

fn resolve_java(java_setting: &str, required_major: u32, paths: &AppPaths, status: Arc<dyn Fn(String) + Send + Sync>) -> Result<PathBuf> {
    let mut candidates = Vec::<PathBuf>::new();
    if !java_setting.trim().is_empty() && !java_setting.eq_ignore_ascii_case("auto") {
        candidates.push(PathBuf::from(java_setting));
    }
    candidates.push(PathBuf::from(if cfg!(windows) { "java.exe" } else { "java" }));
    if let Some(cached) = find_managed_java(paths, required_major)? { candidates.push(cached); }

    for candidate in candidates {
        if check_java_version(&candidate, required_major).is_ok() {
            (status)(format!("Использую Java {}: {}", required_major, candidate.display()));
            return Ok(candidate);
        }
    }
    (status)(format!("Подходящая Java {} не найдена. Автоматически устанавливаю Eclipse Temurin JRE…", required_major));
    install_managed_java(required_major, paths, status)
}

fn check_java_version(java_path: &Path, required_major: u32) -> Result<()> {
    let output = Command::new(java_path).arg("-version").output()
        .with_context(|| format!("Java не найдена: {}", java_path.display()))?;
    if !output.status.success() { bail!("Java {} -version завершилась с ошибкой", java_path.display()); }
    let text = format!("{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let version_token = text.split('"').nth(1).ok_or_else(|| anyhow!("Не удалось определить версию Java из '{}'.", text.trim()))?;
    let mut parts = version_token.split('.');
    let first = parts.next().and_then(|s| leading_number(s)).unwrap_or(0);
    let detected = if first == 1 { parts.next().and_then(|s| leading_number(s)).unwrap_or(0) } else { first };
    if detected != required_major {
        bail!("Для этой версии Minecraft ожидается Java {}, обнаружена Java {}. NEXUS установит совместимую версию отдельно.", required_major, detected);
    }
    Ok(())
}

fn find_managed_java(paths: &AppPaths, major: u32) -> Result<Option<PathBuf>> {
    let root = paths.shared.join("java").join(major.to_string());
    if !root.exists() { return Ok(None); }
    find_file_recursive(&root, if cfg!(windows) { "java.exe" } else { "java" }, 6)
}

fn install_managed_java(major: u32, paths: &AppPaths, status: Arc<dyn Fn(String) + Send + Sync>) -> Result<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (major, paths, status);
        bail!("Автоустановка Java пока поддерживает Windows x64. Укажи установленную Java вручную на этой платформе.");
    }
    #[cfg(target_os = "windows")]
    {
        let root = paths.shared.join("java").join(major.to_string());
        if let Some(existing) = find_file_recursive(&root, "java.exe", 6)? {
            if check_java_version(&existing, major).is_ok() { return Ok(existing); }
            fs::remove_dir_all(&root).context("Не удалось удалить несовместимую Java из кэша NEXUS")?;
        }
        fs::create_dir_all(&root)?;
        (status)(format!("Получаю метаданные Eclipse Temurin JRE {} для Windows x64…", major));
        let client = Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(600)).build()?;
        let metadata_url = format!(
            "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture=x64&image_type=jre&os=windows&vendor=eclipse"
        );
        let metadata: serde_json::Value = client.get(&metadata_url).send()?.error_for_status()
            .context("Eclipse Adoptium не вернул метаданные Java")?.json()
            .context("Eclipse Adoptium вернул некорректные метаданные Java")?;
        let package = metadata.as_array().and_then(|items| items.first())
            .and_then(|item| item.get("binary"))
            .and_then(|binary| binary.get("package"))
            .context("В метаданных Adoptium нет JRE-пакета")?;
        let download_url = package.get("link").and_then(serde_json::Value::as_str)
            .context("Adoptium не вернул ссылку на архив JRE")?;
        let expected_sha256 = package.get("checksum").and_then(serde_json::Value::as_str)
            .context("Adoptium не вернул SHA-256 архива JRE")?;
        if !download_url.starts_with("https://") { bail!("Adoptium вернул небезопасную ссылку Java"); }
        (status)(format!("Скачиваю Temurin JRE {} для Windows x64…", major));
        let bytes = client.get(download_url).send()?.error_for_status()
            .context("Не удалось скачать архив Java с Eclipse Adoptium")?.bytes()?;
        let actual_sha256 = hex::encode(Sha256::digest(&bytes));
        if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            bail!("SHA-256 архива Java не совпал. Загрузка отменена из соображений безопасности.");
        }
        let archive_path = root.join("temurin-jre.zip");
        fs::write(&archive_path, &bytes)?;
        let file = File::open(&archive_path)?;
        let mut archive = ZipArchive::new(file).context("Adoptium вернул повреждённый ZIP Java")?;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let Some(relative) = entry.enclosed_name() else { continue; };
            let destination = root.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(destination)?;
            } else {
                if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
                let mut output = File::create(destination)?;
                std::io::copy(&mut entry, &mut output)?;
            }
        }
        let _ = fs::remove_file(archive_path);
        let java = find_file_recursive(&root, "java.exe", 6)?
            .context("В архиве Temurin не найден bin/java.exe")?;
        check_java_version(&java, major).context("Установленная Java не прошла проверку версии")?;
        (status)(format!("Java {} установлена: {}", major, java.display()));
        Ok(java)
    }
}

fn find_file_recursive(root: &Path, file_name: &str, depth: usize) -> Result<Option<PathBuf>> {
    if depth == 0 || !root.exists() { return Ok(None); }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_file() && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(file_name)) {
            return Ok(Some(path));
        }
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, file_name, depth - 1)? { return Ok(Some(found)); }
        }
    }
    Ok(None)
}

fn download_assets(client: &Client, assets_dir: &Path, game_dir: &Path, index_id: &str, index: AssetIndexContent, status: Arc<dyn Fn(String) + Send + Sync>) -> Result<()> {
    let objects_dir = assets_dir.join("objects");
    fs::create_dir_all(&objects_dir)?;
    let object_pairs: Vec<_> = index.objects.into_iter().collect();
    let total = object_pairs.len();
    let jobs = Arc::new(Mutex::new(object_pairs.clone().into_iter()));
    let completed = Arc::new(AtomicUsize::new(0));
    let errors = Arc::new(Mutex::new(Vec::<String>::new()));
    let workers = 8usize.min(total.max(1));
    thread::scope(|scope| {
        for _ in 0..workers {
            let jobs = Arc::clone(&jobs);
            let completed = Arc::clone(&completed);
            let errors = Arc::clone(&errors);
            let client = client.clone();
            let base = objects_dir.clone();
            let status = Arc::clone(&status);
            scope.spawn(move || loop {
                let next = jobs.lock().ok().and_then(|mut guard| guard.next());
                let Some((_name, object)) = next else { break; };
                let prefix = object.hash.get(0..2).unwrap_or("00");
                let target = base.join(prefix).join(&object.hash);
                let url = format!("https://resources.download.minecraft.net/{}/{}", prefix, object.hash);
                if let Err(err) = download_verified_with_size(
                    &client,
                    &url,
                    &target,
                    Some(&object.hash),
                    (object.size > 0).then_some(object.size),
                ) {
                    if let Ok(mut list) = errors.lock() { list.push(format!("{}: {}", object.hash, err)); }
                }
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 150 == 0 || done == total {
                    status(format!("Загружаю ресурсы Minecraft: {}/{}", done, total));
                }
            });
        }
    });
    if let Ok(list) = errors.lock() {
        if !list.is_empty() { bail!("Не удалось скачать ресурс Minecraft: {}", list[0]); }
    }
    if index.virtual_assets || index.map_to_resources {
        for (name, object) in &object_pairs {
            let prefix = object.hash.get(0..2).unwrap_or("00");
            let source = objects_dir.join(prefix).join(&object.hash);
            let destination = if index.virtual_assets {
                assets_dir.join("virtual").join(index_id).join(name)
            } else {
                game_dir.join("resources").join(name)
            };
            if let Some(parent) = destination.parent() { fs::create_dir_all(parent)?; }
            if !destination.exists() { fs::copy(source, destination)?; }
        }
    }
    Ok(())
}

fn download_verified(client: &Client, url: &str, path: &Path, expected_sha1: Option<&str>) -> Result<()> {
    download_verified_with_size(client, url, path, expected_sha1, None)
}

fn download_verified_with_size(
    client: &Client,
    url: &str,
    path: &Path,
    expected_sha1: Option<&str>,
    expected_size: Option<u64>,
) -> Result<()> {
    if path.exists() {
        let size_matches = match expected_size {
            Some(expected) => fs::metadata(path).map(|meta| meta.len() == expected).unwrap_or(false),
            None => fs::metadata(path).map(|meta| meta.len() > 0).unwrap_or(false),
        };
        let hash_matches = expected_sha1
            .map(|expected| sha1_file(path).map(|actual| actual.eq_ignore_ascii_case(expected)))
            .transpose()?
            .unwrap_or(true);
        if size_matches && hash_matches {
            return Ok(());
        }
        fs::remove_file(path).with_context(|| format!("Не удалось удалить повреждённый файл {}", path.display()))?;
    }

    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let response = client.get(url).send()?.error_for_status()
        .with_context(|| format!("Ошибка загрузки {}", url))?;
    let bytes = response.bytes()?;
    if let Some(expected) = expected_size {
        if bytes.len() as u64 != expected {
            bail!("Размер файла не совпадает для {}: ожидалось {} байт, получено {}", url, expected, bytes.len());
        }
    }
    if let Some(expected) = expected_sha1 {
        let actual = hex::encode(Sha1::digest(&bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            bail!("SHA-1 не совпадает для {}", url);
        }
    }

    let temp_name = format!("{}.{}.download.tmp",
        path.file_name().and_then(|name| name.to_str()).unwrap_or("nexus"), Uuid::new_v4());
    let temp = path.with_file_name(temp_name);
    fs::write(&temp, &bytes)?;
    fs::rename(&temp, path).or_else(|_| {
        fs::copy(&temp, path)?;
        let _ = fs::remove_file(&temp);
        Ok::<(), std::io::Error>(())
    })?;
    Ok(())
}

fn sha1_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        hasher.update(&buffer[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn leading_number(value: &str) -> Option<u32> {
    let digits: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn rules_allow(rules: Option<&[Rule]>) -> bool {
    let Some(rules) = rules else { return true; };
    if rules.is_empty() { return true; }
    let mut allowed = false;
    for rule in rules {
        let os_match = rule.os.as_ref().and_then(|o| o.name.as_deref()).map(|n| n == platform_key()).unwrap_or(true);
        let feature_match = rule.features.as_ref().map(features_allow).unwrap_or(true);
        if os_match && feature_match { allowed = rule.action == "allow"; }
    }
    allowed
}

fn features_allow(features: &HashMap<String, bool>) -> bool {
    // NEXUS launches a normal game session: demo/custom-resolution/quick-play flags are off.
    features.iter().all(|(name, expected)| {
        let actual = match name.as_str() {
            "is_demo_user" | "has_custom_resolution" | "has_quick_plays_support" |
            "is_quick_play_singleplayer" | "is_quick_play_multiplayer" | "is_quick_play_realms" => false,
            _ => return false,
        };
        actual == *expected
    })
}

fn platform_key() -> &'static str {
    if cfg!(target_os = "windows") { "windows" }
    else if cfg!(target_os = "macos") { "osx" }
    else { "linux" }
}

fn flatten_arguments(arguments: &[ArgumentItem]) -> Vec<String> {
    let mut output = Vec::new();
    for argument in arguments {
        match argument {
            ArgumentItem::Plain(value) => output.push(value.clone()),
            ArgumentItem::Conditional(value) if rules_allow(Some(&value.rules)) => match &value.value {
                ArgumentValue::One(s) => output.push(s.clone()),
                ArgumentValue::Many(values) => output.extend(values.clone()),
            },
            ArgumentItem::Conditional(_) => {}
        }
    }
    output
}

fn build_replacements(
    name: &str, uuid: &str, access_token: &str, user_type: &str, xuid: &str,
    version: &str, version_type: &str, game_dir: &Path, assets_dir: &Path, asset_index: &str,
    natives_dir: &Path, shared: &Path, classpath: &str,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let values = [
        ("${auth_player_name}", name.to_owned()),
        ("${auth_uuid}", uuid.to_owned()),
        ("${auth_access_token}", access_token.to_owned()),
        ("${user_type}", user_type.to_owned()),
        ("${auth_xuid}", xuid.to_owned()),
        ("${clientid}", String::new()),
        ("${version_name}", version.to_owned()),
        ("${version_type}", if version_type.is_empty() { "release".to_owned() } else { version_type.to_owned() }),
        ("${game_directory}", game_dir.to_string_lossy().into_owned()),
        ("${assets_root}", assets_dir.to_string_lossy().into_owned()),
        ("${assets_index_name}", asset_index.to_owned()),
        ("${natives_directory}", natives_dir.to_string_lossy().into_owned()),
        ("${library_directory}", shared.join("libraries").to_string_lossy().into_owned()),
        ("${launcher_name}", "NEXUS".to_owned()),
        ("${launcher_version}", "0.2.0".to_owned()),
        ("${classpath}", classpath.to_owned()),
        ("${classpath_separator}", if cfg!(windows) { ";".to_owned() } else { ":".to_owned() }),
        ("${user_properties}", "{}".to_owned()),
        ("${auth_session}", access_token.to_owned()),
        ("${resolution_width}", "854".to_owned()),
        ("${resolution_height}", "480".to_owned()),
    ];
    for (k, v) in values { map.insert(k.to_owned(), v); }
    map
}

fn replace_tokens(input: &str, replacements: &HashMap<String, String>) -> String {
    let mut output = input.to_owned();
    for (key, value) in replacements { output = output.replace(key, value); }
    output
}

fn offline_uuid(name: &str) -> String {
    // Matches Java's UUID.nameUUIDFromBytes("OfflinePlayer:<name>") algorithm.
    let mut hasher = Md5::new();
    hasher.update(format!("OfflinePlayer:{}", name).as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes).to_string().replace('-', "")
}

fn extract_natives(jar_path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(jar_path)?;
    let mut archive = ZipArchive::new(file).context("Повреждённый natives JAR")?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        // zip 2.x already returns an owned PathBuf from enclosed_name().
        let Some(relative) = entry.enclosed_name() else { continue; };
        if relative.components().any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case("META-INF")) { continue; }
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() { fs::create_dir_all(parent)?; }
            let mut out = File::create(target)?;
            std::io::copy(&mut entry, &mut out)?;
        }
    }
    Ok(())
}
