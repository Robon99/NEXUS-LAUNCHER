# NEXUS Launcher

**An independent Minecraft: Java Edition launcher prototype written in Rust and Slint.**

NEXUS is an early-stage desktop project focused on separate game instances, modpack import, Java runtime handling, and Microsoft account authentication through the official sign-in and Minecraft Services flow.

> **Project status:** experimental / under active development. The source archive has not yet been verified by a complete Windows build and end-to-end game-launch tests. Features listed below describe the current source implementation; they should not be treated as a guarantee that every Minecraft version or mod-loader combination has been tested.

> **Not affiliated with or endorsed by Mojang Studios, Microsoft, Xbox, CurseForge, Modrinth, Fabric, Quilt, Forge, NeoForge, or Eclipse Adoptium.** Minecraft is a trademark of Microsoft. This project is an independent third-party launcher.

## Current source features

- Rust desktop application with a Slint UI.
- A library of separate Minecraft instances, each with its own game directory, mods, configs, saves, resource packs, and shader packs.
- Offline nicknames for local/offline play. Offline mode does not authenticate ownership and does not bypass online-mode server authentication.
- Microsoft sign-in flow opened in the system browser, using OAuth Authorization Code with PKCE and a localhost callback.
- Xbox Live, XSTS, and Minecraft Services token exchange, followed by Minecraft Java entitlement and profile checks.
- Refresh-token storage through the Windows Credential Manager integration provided by `keyring`.
- Mojang version metadata, client files, libraries, assets, native libraries, and launch argument handling.
- Fabric and Quilt metadata/profile integration.
- Forge and NeoForge installer invocation in an instance-specific directory.
- Modrinth `.mrpack` import, including overrides and client files.
- CurseForge exported `.zip` import and Core API catalogue/download integration (requires an API key that CurseForge has approved for this use).
- Windows x64 Eclipse Temurin JRE download for the Java major version requested by the Minecraft metadata.

See [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md) for implementation notes and verification gaps.

## Build requirements

Currently targeted: **Windows 10/11 x64**.

1. Install the stable Rust toolchain using [rustup](https://rustup.rs/).
2. Install Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.
3. Clone this repository.
4. Open PowerShell in the repository root and run:

```powershell
cargo check --all-targets
cargo run --release
```

To build the release executable:

```powershell
cargo build --release
```

The executable should be created at `target\release\nexus-launcher.exe`. Cargo will also generate `Cargo.lock`; because NEXUS is an application, commit that lockfile after a successful dependency resolution/build so future builds use the same resolved dependency versions.

Convenience scripts:

- `check-project.bat` — runs `cargo check --all-targets`.
- `run-dev.bat` — runs the application in development mode.
- `build-release.bat` — builds the release executable.

## Microsoft account setup

The Microsoft login implementation requires an application registration and an approved Minecraft App ID. These are separate steps:

1. Register a public desktop application in [Microsoft Entra](https://entra.microsoft.com/).
2. Configure personal Microsoft account support, add the **Mobile and desktop applications** platform, register `http://localhost`, and enable **Allow public client flows**.
3. Set the application's **Application (client) ID** in the environment variable `NEXUS_MICROSOFT_CLIENT_ID`.
4. Submit that Client ID through the [Minecraft App ID Review process](https://aka.ms/mce-reviewappid) and wait for approval before relying on Minecraft Services access.

The launcher uses the system browser and does not ask users to type their Microsoft password into NEXUS. **Do not add a client secret to this desktop application.** Desktop applications are public clients; secrets embedded in distributed binaries cannot be kept confidential.

PowerShell example:

```powershell
setx NEXUS_MICROSOFT_CLIENT_ID "YOUR-APPLICATION-CLIENT-ID"
```

Close and reopen the terminal and launcher after running `setx`.

Detailed notes and a review-text template: [`docs/MICROSOFT_APP_ID_REVIEW.md`](docs/MICROSOFT_APP_ID_REVIEW.md).

## CurseForge setup

CurseForge catalogue search and API-based file retrieval require a Core API key approved for the intended use by CurseForge. Set it as an environment variable:

```powershell
setx NEXUS_CURSEFORGE_API_KEY "YOUR-CURSEFORGE-API-KEY"
```

Do not commit API keys to GitHub. Some CurseForge files may be unavailable to third-party API clients due to author distribution settings. Modrinth `.mrpack` import does not require a CurseForge key.

## Data and privacy

NEXUS stores launcher settings and instance metadata locally. Microsoft refresh tokens are stored using Windows Credential Manager; account display names, UUIDs, and XUID metadata are stored in the local launcher configuration. Minecraft process output is written to an instance-local `nexus-launch.log` file.

The current source does not include a NEXUS account backend or an analytics/telemetry SDK. It does contact Microsoft, Xbox, Minecraft, Mojang, mod-loader, Java-runtime, and modpack services as needed for sign-in and downloads. CurseForge API requests use the API key supplied by the user.

Read [`PRIVACY.md`](PRIVACY.md) for the current source-level privacy description. This document should be reviewed and updated before distributing a public binary.

## Security

- Never commit Microsoft tokens, API keys, `.env` files, launcher settings, or personal game data.
- OAuth is performed in the system browser with PKCE and a validated `state` value.
- Report security issues privately where possible. See [`SECURITY.md`](SECURITY.md).

## License

This project is distributed under the MIT License. See [`LICENSE`](LICENSE).

## Contributing and feedback

This is an experimental project. Bug reports should include:

- Windows version and architecture;
- Rust version (`rustc --version` and `cargo --version`);
- the exact command that failed;
- the complete compiler or runtime error;
- relevant `nexus-launch.log` output after removing usernames, access tokens, UUIDs, and other private data.

Do not publish account tokens, API keys, or private logs in public issues.
