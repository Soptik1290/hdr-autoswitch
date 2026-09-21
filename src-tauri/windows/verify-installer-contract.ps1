param([switch]$RequireIntegration)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$config = Get-Content -Raw (Join-Path $root 'src-tauri\tauri.conf.json') | ConvertFrom-Json
$lock = Get-Content -Raw (Join-Path $root 'package-lock.json') | ConvertFrom-Json -AsHashtable
$contract = Get-Content -Raw (Join-Path $PSScriptRoot 'installer-contract.json') | ConvertFrom-Json
$nsis = Get-Content -Raw (Join-Path $PSScriptRoot 'installer.nsi')
$wixText = Get-Content -Raw (Join-Path $PSScriptRoot 'main.wxs')
$module = Get-Content -Raw (Join-Path $root 'src-tauri\src\legacy_upgrade.rs')
$script:assertions = 0

function Assert-Contract([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:assertions++
}

Assert-Contract ($config.bundle.targets -eq 'all') 'Do not silently drop a shipped installer format.'
Assert-Contract ($lock.packages['node_modules/@tauri-apps/cli'].version -eq $contract.product.tauriCli) 'Re-review installer templates after a Tauri CLI change.'
Assert-Contract ($config.productName -eq $contract.product.name) 'Product-name ownership contract changed.'
Assert-Contract ($config.identifier -eq $contract.product.identifier) 'Application ownership identity changed.'
Assert-Contract ($config.bundle.windows.nsis.installMode -eq 'currentUser') 'NSIS interactive-user scope changed.'
Assert-Contract ($config.bundle.windows.nsis.template -eq 'windows\installer.nsi') 'Safe NSIS template is not selected.'
Assert-Contract ($config.bundle.windows.wix.template -eq 'windows\main.wxs') 'Safe MSI template is not selected.'
Assert-Contract ($config.bundle.windows.wix.upgradeCode.ToUpperInvariant() -eq $contract.evidence.msi.upgradeCode.Trim('{}')) 'MSI upgrade identity differs from the shipped product.'
Assert-Contract ($module.Contains($contract.evidence.msi.upgradeCode)) 'Runtime MSI upgrade identity differs from evidence.'
Assert-Contract ($module.Contains($contract.evidence.msi.executableComponent)) 'Runtime MSI component identity differs from evidence.'
Assert-Contract ($nsis -match '(?m)^RequestExecutionLevel user\r?$') 'NSIS must run as the interactive user.'
Assert-Contract ($nsis -match '(?m)^!macroundef CheckIfAppIsRunning\r?$') 'The Tauri basename-kill macro must be removed.'
Assert-Contract ($nsis -notmatch '(?im)^\s*!insertmacro\s+CheckIfAppIsRunning\b') 'An unsafe Tauri process macro was invoked.'
Assert-Contract ($nsis -notmatch '(?im)^\s*(nsis_tauri_utils::(?:Kill|Find)Process|ExecWait.*uninstall\.exe)') 'NSIS must not kill by name or invoke an old uninstaller.'
Assert-Contract ($nsis -notmatch '(?im)^\s*DeleteRegValue\s+HKCU\s+["''].*\\Run["'']') 'Run entries may only be retired through verified ownership.'
Assert-Contract ($nsis.IndexOf('Section "Verify orderly handoff"') -lt $nsis.IndexOf('Section "WebView2 runtime"')) 'Preflight must precede installer effects.'
Assert-Contract ($nsis.Contains('--hdr-installer-preflight nsis')) 'NSIS preflight command is missing.'
Assert-Contract ($nsis.Contains('--hdr-installer-uninstall nsis')) 'NSIS uninstall ownership check is missing.'
Assert-Contract ($nsis.Contains('SetErrorLevel 1')) 'Unattended NSIS failures must be explicit.'

$uninstall = [regex]::Match($nsis, '(?ms)^Section "Uninstall"\r?\n(.*?)^SectionEnd').Groups[1].Value
$prepare = $uninstall.IndexOf('--hdr-installer-retirement-prepare nsis')
$payload = $uninstall.IndexOf('!insertmacro HdrDeleteUninstallPayload "$INSTDIR\uninstall.exe"')
$retire = $uninstall.IndexOf('--hdr-installer-retirement-commit nsis')
$cleanup = $uninstall.IndexOf('Rename "$HdrUninstallRecovery" "$PLUGINSDIR\hdr-uninstall-complete"')
Assert-Contract ($prepare -ge 0 -and $payload -gt $prepare -and $retire -gt $payload -and $cleanup -gt $retire) 'Exact registry authority and recovery must survive payload deletion and checked retirement.'
Assert-Contract ($uninstall -notmatch '\bDeleteReg(?:Key|Value)\b') 'NSIS must not bypass checked registry retirement with unchecked metadata commands.'
Assert-Contract ($uninstall.Contains('--hdr-installer-retirement-restore nsis') -and $uninstall.Contains('Installation metadata recovery was NOT confirmed')) 'Denied registry rollback must retain a usable recovery helper with explicit instructions.'
Assert-Contract ($module.Contains('fn retire_nsis_registration(') -and $module.Contains('fn restore_nsis_registration(') -and $module.Contains('verify_recovery_pair(target, recovery)?;')) 'Retirement and recovery require checked metadata and complete cleanup executables.'

# A lock on only the old uninstaller must not allow the remaining install
# section or success/launch callbacks to run. Require the error guard directly
# after WriteUninstaller, before another instruction can clear its error flag.
$uninstallerGuard = [regex]::Match(
    $nsis,
    '(?ms)^[ \t]*ClearErrors\r?\n[ \t]*WriteUninstaller "\$INSTDIR\\uninstall\.exe"\r?\n[ \t]*\$\{If\} \$\{Errors\}\r?\n(?<failure>.*?)^[ \t]*\$\{EndIf\}\r?\n[ \t]*WriteRegStr HKCU'
)
Assert-Contract ($uninstallerGuard.Success) 'Uninstaller-only sharing failure must be checked before registry writes or another ClearErrors.'
$uninstallerFailure = $uninstallerGuard.Groups['failure'].Value
Assert-Contract ($uninstallerFailure -match '(?m)^[ \t]*SetErrorLevel 1\r?$') 'Failed uninstaller replacement must return a failure exit code.'
Assert-Contract ($uninstallerFailure -match '(?m)^[ \t]*Abort "' -and $uninstallerFailure -notmatch 'ClearErrors|\bExec(?:Wait)?\b') 'Failed uninstaller replacement must abort without launching anything.'

# Handlebars control directives are text nodes and do not prevent XML parsing.
[xml]$wix = $wixText
$ns = New-Object System.Xml.XmlNamespaceManager($wix.NameTable)
$ns.AddNamespace('w', 'http://schemas.microsoft.com/wix/2006/wi')
Assert-Contract ($null -eq $wix.SelectSingleNode('//w:Property[@Id="ARPNOMODIFY"]', $ns) -and $null -ne $wix.SelectSingleNode('//w:UIRef[@Id="WixUI_InstallDir"]', $ns)) 'Keep ARPNOMODIFY supplied by WixUI_InstallDir; redeclaring it causes LGHT0091.'
$property = $wix.SelectSingleNode('//w:Property[@Id="MSIRESTARTMANAGERCONTROL"]', $ns)
Assert-Contract ($null -ne $property -and $property.Value -eq 'DisableShutdown') 'MSI must not ask Restart Manager to close applications.'
$guard = $wix.SelectSingleNode('//w:CustomAction[@Id="HdrUpgradePreflight"]', $ns)
Assert-Contract ($null -ne $guard -and $guard.Return -eq 'check' -and $guard.Execute -eq 'immediate' -and $guard.Impersonate -eq 'yes') 'MSI preflight must synchronously block replacement in caller context.'
Assert-Contract ($guard.ExeCommand.Contains('--hdr-installer-preflight msi')) 'MSI must enter the check-only executable mode.'
$sequence = $wix.SelectSingleNode('//w:InstallExecuteSequence/w:Custom[@Action="HdrUpgradePreflight"]', $ns)
Assert-Contract ($null -ne $sequence -and $sequence.Before -eq 'InstallValidate') 'MSI preflight must run before validation/removal.'
Assert-Contract ($wixText -notmatch '<(?:\w+:)?CloseApplication\b|KillProcess|taskkill|Stop-Process') 'MSI must not contain process-termination actions.'
Assert-Contract ($module -notmatch '\b(?:TerminateProcess|ExitWindowsEx|SendMessageW|PostMessageW)\s*\(') 'The legacy handoff must not force termination or treat window-close as Quit.'
Assert-Contract ($module -match '(?s)pub fn configure_autostart\(enable: bool\) -> Result<\(\), String> \{\s*apply_autostart_change\(enable\)\.map\(drop\)\s*\}') 'Reconcile flows must share checked partial-failure rollback, not bypass the transaction API.'

if ($RequireIntegration) {
    $lib = Get-Content -Raw (Join-Path $root 'src-tauri\src\lib.rs')
    $commands = Get-Content -Raw (Join-Path $root 'src-tauri\src\commands.rs')
    $dispatch = $lib.IndexOf('legacy_upgrade::installer_command()')
    $builder = $lib.IndexOf('tauri::Builder::default()')
    Assert-Contract ($dispatch -ge 0 -and $builder -gt $dispatch) 'installer_command must dispatch before Tauri initialization.'
    Assert-Contract ($lib -notmatch '\.plugin\(\s*tauri_plugin_autostart::') 'The generic autostart plugin would bypass the owned startup transaction.'
    Assert-Contract ($lib.Contains('legacy_upgrade::check_predecessor()')) 'Runtime startup/recheck must use the verified predecessor guard.'
    $closureTransaction = $commands.Contains('legacy_upgrade::configure_autostart_with_commit(')
    $receiptTransaction = $commands.Contains('legacy_upgrade::apply_autostart_change') -and
        $commands.Contains('legacy_upgrade::AutostartChange::rollback') -and
        $module.Contains('pub fn apply_autostart_change(') -and
        $module.Contains('pub struct AutostartChange')
    Assert-Contract ($closureTransaction -or $receiptTransaction) 'Canonical startup patches need exact-byte checked rollback, not an effective-state boolean.'
    Assert-Contract ($commands -notmatch 'configure_autostart\(\s*previous\s*\)') 'Boolean startup rollback can delete an unchanged Windows-disabled registration.'
}

Write-Output "$script:assertions isolated installer-contract assertions passed. No app, registry, process, or installer was exercised."
