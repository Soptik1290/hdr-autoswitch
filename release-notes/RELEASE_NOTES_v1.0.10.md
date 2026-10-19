# HDR Auto-Switch v1.0.10 — Shared Executable Disambiguation & Multi-Game Support

> **Bugfix and usability release: resolves Issue #6 by allowing distinct games that share the same executable filename (such as `hl2.exe`, `metro.exe`, `xrEngine.exe`, `Shipping.exe`, etc.) to be scanned, imported, managed, and tracked independently without prompting to overwrite or update each other's installation path.**

---

### 🛠️ What's Fixed & Improved in v1.0.10

#### 1. 🎮 Independent Tracking for Games Sharing the Same Executable Name
* Many classic and modern game engines reuse a common runtime executable filename across multiple standalone titles:
  - **Source Engine**: *Half-Life 2*, *Portal*, *Half-Life 2: Episode One/Two*, *Team Fortress 2* (`hl2.exe`)
  - **4A Engine**: *Metro 2033 / Redux* and *Metro: Last Light / Redux* (`metro.exe`)
  - **X-Ray Engine**: *S.T.A.L.K.E.R.: Shadow of Chernobyl*, *Clear Sky*, and *Call of Pripyat* (`xrEngine.exe`)
* Previously, library lookup (`findLibraryApp` / `same_game`) matched solely on `exe_name` first, causing newly scanned titles sharing an executable name with an existing library entry to display a false **`UPDATE PATH`** badge and overwrite the existing game on import.
* **v1.0.10 disambiguates shared executable names**:
  - Entries sharing an executable name are now recognized as **distinct games** whenever they have different game titles and distinct installation paths (or different Steam AppIDs).
  - Genuine installation moves of the **same game** (matching title or Steam AppID moved to a new drive/folder) continue to display **`UPDATE PATH`** and update the existing row seamlessly.

#### 2. 📥 Batch Import Support for Shared-Executable Titles
* Updated `library::import_games` so selecting multiple distinct games that share an executable filename in the Scan Modal (e.g. importing *Half-Life 2* and *Portal*, or all three *S.T.A.L.K.E.R.* titles at once) succeeds cleanly and creates independent library rows for each installation.
* Conflicting selections for the *same* game or identical installation path remain safely rejected before mutating settings.

#### 3. 🗂️ Per-Installation Toggle, Removal & Runtime Matching
* Updated the **My Games** Grid and List views along with backend `toggle_app` and `remove_app` commands to scope operations by installation path (`path`) when present, ensuring that pausing or removing one shared-executable title (e.g. *Portal*) never affects another (e.g. *Half-Life 2*).
* Runtime foreground detection (`runtime_policy::resolve`) prioritizes exact normalized full executable paths, activating HDR only for the specific enabled game installation.

#### 4. 🧪 Test Suite & Regressions
* **314 of 314 Rust backend tests** passing (`cargo test`).
* **92 of 92 Frontend unit tests** passing (`npm test`).
* **Clean TypeScript & Vite compilation** (`npm run build`).
* Added dedicated regression tests:
  - `distinct_games_sharing_executable_name_can_be_imported_and_stored_separately`
  - `distinct games sharing the same executable name do not conflate or trigger false path updates`

---

### 📦 Downloads & Installation

| Asset | Size | Description |
| :--- | :--- | :--- |
| 💿 **[HDR Auto-Switch_1.0.10_x64-setup.exe](https://github.com/Soptik1290/hdr-autoswitch/releases/download/v1.0.10/HDR.Auto-Switch_1.0.10_x64-setup.exe)** | ~3.5 MB | Recommended NSIS installer with desktop & start menu shortcuts |
| 📦 **[HDR Auto-Switch_1.0.10_x64_en-US.msi](https://github.com/Soptik1290/hdr-autoswitch/releases/download/v1.0.10/HDR.Auto-Switch_1.0.10_x64_en-US.msi)** | ~14.3 MB | Windows Installer package (MSI) |

---

### 💻 Compatibility
* **OS**: Windows 10 (20H2+) / Windows 11 (21H2, 22H2, 23H2, 24H2)
* **Display**: At least one HDR10 / VESA DisplayHDR / OLED capable monitor
