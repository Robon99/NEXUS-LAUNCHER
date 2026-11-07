mod accounts;
mod config;
mod minecraft;
mod modpacks;

slint::include_modules!();

use anyhow::{Context, Result};
use config::{Account, AccountKind, AppPaths, Profile, Settings};
use slint::{ModelRc, SharedString, VecModel, Weak};
use std::{path::Path, sync::{Arc, Mutex}, thread};

fn main() -> Result<()> {
    let paths = AppPaths::discover()?;
    let settings = paths.load_settings()?;
    let shared_settings = Arc::new(Mutex::new(settings));
    let app = AppWindow::new().context("Не удалось создать окно лаунчера")?;

    {
        let cfg = shared_settings.lock().expect("settings mutex poisoned");
        apply_settings_to_ui(&app, &cfg);
    }

    // Создание независимого инстанса. Игровые файлы будут храниться отдельно.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_create_instance(move || {
            let (name, version, nickname) = match weak.upgrade() {
                Some(ui) => (
                    ui.get_profile_name().to_string(),
                    ui.get_version_text().to_string(),
                    ui.get_nickname_text().to_string(),
                ),
                None => return,
            };
            match paths.make_profile(&name, &version, &nickname) {
                Ok(profile) => {
                    if let Ok(mut cfg) = state.lock() {
                        cfg.selected_profile = Some(profile.id.clone());
                        cfg.profiles.push(profile);
                        let _ = paths.save_settings(&cfg);
                        if let Some(ui) = weak.upgrade() {
                            apply_settings_to_ui(&ui, &cfg);
                            ui.set_status_text("Профиль создан. Java при необходимости установится автоматически.".into());
                        }
                    }
                }
                Err(err) => set_status(&weak, format!("Не удалось создать профиль: {err:#}")),
            }
        });
    }

    // Выбор профиля.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_select_instance(move |id| {
            if let Ok(mut cfg) = state.lock() {
                cfg.selected_profile = Some(id.to_string());
                if let Some(profile) = cfg.profiles.iter().find(|p| p.id == id.as_str()) {
                    if let Some(ui) = weak.upgrade() {
                        ui.set_selected_id(profile.id.clone().into());
                        ui.set_profile_name(profile.name.clone().into());
                        ui.set_version_text(profile.mc_version.clone().into());
                        ui.set_nickname_text(profile.nickname.clone().into());
                        ui.set_account_text(account_label(profile, &cfg).into());
                    }
                }
                let _ = paths.save_settings(&cfg);
            }
        });
    }

    // Удаляем запись из библиотеки, но не стираем папку с мирами и модами.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_delete_instance(move |id| {
            if let Ok(mut cfg) = state.lock() {
                cfg.profiles.retain(|p| p.id != id.as_str());
                if cfg.selected_profile.as_deref() == Some(id.as_str()) {
                    cfg.selected_profile = cfg.profiles.first().map(|p| p.id.clone());
                }
                let _ = paths.save_settings(&cfg);
                if let Some(ui) = weak.upgrade() {
                    apply_settings_to_ui(&ui, &cfg);
                    ui.set_status_text("Профиль удалён из библиотеки. Его папка с данными сохранена.".into());
                }
            }
        });
    }

    // Установка/проверка файлов и запуск игры выполняются в отдельном потоке.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_launch_instance(move |id| {
            let snapshot = match state.lock() {
                Ok(cfg) => cfg.clone(),
                Err(_) => { set_status(&weak, "Не удалось прочитать настройки".into()); return; }
            };
            let Some(profile) = snapshot.profiles.iter().find(|p| p.id == id.as_str()).cloned() else {
                set_status(&weak, "Профиль не найден".into());
                return;
            };
            set_status(&weak, "Подготавливаю запуск…".into());
            let weak_worker = weak.clone();
            let paths_worker = paths.clone();
            thread::spawn(move || {
                let status_weak = weak_worker.clone();
                let result = minecraft::install_and_launch(&profile, &snapshot, &paths_worker, move |message| {
                    set_status(&status_weak, message);
                });
                match result {
                    Ok(()) => set_status(&weak_worker, "Команда запуска передана Java. Если окно игры не появилось, проверь nexus-launch.log.".into()),
                    Err(err) => set_status(&weak_worker, format!("Ошибка запуска: {err:#}")),
                }
            });
        });
    }

    // Import Modrinth .mrpack and CurseForge .zip archives.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_import_modpack(move || {
            let Some(file) = rfd::FileDialog::new()
                .add_filter("Minecraft modpacks", &["mrpack", "zip"])
                .pick_file() else { return; };
            let display_name = file.file_name().map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "сборку".to_owned());
            set_status(&weak, format!("Импортирую {}…", display_name));
            let weak_worker = weak.clone();
            let state_worker = Arc::clone(&state);
            let paths_worker = paths.clone();
            thread::spawn(move || {
                let status_weak = weak_worker.clone();
                let status_callback = move |message| set_status(&status_weak, message);
                let is_zip = file.extension().and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
                let result = if is_zip {
                    let api_key = std::env::var("NEXUS_CURSEFORGE_API_KEY").unwrap_or_default();
                    modpacks::import_curseforge_zip(&file, &paths_worker, &api_key, status_callback)
                } else {
                    modpacks::import_mrpack(&file, &paths_worker, status_callback)
                };
                match result {
                    Ok(profile) => add_imported_profile(&state_worker, &paths_worker, &weak_worker, profile),
                    Err(err) => set_status(&weak_worker, format!("Ошибка импорта: {err:#}")),
                }
            });
        });
    }

    // CurseForge modpack catalogue search (requires the user's/developer's API key).
    {
        let weak = app.as_weak();
        app.on_search_curseforge(move |query| {
            let query = query.to_string();
            if query.trim().is_empty() {
                set_status(&weak, "Введи название сборки для поиска на CurseForge.".into());
                return;
            }
            let api_key = std::env::var("NEXUS_CURSEFORGE_API_KEY").unwrap_or_default();
            let weak_worker = weak.clone();
            set_status(&weak, "Ищу сборки на CurseForge…".into());
            thread::spawn(move || {
                match modpacks::search_curseforge_modpacks(&query, &api_key) {
                    Ok(results) => {
                        let cards: Vec<CurseForgeCard> = results.into_iter().map(|pack| CurseForgeCard {
                            id: pack.id.to_string().into(),
                            name: pack.name.into(),
                            summary: pack.summary.into(),
                            downloads: pack.download_count.to_string().into(),
                        }).collect();
                        let count = cards.len();
                        let weak_ui = weak_worker.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(ui) = weak_ui.upgrade() {
                                ui.set_curseforge_results(ModelRc::new(VecModel::from(cards)));
                                ui.set_status_text(format!("CurseForge: найдено сборок — {count}.").into());
                            }
                        });
                    }
                    Err(err) => set_status(&weak_worker, format!("Ошибка поиска CurseForge: {err:#}")),
                }
            });
        });
    }

    // Download the selected CurseForge modpack using the official Core API and import it.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_install_curseforge(move |id| {
            let Ok(mod_id) = id.parse::<i64>() else {
                set_status(&weak, "Некорректный ID сборки CurseForge.".into());
                return;
            };
            let api_key = std::env::var("NEXUS_CURSEFORGE_API_KEY").unwrap_or_default();
            let weak_worker = weak.clone();
            let state_worker = Arc::clone(&state);
            let paths_worker = paths.clone();
            set_status(&weak, format!("Готовлю загрузку сборки CurseForge #{mod_id}…"));
            thread::spawn(move || {
                let status_weak = weak_worker.clone();
                let result = modpacks::download_curseforge_modpack(mod_id, &paths_worker, &api_key, move |message| {
                    set_status(&status_weak, message);
                });
                match result {
                    Ok(profile) => add_imported_profile(&state_worker, &paths_worker, &weak_worker, profile),
                    Err(err) => set_status(&weak_worker, format!("Ошибка CurseForge: {err:#}")),
                }
            });
        });
    }

    // Применить никнейм как отдельный офлайн-аккаунт и привязать к выбранному инстансу.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_add_offline_account(move |nickname| {
            let name = config::clean_nickname(nickname.as_str());
            if let Ok(mut cfg) = state.lock() {
                let id = format!("offline-{}", uuid::Uuid::new_v4());
                let account = Account {
                    id: id.clone(),
                    name: name.clone(),
                    uuid: uuid::Uuid::new_v4().to_string(),
                    kind: AccountKind::Offline,
                    xuid: None,
                };
                cfg.accounts.push(account);
                if let Some(selected) = cfg.selected_profile.clone() {
                    if let Some(profile) = cfg.profiles.iter_mut().find(|p| p.id == selected) {
                        profile.nickname = name.clone();
                        profile.account_id = Some(id);
                    }
                }
                let _ = paths.save_settings(&cfg);
                if let Some(ui) = weak.upgrade() {
                    apply_settings_to_ui(&ui, &cfg);
                    ui.set_status_text(format!("Офлайн-аккаунт {} добавлен.", name).into());
                }
            }
        });
    }

    // Переключение между сохранёнными аккаунтами для выбранного инстанса.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_switch_account(move || {
            if let Ok(mut cfg) = state.lock() {
                let Some(selected_id) = cfg.selected_profile.clone() else {
                    if let Some(ui) = weak.upgrade() { ui.set_status_text("Сначала выбери сборку.".into()); }
                    return;
                };
                if cfg.accounts.is_empty() {
                    if let Some(ui) = weak.upgrade() { ui.set_status_text("Сначала добавь офлайн- или Microsoft-аккаунт.".into()); }
                    return;
                }
                let current_id = cfg.profiles.iter()
                    .find(|profile| profile.id == selected_id)
                    .and_then(|profile| profile.account_id.clone());
                let current_index = current_id.as_ref()
                    .and_then(|id| cfg.accounts.iter().position(|account| &account.id == id));
                let next_index = current_index.map(|index| (index + 1) % cfg.accounts.len()).unwrap_or(0);
                let account = cfg.accounts[next_index].clone();
                if let Some(profile) = cfg.profiles.iter_mut().find(|profile| profile.id == selected_id) {
                    profile.account_id = Some(account.id.clone());
                    profile.nickname = account.name.clone();
                }
                let _ = paths.save_settings(&cfg);
                if let Some(ui) = weak.upgrade() {
                    apply_settings_to_ui(&ui, &cfg);
                    ui.set_status_text(format!("Для выбранной сборки выбран аккаунт {}.", account.name).into());
                }
            }
        });
    }

    // Реальная цепочка Microsoft -> Xbox Live -> XSTS -> Minecraft Services.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_microsoft_login(move || {
            let Some(client_id) = std::env::var("NEXUS_MICROSOFT_CLIENT_ID").ok() else {
                set_status(&weak, "Microsoft-вход не настроен: задай NEXUS_MICROSOFT_CLIENT_ID после регистрации приложения Microsoft.".into());
                return;
            };
            let weak_worker = weak.clone();
            let state_worker = Arc::clone(&state);
            let paths_worker = paths.clone();
            thread::spawn(move || {
                let status_weak = weak_worker.clone();
                let result = accounts::microsoft::sign_in(&client_id, move |message| {
                    set_status(&status_weak, message);
                });
                match result {
                    Ok(login) => {
                        if let Ok(mut cfg) = state_worker.lock() {
                            cfg.accounts.retain(|a| a.id != login.account.id);
                            cfg.accounts.push(login.account.clone());
                            if let Some(selected) = cfg.selected_profile.clone() {
                                if let Some(profile) = cfg.profiles.iter_mut().find(|p| p.id == selected) {
                                    profile.nickname = login.account.name.clone();
                                    profile.account_id = Some(login.account.id.clone());
                                }
                            }
                            let _ = paths_worker.save_settings(&cfg);
                            let snapshot = cfg.clone();
                            let account_name = login.account.name.clone();
                            let weak_ui = weak_worker.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = weak_ui.upgrade() {
                                    apply_settings_to_ui(&ui, &snapshot);
                                    ui.set_status_text(format!("Microsoft-аккаунт {} добавлен. Лицензия проверена.", account_name).into());
                                }
                            });
                        }
                    }
                    Err(err) => set_status(&weak_worker, format!("Ошибка Microsoft-входа: {err:#}")),
                }
            });
        });
    }

    // Сохранение Java и выделенной памяти.
    {
        let weak = app.as_weak();
        let state = Arc::clone(&shared_settings);
        let paths = paths.clone();
        app.on_save_settings(move || {
            if let (Some(ui), Ok(mut cfg)) = (weak.upgrade(), state.lock()) {
                cfg.memory_gb = ui.get_memory_gb().clamp(2, 32) as u8;
                let java = ui.get_java_path().trim().to_owned();
                cfg.java_path = if java.is_empty() { "auto".to_owned() } else { java };
                match paths.save_settings(&cfg) {
                    Ok(()) => ui.set_status_text("Настройки сохранены.".into()),
                    Err(err) => ui.set_status_text(format!("Не удалось сохранить настройки: {err:#}").into()),
                }
            }
        });
    }

    {
        let folder = paths.instances.clone();
        app.on_open_instances_folder(move || {
            let _ = open::that(&folder);
        });
    }

    app.run().context("Ошибка цикла интерфейса")?;
    Ok(())
}

fn add_imported_profile(
    state: &Arc<Mutex<Settings>>,
    paths: &AppPaths,
    weak: &Weak<AppWindow>,
    profile: Profile,
) {
    if let Ok(mut cfg) = state.lock() {
        cfg.selected_profile = Some(profile.id.clone());
        cfg.profiles.push(profile);
        if let Err(err) = paths.save_settings(&cfg) {
            set_status(weak, format!("Сборка импортирована, но настройки не сохранились: {err:#}"));
            return;
        }
        let snapshot = cfg.clone();
        let weak_ui = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak_ui.upgrade() {
                apply_settings_to_ui(&ui, &snapshot);
                ui.set_status_text("Сборка добавлена в библиотеку. Нажми «Установить / Играть».".into());
            }
        });
    }
}

fn apply_settings_to_ui(ui: &AppWindow, settings: &Settings) {
    let items: Vec<InstanceCard> = settings.profiles.iter().map(|profile| {
        InstanceCard {
            id: SharedString::from(profile.id.clone()),
            name: SharedString::from(profile.name.clone()),
            mc_version: SharedString::from(profile.mc_version.clone()),
            loader: SharedString::from(profile.loader.clone()),
            nickname: SharedString::from(profile.nickname.clone()),
            installed: Path::new(&profile.game_dir).join(".nexus-installed-version").exists(),
        }
    }).collect();
    ui.set_instances(ModelRc::new(VecModel::from(items)));
    ui.set_selected_id(settings.selected_profile.clone().unwrap_or_default().into());
    ui.set_memory_gb(settings.memory_gb.clamp(2, 32) as i32);
    ui.set_java_path(settings.java_path.clone().into());
    if let Some(profile) = settings.profiles.iter().find(|p| Some(p.id.as_str()) == settings.selected_profile.as_deref()) {
        ui.set_profile_name(profile.name.clone().into());
        ui.set_version_text(profile.mc_version.clone().into());
        ui.set_nickname_text(profile.nickname.clone().into());
        ui.set_account_text(account_label(profile, settings).into());
    } else {
        ui.set_profile_name("Моя сборка".into());
        ui.set_version_text("latest-release".into());
        ui.set_nickname_text("Player".into());
        ui.set_account_text("Офлайн-профиль".into());
    }
}

fn account_label(profile: &Profile, settings: &Settings) -> String {
    if let Some(id) = profile.account_id.as_deref() {
        if let Some(account) = settings.accounts.iter().find(|a| a.id == id) {
            return match account.kind {
                AccountKind::Microsoft => format!("Microsoft • {}", account.name),
                AccountKind::Offline => format!("Офлайн • {}", account.name),
            };
        }
    }
    format!("Офлайн • {}", profile.nickname)
}

fn set_status(weak: &Weak<AppWindow>, message: String) {
    let weak = weak.clone();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_text(message.into());
        }
    });
}

