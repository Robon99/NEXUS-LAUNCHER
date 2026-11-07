# NEXUS Launcher

**Независимый экспериментальный лаунчер Minecraft: Java Edition на Rust и Slint.**

NEXUS — проект на ранней стадии разработки. Его цель — управление отдельными игровыми инстансами, импорт сборок, работа с Java Runtime и авторизация Microsoft через официальный вход и Minecraft Services.

> **Статус проекта:** экспериментальная разработка. Полная сборка под Windows и сквозные тесты запуска игры пока не подтверждены. Перечисленные ниже функции присутствуют в исходниках, но не все комбинации версий Minecraft и загрузчиков проверены в реальной игре.

> **Проект не связан и не одобрен Mojang Studios, Microsoft, Xbox, CurseForge, Modrinth, Fabric, Quilt, Forge, NeoForge или Eclipse Adoptium.** Minecraft является товарным знаком Microsoft. NEXUS — независимый сторонний лаунчер.

## Что реализовано в исходниках

- Нативный интерфейс на Rust + Slint.
- Отдельные игровые инстансы со своими папками модов, конфигураций, миров и ресурспаков.
- Офлайн-ники для локальной игры. Офлайн-режим не подтверждает владение Minecraft и не обходит авторизацию серверов с `online-mode=true`.
- Вход Microsoft в системном браузере через OAuth Authorization Code + PKCE и localhost callback.
- Обмен токенами Microsoft → Xbox Live → XSTS → Minecraft Services, проверка лицензии Java Edition и игрового профиля.
- Хранение refresh-токена через Windows Credential Manager.
- Загрузка официальных метаданных Minecraft, клиентских файлов, библиотек, ресурсов и natives.
- Интеграция метаданных Fabric и Quilt.
- Запуск установщиков Forge и NeoForge внутри папки выбранного инстанса.
- Импорт Modrinth `.mrpack` и CurseForge `.zip`.
- Интеграция каталога и скачивания CurseForge через Core API (нужен разрешённый CurseForge API key).
- Автоматическая загрузка Eclipse Temurin JRE для Windows x64.

Подробности и ограничения: [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md).

## Сборка на Windows 10/11 x64

1. Установи стабильный Rust через [rustup](https://rustup.rs/).
2. Установи Visual Studio Build Tools с компонентом **Desktop development with C++** и Windows SDK.
3. Открой PowerShell в папке проекта.
4. Выполни:

```powershell
cargo check --all-targets
cargo run --release
```

Для release-сборки:

```powershell
cargo build --release
```

Файл должен появиться в `target\release\nexus-launcher.exe`.

## Microsoft-вход

Для Microsoft-входа нужны два отдельных шага: регистрация desktop-приложения в Microsoft Entra и одобрение Client ID для доступа к Minecraft Services.

Инструкция и шаблон текста заявки: [`docs/MICROSOFT_APP_ID_REVIEW.md`](docs/MICROSOFT_APP_ID_REVIEW.md).

После регистрации приложения передай его Application (client) ID через переменную Windows:

```powershell
setx NEXUS_MICROSOFT_CLIENT_ID "ТВОЙ-APPLICATION-CLIENT-ID"
```

Не добавляй client secret: desktop-приложение является public client, а секрет в распространяемом `.exe` невозможно защитить.

## CurseForge

Для каталога CurseForge и скачивания файлов нужен API key, разрешённый CurseForge для такого использования:

```powershell
setx NEXUS_CURSEFORGE_API_KEY "ТВОЙ-CURSEFORGE-API-KEY"
```

Не публикуй ключ в GitHub. Импорт Modrinth `.mrpack` не требует ключа CurseForge.

## Конфиденциальность и безопасность

Настройки и данные инстансов сохраняются локально. Refresh-токен Microsoft находится в Windows Credential Manager. Имя аккаунта, UUID и XUID сохраняются в локальной конфигурации. Вывод процесса Minecraft записывается в `nexus-launch.log` внутри инстанса.

В текущих исходниках нет сервера аккаунтов NEXUS и SDK аналитики/телеметрии. Для авторизации и скачивания используются внешние сервисы Microsoft, Xbox, Minecraft, Mojang, Fabric, Quilt, Forge, NeoForge, Adoptium и CurseForge.

См. [`PRIVACY.md`](PRIVACY.md) и [`SECURITY.md`](SECURITY.md).
