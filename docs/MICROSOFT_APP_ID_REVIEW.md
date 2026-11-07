# Microsoft / Minecraft App ID setup and review

This guide is for the current NEXUS desktop source. Microsoft Entra app registration and Minecraft App ID approval are separate steps. Registering an Entra application alone does not guarantee that Minecraft Services will accept its Client ID.

## 1. Create the Microsoft Entra application

1. Open the [Microsoft Entra admin center](https://entra.microsoft.com/).
2. Go to **Identity → Applications → App registrations → New registration**.
3. Name the app `NEXUS Launcher`.
4. For the account type, select the option that supports **personal Microsoft accounts**. If the interface offers "Personal Microsoft accounts only", use that; otherwise choose the supported option that includes personal accounts.
5. Complete registration and copy the **Application (client) ID**.
6. Open **Authentication → Add a platform → Mobile and desktop applications**.
7. Add the system-browser redirect URI `http://localhost`.
8. Under advanced settings, enable **Allow public client flows** and save.

The current source uses the `consumers` Microsoft identity authority and a dynamic localhost port. Microsoft documents `http://localhost` for system-browser desktop sign-in and treats loopback ports as dynamic for this use. See the official [desktop app configuration guide](https://learn.microsoft.com/en-us/entra/identity-platform/scenario-desktop-app-configuration) and [redirect URI guidance](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url).

**Do not create a client secret.** NEXUS is a native desktop public client. A secret embedded in a distributed executable would not be confidential.

## 2. Configure NEXUS locally

In PowerShell, set the Application (client) ID:

```powershell
setx NEXUS_MICROSOFT_CLIENT_ID "PASTE-YOUR-CLIENT-ID-HERE"
```

Close and reopen PowerShell and NEXUS Launcher. This environment variable is local configuration; do not commit it to GitHub.

## 3. Request Minecraft App ID approval

Open the [Minecraft App ID Review entry point](https://aka.ms/mce-reviewappid). A developer report on Microsoft Q&A describes submitting the Client ID through this process and receiving confirmation that the ID had been approved for the Minecraft Services allow list. This is a community report, not a guarantee of approval or a complete official statement of review criteria: [Microsoft Q&A discussion](https://learn.microsoft.com/en-us/answers/questions/5984225/minecraft-services-http-403-invalid-app-registrati).

The form may change. Use the fields currently displayed on the official form. Where relevant, prepare:

- Application name: `NEXUS Launcher`
- Application (client) ID: the ID created in Microsoft Entra
- Platform: Windows desktop application
- Project/source URL: `https://github.com/YOUR-GITHUB-USERNAME/nexus-launcher`
- Purpose/justification: the text below

Do not invent a publisher, company, website, user count, release date, or production status. If a field does not apply, answer honestly or leave it blank when the form permits.

## 4. Suggested application description / justification

Use this as a starting draft and adapt it to the exact form questions. It accurately describes the project as an AI-assisted independent hobby project; it does not claim that the project is complete or already approved.

> NEXUS Launcher is an independent, early-stage Minecraft: Java Edition launcher for Windows, written in Rust with the Slint UI framework.
>
> I am developing this project as an independent hobby developer. I use AI-assisted development tools to help with implementation, debugging, and architecture. I personally define the requirements, direct the development, and run builds and tests on my Windows computer as the project progresses. The source code is available for review at: https://github.com/YOUR-GITHUB-USERNAME/nexus-launcher
>
> The launcher is intended to help users manage separate Minecraft Java Edition instances, game versions, and modpacks. For users who own Minecraft: Java Edition, I want to provide sign-in through Microsoft's system browser and the official Microsoft, Xbox, and Minecraft authentication services.
>
> The intended authentication flow is Microsoft OAuth Authorization Code with PKCE, followed by Xbox Live authentication, XSTS authentication, and Minecraft Services entitlement/profile verification. The launcher does not ask users to enter their Microsoft password into NEXUS. It does not intend to bypass ownership checks, provide unauthorized game access, or use another application's Client ID.
>
> This is an experimental project under active development. I am requesting review and approval of this application's Client ID for the intended legitimate Minecraft: Java Edition authentication flow. I understand that Entra app registration alone does not grant access to Minecraft Services and that this request is subject to your review.

## 5. After submitting

- Save the confirmation page or email for your records.
- Wait for a response; approval is not guaranteed and timing is not guaranteed.
- If Minecraft Services returns `403 Invalid app registration`, do not try to bypass it or use another launcher's Client ID. Check the review status and follow up through the official channel.
- Only test with an account that owns Minecraft: Java Edition.
