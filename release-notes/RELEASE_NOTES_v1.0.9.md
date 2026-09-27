# HDR Auto-Switch v1.0.9 — Game Scanner Fixes, Storefront Restoration & Path Normalization

> **Bugfix and reliability release: fixes Issue #5 by restoring storefront game discovery for Steam and Xbox in the SDR section, eliminates non-game desktop software (like AIDA64 and AMD Ryzen Master) from Windows registry scans, and strips verbatim `\\?\` UNC prefixes to eliminate bogus `UPDATE PATH` badges.**

---

### 🛠️ What's Fixed & Improved in v1.0.9

#### 1. 🎮 Restored Storefront Game Discovery (Steam, Xbox, Epic, GOG)
* In v1.0.7, strict automatic authority matching caused storefront games without pre-configured catalog HDR profiles to be omitted from the scan results entirely.
* **v1.0.9 restores full SDR discovery**:
  - Games installed on Steam (via library manifests) and Xbox (via `XboxGames`) that are not present in the HDR catalog are now correctly identified and populated in the **"Other Installed Games - SDR (Compatible with RTX HDR / Mods)"** section.
  - Games in this section remain disabled by default (`[AUTO OFF]`), allowing users to easily select and enable them for RTX HDR, Special K, or custom reshade mods with a single click.
  - Added smart candidate executable selection on `InstallEvidence` that locates the primary game runtime executable in the install directory while ignoring crash utilities, redistributables, and diagnostic tools.

#### 2. 🧹 Clean Windows Registry Scanning & Desktop App Filtering
* Resolved a regression where scanning Windows uninstall keys enumerated general desktop software (e.g. AIDA64 Extreme, AMD Ryzen Master, 7-Zip, Discord) and categorized them as games.
* **Strict Non-Game Filtering**:
  - Implemented `is_known_non_game_software` to immediately bypass hardware monitoring utilities, developer suites, office applications, and communication tools.
  - Added `is_valid_windows_game` to ensure only recognized game directories and publisher roots are scanned.
  - Added guards rejecting generic binaries such as `setup.exe`, `installer.exe`, and `unins000.exe`.

#### 3. 🏷️ Verbatim Path Normalization (Fix for Bogus `UPDATE PATH` Badges)
* Fixed an issue where canonical path resolution introduced Windows verbatim path prefixes (`\\?\` or `\\?\UNC\`).
* Because stored library paths in `config-v2.json` did not contain this prefix, the frontend detected a path mismatch on scan and erroneously displayed the yellow **`UPDATE PATH`** status badge on existing games.
* Introduced `clean_path_display` across all authority and scanner result builders, ensuring clean standard Windows drive and UNC paths across the application.

#### 4. 🧪 Test Suite & Regressions
* **313 of 313 Rust backend tests** passing (`cargo test`).
* **91 of 91 Frontend unit tests** passing (`npm test`).
* **Clean TypeScript & Vite compilation** (`npm run build`).
* Added dedicated regression tests:
  - `steam_sdr_game_is_suggested_with_auto_off_and_clean_path`
  - `windows_registry_scanner_ignores_non_game_desktop_software`

---

### 📦 Downloads & Installation

| Asset | Size | Description |
| :--- | :--- | :--- |
| 💿 **[HDR Auto-Switch_1.0.9_x64-setup.exe](https://github.com/Soptik1290/hdr-autoswitch/releases/download/v1.0.9/HDR.Auto-Switch_1.0.9_x64-setup.exe)** | ~3.5 MB | Recommended NSIS installer with desktop & start menu shortcuts |
| 📦 **[HDR Auto-Switch_1.0.9_x64_en-US.msi](https://github.com/Soptik1290/hdr-autoswitch/releases/download/v1.0.9/HDR.Auto-Switch_1.0.9_x64_en-US.msi)** | ~14.3 MB | Windows Installer package (MSI) |

---

### 💻 Compatibility
* **OS**: Windows 10 (20H2+) / Windows 11 (21H2, 22H2, 23H2, 24H2)
* **Display**: At least one HDR10 / VESA DisplayHDR / OLED capable monitor
