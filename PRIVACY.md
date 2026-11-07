# Privacy Notice — NEXUS Launcher

Last updated: 7 November 2026

NEXUS Launcher is an independent, experimental desktop application. This notice describes the behavior visible in the current source code. It is not a promise that future versions will behave identically; this notice must be updated when the software changes.

## Information stored locally

The application stores the following data on the user's Windows device:

- launcher preferences, including selected profile, memory allocation, and Java path;
- instance metadata, such as instance name, Minecraft version, loader name, and game directory;
- account display name, Minecraft profile UUID, and Xbox XUID metadata;
- offline nickname selected by the user;
- Minecraft process output in the instance's `nexus-launch.log` file.

Microsoft refresh tokens are stored using the Windows Credential Manager integration provided by the `keyring` crate. Microsoft passwords are entered on Microsoft's own sign-in page in the system browser; NEXUS does not provide a password field for Microsoft credentials.

## Services contacted

Depending on the features used, NEXUS sends network requests to third-party services, including:

- Microsoft identity platform (`login.microsoftonline.com`) for OAuth sign-in and token renewal;
- Xbox authentication services (`user.auth.xboxlive.com` and `xsts.auth.xboxlive.com`);
- Minecraft Services (`api.minecraftservices.com`) for Minecraft authentication, entitlement, and profile checks;
- Mojang/Minecraft metadata, library, asset, and download hosts for game files;
- Fabric and Quilt metadata services;
- Forge and NeoForge Maven repositories;
- Eclipse Adoptium API and download hosts for Windows Java runtimes;
- CurseForge Core API and download sources when the user enables CurseForge features.

When the user connects to a multiplayer server, Minecraft itself communicates with that server according to the game's normal behavior.

## Data sent during Microsoft sign-in

When the user selects Microsoft sign-in, NEXUS opens the system browser and performs an OAuth Authorization Code flow with PKCE. Authentication tokens are then exchanged with Xbox and Minecraft Services to check access to Minecraft: Java Edition and retrieve the game profile. Those providers receive the information required by their authentication protocols.

NEXUS does not intentionally send Microsoft passwords to the NEXUS project maintainers.

## CurseForge API key

If configured, the user-provided `NEXUS_CURSEFORGE_API_KEY` is sent to the CurseForge Core API in requests needed for catalogue search and modpack file retrieval. The key is read from an environment variable and is not written to the launcher settings file by the current source.

## Telemetry and project servers

The current source tree does not include a NEXUS account backend, analytics SDK, or NEXUS-operated telemetry endpoint. This statement should be re-audited before each public release.

## Data sharing and retention

The project maintainers do not receive launcher settings, account tokens, or game logs through a NEXUS-operated backend in the current source. Local data remains on the user's device until the user removes it. Third-party providers process requests under their own terms and privacy notices.

## User controls

Users can remove local launcher data manually from the application data directory and remove stored credentials separately through Windows Credential Manager. Users can also revoke the application's access through their Microsoft account security settings. Removing the executable does not necessarily remove local configuration, game instances, logs, or credentials.

## Contact

For privacy questions, contact the repository owner through the GitHub profile associated with this repository. Do not include passwords, tokens, or API keys in public issues.
