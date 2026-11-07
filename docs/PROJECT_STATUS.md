# NEXUS Launcher 0.2.0 — project status

This document distinguishes source-level implementation from verified behavior. The project has not yet completed a full Windows build-and-test pass in the development environment used to prepare this repository.

## Present in the source tree

- Native Slint desktop UI and an instance library.
- Create, select, and delete isolated game instances.
- JSON configuration and shared Minecraft caches.
- Offline nicknames and multiple saved account records.
- Microsoft OAuth Authorization Code + PKCE in the system browser with a localhost callback.
- Microsoft → Xbox Live → XSTS → Minecraft Services exchange, Java entitlement/profile checks, Windows Credential Manager refresh-token storage, and token renewal code.
- Mojang version metadata, client/libraries/natives/assets/logging downloads, and launch argument assembly.
- Fabric and Quilt metadata/profile retrieval.
- Forge and NeoForge installer download and `--installClient` invocation.
- Modrinth `.mrpack` import, client files, overrides, hashes, and safe archive path handling.
- CurseForge exported `.zip` manifest import, mod downloads through Core API, and catalogue search/download code.
- Windows x64 Temurin JRE download for the Java major version requested by Minecraft metadata.

## External setup required

- Microsoft Entra desktop app registration and `NEXUS_MICROSOFT_CLIENT_ID`.
- Minecraft App ID Review approval for the Client ID before relying on Minecraft Services access.
- A CurseForge-approved Core API key and `NEXUS_CURSEFORGE_API_KEY` for CurseForge catalogue/API downloads.

## Not yet verified / follow-up work

- A complete `cargo check --all-targets` and `cargo build --release` on Windows 10/11 x64.
- End-to-end sign-in with an approved Client ID and a Minecraft Java account.
- Actual game launch smoke tests for representative Vanilla, Fabric, Quilt, Forge, and NeoForge versions.
- Modpack compatibility tests across a range of real Modrinth and CurseForge exports.
- Exact loader version selectors when creating a new instance outside imported modpacks.
- Mod manager, update manager, instance clone/backup, SQLite migration, installer packaging, and auto-update.
- macOS/Linux support. Automatic Java runtime download currently targets Windows x64.

## Recommended verification sequence

Run `cargo check --all-targets` on Windows, then test each feature separately. Record the Windows version, Rust version, Minecraft version, loader version, and the relevant redacted `nexus-launch.log` when reporting issues. Never publish account tokens or API keys.
