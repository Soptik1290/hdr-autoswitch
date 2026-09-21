# 🎮 HDR Auto-Switch for Windows

<div align="center">

![HDR Auto-Switch Banner](docs/screenshot.png)

**Automatic, lightweight HDR display switcher for Windows 10 and 11.**\
*No more manual `Win + Alt + B` or monitor blackouts before and after every gaming session.*

[![Version](https://img.shields.io/badge/Version-v1.0.8-5accf5?style=for-the-badge)](https://github.com/Soptik1290/hdr-autoswitch/releases/tag/v1.0.8)
[![Windows](https://img.shields.io/badge/Platform-Windows%2010%20%7C%2011-0078D6?style=for-the-badge&logo=windows&logoColor=white)](https://github.com/Soptik1290/hdr-autoswitch)
[![Tauri v2](https://img.shields.io/badge/Tauri-v2-FFC131?style=for-the-badge&logo=tauri&logoColor=black)](https://v2.tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-Backend-orange?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![React 19](https://img.shields.io/badge/React-19-61DAFB?style=for-the-badge&logo=react&logoColor=black)](https://react.dev/)
[![License](https://img.shields.io/badge/License-MIT-green?style=for-the-badge)](LICENSE)
[![Ko-fi](https://img.shields.io/badge/Ko--fi-Buy%20me%20a%20coffee-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white)](https://ko-fi.com/s0pt1k)

[**Download Latest Release (.exe Installer)**](https://github.com/Soptik1290/hdr-autoswitch/releases/latest) • [**Release Notes**](release-notes/RELEASE_NOTES_v1.0.8.md) • [**Support on Ko-fi**](https://ko-fi.com/s0pt1k) • [**Report Bug**](https://github.com/Soptik1290/hdr-autoswitch/issues)


</div>

---

## ⚡ Why HDR Auto-Switch?

Windows HDR looks breathtaking in games and movies, but running desktop apps and web browsers in constant HDR often causes washed-out SDR colors, unnecessary power draw, and panel wear on OLED monitors.

**HDR Auto-Switch** runs quietly in your system tray and monitors window focus using native Windows OS events. The moment you launch or switch into an HDR-enabled game, your monitor instantly engages HDR10 / Rec.2020. When you close the game or return to your desktop, it gracefully switches back to SDR BT.709.

---

## ✨ Key Features

### ⚡ 1. Event-Driven Switching with a Missed-Event Watchdog
HDR Auto-Switch uses the native `SetWinEventHook` `EVENT_SYSTEM_FOREGROUND`
notification as its primary trigger. A lightweight once-per-second watchdog only
compares the current foreground process ID and runs the full controller logic when
that ID changes, recovering if Windows drops a foreground event after startup or
resume. Incomplete observations retry with bounded backoff and a slow rearm;
successful observations keep the cheap cached-process path. Settings changes
always recheck current authorization, including after a delayed observation.

### 🔍 2. Automated Multi-Drive Game Scanner
* **Deep Multi-Drive Discovery**: Automatically scans all connected storage drives (`C:`, `D:`, `E:`, etc.) via Steam's `libraryfolders.vdf` and `appmanifest_*.acf` manifests, Epic Games Launcher manifests (`%ProgramData%\Epic`), GOG Galaxy, and Windows Registry.
* **Provider-specific Executable Support**: Automatic selection requires a provider-authorized executable that exists locally. Finding an `.exe` recursively, matching a title, or knowing a Steam AppID alone is not executable authority. Unsupported or conflicting evidence remains unresolved rather than guessed.
* **Categorized & Pre-Selected Results**:
  - **HDR Supported Games (Top)**: Verified HDR titles stay in this group regardless of selection. Automatic game detection controls initial selection, not the support classification.
  - **Other Installed Games (Bottom)**: Games with unverified HDR support are unselected by default; this is not a claim that they are SDR-only. You can explicitly select them for RTX HDR or community mods.
* **Smart Library State Badges**:
  - `★ NEW`: Newly discovered HDR games ready to be added.
  - `✓ IN LIBRARY`: Previously tracked games that are already up to date.
  - `⚡ UPDATE PATH`: Automatically detects when a game has moved to another drive or folder and updates its path.

### 📂 3. Native File Picker ("Browse...") & Drag & Drop
* **Native Windows File Picker**: Click **"Browse... / Procházet..."** in the Manual Add modal to select any `.exe` using the standard Windows 64-bit file dialog.
* **Global Drag & Drop**: Drag any `.exe` file from Windows Explorer directly into the application window. The app automatically inspects the binary, queries the database, and pre-fills the title and HDR support tier.
* Editing the primary executable after browsing clears an incompatible picked path. Add, import, update, and repair also validate primary/path consistency in the backend.

### 🛡️ 4. Flexible HDR Deactivation Policies
* **Only when game exits (Recommended)**: Keeps HDR active during Alt+Tab (e.g. checking Discord, Spotify, or a walkthrough in your browser). Completely eliminates monitor renegotiation blackouts, signal delay, and DirectX swapchain desync. Switches back to SDR immediately when the game closes.
* **Deactivate on Alt+Tab (with Debounce)**: Reverts to SDR when leaving the game window after a configurable delay (0 to 10 seconds).

### 📚 5. Multi-Source Verified Database (Steam Curator, HDR Gamer, PCGamingWiki)
* **Steam AppID Pairing**: Pre-linked Steam AppIDs identify catalog candidates; automatic selection also requires that provider's supported game executable on disk.
* Comprehensive catalog of **1,027+ verified PC titles** including Native HDR (*Silent Hill 2, Alan Wake 2 & Remastered, Resident Evil 2/3/4/7/Village, Cyberpunk 2077, Black Myth: Wukong, Borderlands GOTY Enhanced, Baldur's Gate 3, Ghostrunner 1 & 2, Mass Effect Legendary Edition*), Windows Auto HDR, and HDR Gamer calibration profiles.
* Built-in 1-click online synchronization with PCGamingWiki API and GitHub master database.


### 💽 6. Drive Migration & Disk Path Verification
* Real-time path checking detects if an executable has been moved across drives or uninstalled, marking it with a `[FILE NOT FOUND]` badge and prompting you to run the scanner to refresh the location.
* Explicitly importing a moved game updates its path and available launcher metadata. Conflicting installations or ambiguous library owners require selection or repair rather than silently choosing a row.

### 🖥️ 7. Native Win32 DisplayConfig API & Per-Monitor Targeting
* Interacts directly with GPU display drivers via native Windows `QueryDisplayConfig` / `SetDisplayConfig` APIs.
* Operates independently of Xbox Game Bar, without simulated keyboard shortcuts or an unscoped keyboard fallback.
* Choose all connected HDR displays or a specific display. The selection uses the Windows device-interface path, not an adapter address that changes after a reboot.
* A disconnected, ambiguous, or unidentifiable selected display stays selected and unavailable. The app never substitutes another display or silently changes the selection to All.

### 🌐 8. Bilingual Interface & System Tray
* Automatically detects system language: launches in **Czech** for Czech/Slovak systems and **English** for all others, with an instant 1-click header toggle (`CZ` / `EN`).
* Silent autostart on Windows boot and minimization to the system tray.
* Spacious, modern **1280 × 720** cyberpunk UI with monospace typography (`Kode Mono`) and optional GSAP CRT scanlines.

---

## 📦 Installation & Download

### Option 1: Pre-built Windows Installer (Recommended)
Download the latest installer (`.exe` setup or `.msi`) from the [**Releases Page**](https://github.com/Soptik1290/hdr-autoswitch/releases/latest).

1. Run `HDR Auto-Switch_1.0.8_x64-setup.exe`.
2. Follow the installer instructions (creates desktop and start menu shortcuts).
3. The app will detect your connected displays automatically.
4. Click **"Scan PC for Games"** on the My Games tab to populate your library.

### ⚙️ Settings Migration & Technical Invariants

<details>
<summary><b>Click to expand architecture details (Settings, Storefronts & Recovery)</b></summary>

#### Settings & Migration
* **Machine-Local Storage**: Settings are stored per-machine in `%LOCALAPPDATA%\com.soptik.hdr-autoswitch\config-v2.json`. The `controller.lock` file coordinates access across running instances.
* **Legacy Import**: Older settings from `%APPDATA%\HDRAutoSwitch\config.json` can be imported on first launch without modifying the original file.
* **Persistent Display Identity**: Monitor selection binds to durable Windows display device instances. Primary monitor detection uses Windows GDI primary-source metadata.
* **Atomic Transactions**: Settings saves use flushed staging files with checksums and atomic file replacement to prevent corruption.

#### Storefronts, Executables & Quarantine
* **Provider Authority**: Executable support is **provider-specific**, **not a universal storefront mapping**. Only embedded executable authority can authorize automatic matching or canonical row enrichment.
* **Xbox Games**: Discovery covers accessible local `XboxGames` via bounded `MicrosoftGame.config` parsing without probing protected packages. Competing files stay **unresolved rather than guessed**. The verified AOE3 Xbox binding selects `AoE3DE.exe` (**not a claim of live AOE3 HDR verification**).
* **Crash Reporter Quarantine**: Known shared helpers or crash utilities (`GameLaunchHelper.exe`, `BsSndRpt.exe`, `BugSplat.exe`, etc.) are **quarantined at runtime** with status alerts, allowing one-click repair that **preserves other choices**.
* **Inconsistent Legacy Bindings**: A saved primary whose filename does not match its path is also quarantined, including its historical aliases. The derived warning identifies the affected row without rewriting saved data. Repair discards historical aliases, preserves other preferences, and rejects collisions with enabled or disabled owners.
* **Legacy Helper Aliases**: A healthy primary may retain old helper aliases, which never authorize runtime matching. Confirmed add/import updates remove only exact permanently excluded helper aliases while preserving other aliases and the existing explicit-update metadata/enablement policy. Incoming helpers still fail validation; background enrichment never silently removes saved aliases.
* **Precise Path Matching**: Runtime matching prioritizes exact normalized paths to prevent spoofing or misattribution between different game editions.
* **Row-Scoped Commands**: Toggle, delete, and repair use a snapshot-local index plus primary/path identity and a library-generation fence. Add/import updates require a unique path-aware owner. Even identical legacy duplicate rows can be removed individually; no persisted row-ID migration is needed. Rejected actions leave config bytes and revision/generation unchanged.
* **Catalog Identity & Synchronization**: Explicit embedded name aliases canonicalize proven equivalent products (including Dead Space and Resident Evil remake names and Baldur's Gate 3's DX11 label). Distinct products sharing a basename remain ambiguous. Sync and reload share one merge policy: only support tier and notes overlay embedded records; executable, type, product, and storefront authority remain authored. Serialized, flushed atomic cache publication prevents partial writes and stale startup/manual sync results from becoming current.

#### Display Control & Cleanup
* **Native Switching**: Direct per-display Win32 HDR switching without sending simulated hotkeys.
* **Safe Cleanup**: Automatic restoration only reverts HDR changes that were verified to be initiated by the application, leaving existing user HDR states intact.
* **Inventory Revisions**: Display inventory and status carry monotonic revisions, including name, capability, connection, and known-state changes that leave aggregate HDR unchanged. An idle read-only probe runs at most every five seconds, independently of the cheap one-second foreground watchdog. Identical observations remain quiet; stale frontend replies cannot replace newer observations.
* **Current Manual Warnings**: A verified scoped recovery retires the corresponding manual-failure warning, not unresolved uncertainty, other displays' failures, or controller conflicts.
* **Cross-Origin Manual Ordering**: GUI and tray results share actor-issued manual revisions, while every request also carries an ephemeral client identity and monotonic client sequence. Actor admission echoes this identity and refuses duplicate/older requests from the same client and scope. Snapshots preserve each client's latest per-scope completion proof. A submission/transport failure is retired only by a matching result or a later same-client, same-scope completed retry, never by an actor revision that might describe an older missed request. Unknown GUI delivery failures therefore remain visible after unrelated tray results; retry from the original control to reconcile them. Native conditions still follow actor status ordering, so late replies cannot hide newer failures, controller conflicts, or uncertainty. No request identity is persisted in settings.
* **Process Ownership**: Native process handles are immediately RAII-owned, including failed queries and duplicate windows. Process listings distinguish same-basename installations by normalized full path.
* **Recoverable Uninstallation**: Payload deletion and owned registry retirement are checked. Registry deletion/readback failures are explicit failures; verified cleanup executables and transaction evidence remain available for recovery instead of leaving uninstall registration pointing to missing files.

</details>

---

## 🛠️ Tech Stack & Architecture

| Layer | Technology | Details |
|---|---|---|
| **Runtime** | [Tauri v2](https://v2.tauri.app/) | Lightweight native desktop framework |
| **Backend** | Rust 2021 | `windows-rs` (Win32 DisplayConfig & WinEventHook), `rfd` (Native dialogs), `reqwest` |
| **Frontend** | React 19, TypeScript | Strict type checking, Vite 8, Tailwind CSS v4 |
| **Animation** | GSAP | SVG displacement filters, text scramble, and CRT scanlines |
| **Icons & Typography** | Lucide React & `Kode Mono` | High-contrast cyberpunk aesthetic |

---

## 💻 Developer Quickstart

### Prerequisites
* [Node.js](https://nodejs.org/) (v22.18+ or v24 LTS, including native TypeScript support for tests)
* [Rust](https://www.rust-lang.org/) (stable toolchain)
* Windows 10 (build 19041+) or Windows 11 with an HDR-capable display

### Development Mode
```bash
# Clone the repository
git clone https://github.com/Soptik1290/hdr-autoswitch.git
cd hdr-autoswitch

# Install dependencies
npm install

# Launch in live dev mode with hot reload
npm run tauri dev
```

### Production Build
To compile the release binaries and generate Windows NSIS and MSI installers:
```bash
npm run tauri build
```
Output files will be generated in:
- `src-tauri/target/release/bundle/nsis/HDR Auto-Switch_1.0.8_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/HDR Auto-Switch_1.0.8_x64_en-US.msi`

### Checks without changing real HDR or installed settings

```powershell
npm ci
npm run build
npm test
cargo test --manifest-path .\src-tauri\Cargo.toml --lib
```

Rust tests use temporary settings directories and mocked display operations.
The Node tests cover mutation ordering/history fences and English/Czech rendering,
including every shipped catalog description. Frontend labels/descriptions, tray
labels, and file-picker titles follow the selected language; game names and
unknown external catalog descriptions are preserved. Native diagnostic details
retain their backend or Windows language.

Run the isolated NSIS source/model regressions with `node --test .\scripts\test-nsis-uninstall.mjs`. These checks simulate sharing locks, missing files, registry deletion/readback failures after payload deletion, and recovery without executing an installer or touching Windows metadata. Rust regressions use injected registry operations, display backends, owned process-handle doubles, and isolated cache paths. Template compilation and disposable-VM uninstall/upgrade qualification remain separate release gates.

For browser-only UI checks, run `npm run dev` and open
`http://localhost:1420/tests/ui-fixture.html`. This uses Tauri's IPC mocks and
synthetic settings/displays, not the native application. The fixture accepts
`?mode=first_run`, `?mode=import_available`, `?mode=recovery_required`,
`?mode=unsupported_schema`, and `?failSave=1`. It is not included in the production
bundle. `?mixed=1` exercises an All scope containing both HDR and SDR displays.
`?aliasMerge=1` starts with a disabled `renderer.exe` library row: adding the
catalog's `game.exe` enables that canonical row and records the alias. Catalog
removal targets the generation-fenced `renderer.exe` row and path, and the running-process view recognizes the
enabled alias rather than offering a duplicate Add.

Debug builds can exercise the real native window and read the live display
inventory without using the installed profile:

```powershell
$env:HDR_AUTOSWITCH_SAFE_TEST_DIR = 'C:\absolute\temporary\test-directory'
.\src-tauri\target\debug\tauri-app.exe
```

This debug-only mode stores settings under the supplied directory and blocks HDR
writes, autostart changes, the foreground hook, and background/manual catalog synchronization.
Manual controls remain blocked in setup and recovery modes, too. Release builds
ignore this environment variable.

These checks do not certify real-monitor reboot/hotplug behavior, power-loss
durability, or NSIS/MSI upgrades of installed older releases. Those scenarios need
separate Windows VM/hardware release validation. The display query and setter
remain separate Windows operations, so native topology changes cannot be made
fully atomic by the application.

---

## 🤝 Community & Contributors

* A special thank you to **[@Karltang93](https://github.com/Karltang93)** for contributing [PR #2](https://github.com/Soptik1290/hdr-autoswitch/pull/2) (persistent monitor identification across restarts, transactional settings storage) and [PR #3](https://github.com/Soptik1290/hdr-autoswitch/pull/3) (provider executable authority, Xbox config resolution, helper quarantine, and bounded foreground retries)!

---

## ☕ Support the Project

If you find HDR Auto-Switch helpful and want to support ongoing development, maintenance, and catalog updates, you can support the project on Ko-fi!

[![Support on Ko-fi](https://img.shields.io/badge/Ko--fi-Support%20the%20Project-FF5E5B?style=for-the-badge&logo=ko-fi&logoColor=white)](https://ko-fi.com/s0pt1k)

---

## 📄 License

Distributed under the [MIT License](LICENSE).  
Developed with ❤️ for the PC and OLED gaming community.
