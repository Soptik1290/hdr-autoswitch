import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const template = readFileSync(
  new URL("../src-tauri/windows/installer.nsi", import.meta.url),
  "utf8",
);
const helper = readFileSync(
  new URL("../src-tauri/src/legacy_upgrade.rs", import.meta.url),
  "utf8",
);
const contract = JSON.parse(readFileSync(
  new URL("../src-tauri/windows/installer-contract.json", import.meta.url),
  "utf8",
));
const uninstall = template.match(/Section "Uninstall"\r?\n([\s\S]*?)SectionEnd/)[1];
const main = "tauri-app.exe";
const uninstaller = "uninstall.exe";
const resource = "assets\\catalog.json";
const sidecar = "support.exe";
const cleanupFiles = [main, uninstaller];
const checkedRetirement = uninstall.includes("--hdr-installer-retirement-commit nsis");

function macro(name) {
  const match = template.match(new RegExp(`!macro ${name} [^\\r\\n]+\\r?\\n([\\s\\S]*?)!macroend`));
  assert.ok(match, `missing ${name}`);
  return match[1].replace(/^[ \t]*;[^\r\n]*/gm, "");
}

function ordered(text, fragments) {
  let position = -1;
  for (const fragment of fragments) {
    const next = text.indexOf(fragment, position + 1);
    assert.ok(next > position, `missing or out-of-order: ${fragment}`);
    position = next;
  }
}

function renderFixture(section) {
  const items = {
    resources: ["assets\\catalog.json"],
    binaries: ["support.exe"],
    resources_ancestors: ["assets"],
  };
  return section
    .replace(/\{\{#each (\w+)\}\}([\s\S]*?)\{\{\/each\}\}/g, (_, name, body) => {
      assert.ok(items[name], `unexpected fixture collection: ${name}`);
      return items[name].map(value => body
        .replaceAll("{{this.[1]}}", value)
        .replaceAll("{{this}}", value)).join("");
    })
    .replaceAll("${MAINBINARYNAME}", "tauri-app");
}

const rendered = renderFixture(uninstall);
const removals = [...rendered.matchAll(
  /^\s*(Delete|!insertmacro HdrDeleteUninstallPayload) "(\$INSTDIR\\[^"]+)"\s*$/gm,
)].map(([, command, path]) => ({
  path: path.replace(/\\+/g, "\\").slice("$INSTDIR\\".length),
  checked: command !== "Delete",
}));
const backedUp = [...rendered.matchAll(
  /!insertmacro HdrBackupUninstallFile "([^"]+)"/g,
)].map(([, name]) => name);
const restored = [...rendered.matchAll(
  /!insertmacro HdrRestoreUninstallFile "([^"]+)"/g,
)].map(([, name]) => name);
const restoreDirectory = macro("HdrRestoreUninstallFile").match(
  /!insertmacro HdrCreateUninstallDirectory \$HdrUninstallRestoreDir "([^"]+)"/,
);
assert.ok(restoreDirectory, "model requires an explicit restore staging parent");
const restoreParent = restoreDirectory[1];

// Fault-injection model: consume the actual template's payload order/guard and
// recovery declarations. No installer, executable, filesystem write, or registry
// operation is executed; static assertions below bind the model to NSIS branches.
function simulate({
  initialFiles,
  missing = [],
  locked = [],
  readbackPresent = [],
  readbackDenied = [],
  backupFailure,
  restoreCopyFailure = [],
  restoreFailure = [],
  restoreDirectoryFailure = false,
  temporaryVolume = "C:",
  installationVolume = "C:",
  replacementOnFailure,
  replacementBeforeRename,
  recoveryMoveFails = false,
  preflightFails = false,
  selfCopy = true,
  retirementFailure,
  registryRollbackFails = false,
} = {}) {
  const files = new Map(initialFiles ?? [
    [main, "original main bytes"],
    [uninstaller, "original uninstaller bytes"],
    [resource, "bundled resource"],
    [sidecar, "bundled sidecar"],
    ["personal.txt", "user data"],
    ["assets\\custom.ini", "user settings"],
  ]);
  for (const name of missing) files.delete(name);
  const initial = new Map(files);
  const recovery = new Map();
  const restoreStaging = new Map();
  const restoreVolume = new Map([
    ["$TEMP", temporaryVolume],
    ["$INSTDIR", installationVolume],
  ]).get(restoreParent);
  assert.ok(restoreVolume, "unexpected restore staging parent");
  let restoreDirectoryOwned = false;
  const events = [];
  let metadata = true;
  let shortcuts = true;
  let exitCode = 0;
  let retirementStarted = false;
  let productPath = true;
  let uninstallRegistration = true;

  function result() {
    return {
      files, initial, recovery, restoreStaging, restoreDirectoryOwned,
      temporaryVolume, restoreVolume, installationVolume,
      events, metadata, shortcuts, exitCode, productPath, uninstallRegistration,
    };
  }

  function fail() {
    exitCode = 1;
    if (replacementOnFailure) {
      files.set(replacementOnFailure, "concurrent unrelated replacement");
    }
    for (const name of restored) {
      if (!files.has(name) && recovery.has(name)) {
        events.push(`restore:${name}`);
        if (restoreDirectoryFailure) break;
        restoreDirectoryOwned = true;
        if (restoreCopyFailure.includes(name)) {
          restoreStaging.set(name, "incomplete staged copy");
          continue;
        }
        restoreStaging.set(name, recovery.get(name));
        if (replacementBeforeRename === name) {
          files.set(name, "concurrent unrelated replacement");
        }
        if (restoreVolume === installationVolume && !restoreFailure.includes(name) && !files.has(name)) {
          files.set(name, restoreStaging.get(name));
          restoreStaging.delete(name);
        }
      }
    }
    if (retirementStarted) {
      events.push("restore-registration");
      if (!registryRollbackFails && cleanupFiles.every(name => files.get(name) === recovery.get(name))) {
        productPath = true;
        uninstallRegistration = true;
        metadata = true;
      }
    }
    return result();
  }

  events.push("preflight");
  if (preflightFails || cleanupFiles.some(name => !files.has(name))) return fail();
  for (const name of backedUp) {
    events.push(`backup:${name}`);
    if (backupFailure === name) return fail();
    recovery.set(name, files.get(name));
  }
  if (checkedRetirement) events.push("prepare-registration-receipt");
  for (const removal of removals) {
    const name = removal.path;
    events.push(`delete:${name}`);
    const error = locked.includes(name) || (!selfCopy && name === uninstaller);
    if (!error && !readbackPresent.includes(name)) files.delete(name);
    if (removal.checked && (error || files.has(name) || readbackDenied.includes(name))) {
      return fail();
    }
  }
  if (checkedRetirement) {
    retirementStarted = true;
    events.push("retire-registration");
    for (const step of ["product-delete", "product-readback", "uninstall-delete", "uninstall-readback"]) {
      events.push(step);
      if (retirementFailure === step) return fail();
      if (step === "product-delete") productPath = false;
      if (step === "uninstall-delete") uninstallRegistration = false;
      metadata = productPath && uninstallRegistration;
    }
    metadata = false;
  }
  if (backedUp.length) {
    events.push("move-recovery-to-private-plugin-directory");
    if (recoveryMoveFails) return fail();
    recovery.clear();
  }
  events.push("remove-owned-shortcuts");
  shortcuts = false;
  if (!checkedRetirement) events.push("remove-registration");
  metadata = false;
  return result();
}

function preserved(result) {
  assert.equal(result.exitCode, 1);
  assert.equal(result.metadata, true, "failed uninstall must retain ownership");
  assert.equal(result.shortcuts, true, "failed uninstall must retain shortcuts");
  assert.ok(!result.events.includes("remove-registration"));
  assert.equal(result.files.get("personal.txt"), "user data");
  assert.equal(result.files.get("assets\\custom.ini"), "user settings");
}

test("all bundled payloads use checked deletion; cleanup executables are last", () => {
  assert.deepEqual(removals, [
    { path: resource, checked: true },
    { path: sidecar, checked: true },
    { path: main, checked: true },
    { path: uninstaller, checked: true },
  ]);
});

test("deletion checks both NSIS errors and native absence, not ambiguous file lookup", () => {
  const body = macro("HdrDeleteUninstallPayload");
  ordered(body, [
    'StrCpy $HdrUninstallFailure "${PATH}"',
    "ClearErrors",
    'Delete "${PATH}"',
    "${If} ${Errors}",
    "Goto hdr_uninstall_failed",
    'GetFileAttributesW(w "${PATH}") i .r0 ?e',
    "Pop $1",
    "${If} $0 != -1",
    "Goto hdr_uninstall_failed",
    "${If} $1 != 2",
    "${AndIf} $1 != 3",
    "Goto hdr_uninstall_failed",
  ]);
  assert.doesNotMatch(body, /REBOOTOK|IfFileExists|\$\{FileExists\}/);
});

test("recovery is verified before deletion and detached from automatic failure cleanup", () => {
  assert.deepEqual(backedUp, cleanupFiles);
  ordered(uninstall, [
    "--hdr-installer-uninstall",
    "InitPluginsDir",
    '!insertmacro HdrCreateUninstallDirectory $HdrUninstallRecovery "$TEMP" "hdr-uninstall-recovery" hdr_uninstall_prepare_failed',
    '!insertmacro HdrBackupUninstallFile "${MAINBINARYNAME}.exe"',
    '!insertmacro HdrBackupUninstallFile "uninstall.exe"',
    "--hdr-installer-retirement-prepare nsis",
    '{{#each resources}}',
    '{{#each binaries}}',
    '!insertmacro HdrDeleteUninstallPayload "$INSTDIR\\${MAINBINARYNAME}.exe"',
    '!insertmacro HdrDeleteUninstallPayload "$INSTDIR\\uninstall.exe"',
    "--hdr-installer-retirement-commit nsis",
    "Goto hdr_uninstall_failed",
    'Rename "$HdrUninstallRecovery" "$PLUGINSDIR\\hdr-uninstall-complete"',
    "Goto hdr_uninstall_failed",
    "!insertmacro IsShortcutTarget",
    "Goto hdr_uninstall_done",
    "hdr_uninstall_failed:",
    "hdr_uninstall_done:",
  ]);
  ordered(macro("HdrBackupUninstallFile"), [
    'CopyFileW(w "$INSTDIR\\${NAME}", w "$HdrUninstallRecovery\\${NAME}", i 1) i .r0',
    "${If} $0 == 0",
    "Goto hdr_uninstall_prepare_failed",
  ]);
});

test("registry retirement is checked after payload deletion and before releasing recovery", () => {
  assert.ok(checkedRetirement, "missing checked registry retirement after payload deletion");
  assert.doesNotMatch(uninstall, /DeleteReg(?:Key|Value)/);
  const commit = uninstall.slice(uninstall.indexOf("--hdr-installer-retirement-commit nsis"));
  ordered(commit, [
    "${If} ${Errors}", "StrCpy $0 1", "${If} $0 != 0",
    "Goto hdr_uninstall_failed",
    'Rename "$HdrUninstallRecovery" "$PLUGINSDIR\\hdr-uninstall-complete"',
  ]);
  ordered(uninstall.slice(uninstall.indexOf("hdr_uninstall_failed:")), [
    '!insertmacro HdrRestoreUninstallFile "${MAINBINARYNAME}.exe"',
    '!insertmacro HdrRestoreUninstallFile "uninstall.exe"',
    "--hdr-installer-retirement-restore nsis",
    "SetErrorLevel 1",
    "Quit",
  ]);
  for (const fragment of [
    "fn retire_nsis_registration(", "fn restore_nsis_registration(",
    "fn nsis_retirement(", "RegDeleteKeyExW", "RegEnumValueW",
    "create_new(true)", "sync_all()", "retirement_receipt_path",
  ]) assert.ok(helper.includes(fragment), `missing retirement implementation: ${fragment}`);
});

test("registry retirement compares exact receipt values and checks every native readback", () => {
  const retire = helper.slice(
    helper.indexOf("fn retire_nsis_registration("),
    helper.indexOf("fn restore_nsis_registration("),
  );
  ordered(retire, [
    "registry.read(RetirementKey::Uninstall, entry.view)",
    "registry.read(RetirementKey::Product, entry.view)",
    "registry.delete_product_path(entry.view)?",
    'return Err("Product-path retirement failed readback"',
    "registry.read(RetirementKey::Uninstall, entry.view)",
    "registry.delete_uninstall(entry.view)?",
    'return Err("Uninstall registration retirement failed readback"',
    'return Err("Installation metadata retirement was not confirmed"',
  ]);
  assert.ok(helper.includes("entry.uninstall.get(name) != Some(value)"));
  assert.ok(helper.includes("if !actual.contains_key(name)"));
  assert.ok(helper.includes("original.is_empty() || original != backup"));
  assert.match(helper, /"--hdr-installer-retirement-restore" => \{\s*verify_recovery_pair\(target, recovery\)\?;\s*restore_nsis_registration/);
  assert.match(helper, /write_recovery_artifact\(&recovery\.join\("RECOVERY\.txt"\)/);
  assert.match(uninstall, /Installation metadata recovery was NOT confirmed/);
  assert.match(uninstall, /recovery-only command in RECOVERY\.txt/);
  for (const regression of [
    "nsis_retirement_checks_deletion_and_readback_before_success",
    "nsis_retirement_preserves_unrelated_product_metadata",
    "nsis_retirement_precheck_refuses_concurrent_metadata_without_writes",
    "nsis_retirement_rollback_restores_exact_bytes_after_recovery_cleanup_failure",
    "nsis_retirement_rollback_refuses_unknown_replacements",
    "nsis_retirement_rollback_failure_is_retryable_with_the_same_receipt",
    "nsis_retirement_partial_metadata_restoration_is_retryable_without_overwriting",
    "nsis_retirement_and_rollback_support_aliased_registry_views",
    "nsis_retirement_checks_and_restores_independent_registry_views",
  ]) assert.ok(helper.includes(`fn ${regression}(`), `missing regression: ${regression}`);
});

test("shared registry views permit checked absence after the complete precheck", () => {
  const retire = helper.slice(
    helper.indexOf("fn retire_nsis_registration("),
    helper.indexOf("fn restore_nsis_registration("),
  );
  assert.match(retire, /if let Some\(actual\) = registry\.read\(RetirementKey::Uninstall, entry\.view\)\? \{\s*if actual != entry\.uninstall \{[\s\S]*?registry\.delete_uninstall\(entry\.view\)\?;\s*\}\s*if registry\.read\(RetirementKey::Uninstall, entry\.view\)\?\.is_some\(\)/);
});

test("private directory creation refuses reuse before taking ownership", () => {
  ordered(macro("HdrCreateUninstallDirectory"), [
    "ClearErrors",
    'GetTempFileName ${DIRECTORY} "${PARENT}"',
    "${If} ${Errors}",
    "Goto ${FAILURE}",
    "ClearErrors",
    'Delete "${DIRECTORY}"',
    "${If} ${Errors}",
    "Goto ${FAILURE}",
    'StrCpy ${DIRECTORY} "${DIRECTORY}.${SUFFIX}"',
    'CreateDirectoryW(w "${DIRECTORY}", p 0) i .r0',
    "${If} $0 == 0",
    "Goto ${FAILURE}",
  ]);
  ordered(macro("HdrRestoreUninstallFile"), [
    "${If} $HdrUninstallRestoreDirOwned != 1",
    '!insertmacro HdrCreateUninstallDirectory $HdrUninstallRestoreDir "$INSTDIR" "hdr-uninstall-restore" hdr_uninstall_restore_incomplete',
    "StrCpy $HdrUninstallRestoreDirOwned 1",
    "CopyFileW",
  ]);
  assert.equal(restoreParent, "$INSTDIR");
});

test("failure recovery never overwrites an existing or uninspectable path", () => {
  assert.deepEqual(restored, cleanupFiles);
  ordered(macro("HdrRestoreUninstallFile"), [
    'GetFileAttributesW(w "$INSTDIR\\${NAME}") i .r0 ?e',
    "Pop $1",
    "${If} $0 == -1",
    "${If} $1 == 2",
    "${OrIf} $1 == 3",
    'CopyFileW(w "$HdrUninstallRecovery\\${NAME}", w "$HdrUninstallRestoreDir\\${NAME}.restore", i 1) i .r0',
    "${If} $0 != 0",
    'MoveFileExW(w "$HdrUninstallRestoreDir\\${NAME}.restore", w "$INSTDIR\\${NAME}", i 0) i .r0',
    "${If} $0 == 0",
    "StrCpy $HdrUninstallRestoreFailed 1",
    "${Else}",
    "StrCpy $HdrUninstallRestoreFailed 1",
  ]);
  const failure = uninstall.slice(uninstall.indexOf("hdr_uninstall_failed:"));
  ordered(failure, [
    "!insertmacro HdrRestoreUninstallFile",
    "SetErrorLevel 1",
    "DetailPrint",
    "MessageBox",
    "Quit",
  ]);
  assert.match(failure, /startup may already be disabled/);
  assert.match(failure, /not overwritten/);
  assert.match(failure, /restore the two recovery files/);
  assert.doesNotMatch(failure, /DeleteReg|^\s*Delete |^\s*RMDir "\$INSTDIR"/m);
  ordered(failure, [
    "hdr_uninstall_restore_report:",
    "${If} $HdrUninstallRestoreDirOwned == 1",
    "ClearErrors",
    'RMDir "$HdrUninstallRestoreDir"',
    "${If} ${Errors}",
    'DetailPrint "Nonempty or locked recovery staging retained',
  ]);
});

test("preflight retains its existing requirement for both registered cleanup files", () => {
  assert.ok(helper.includes("let image = canonical(&location.join(MAIN_BINARY))?;"));
  assert.ok(helper.includes('canonical(&location.join("uninstall.exe"))?'));
  ordered(uninstall, [
    "--hdr-installer-uninstall",
    '${If} $0 != 0',
    "SetErrorLevel 1",
    "Quit",
    "!insertmacro HdrBackupUninstallFile",
  ]);
});

for (const name of [main, resource, sidecar, uninstaller]) {
  test(`locked ${name} fails without losing ownership or the cleanup pair`, () => {
    const result = simulate({ locked: [name] });
    preserved(result);
    for (const cleanupFile of cleanupFiles) {
      assert.equal(result.files.get(cleanupFile), result.initial.get(cleanupFile));
      assert.equal(result.recovery.get(cleanupFile), result.initial.get(cleanupFile));
    }
    const failure = result.events.indexOf(`delete:${name}`);
    assert.ok(result.events.slice(failure + 1).every(event => event.startsWith("restore:")));
  });
}

test("missing resources and sidecars are idempotent, not deletion failures", () => {
  const result = simulate({ missing: [resource, sidecar] });
  assert.equal(result.exitCode, 0);
  assert.equal(result.metadata, false);
  assert.equal(result.shortcuts, false);
});

for (const name of cleanupFiles) {
  test(`missing ${name} fails preflight rather than bypassing ownership`, () => {
    const result = simulate({ missing: [name] });
    preserved(result);
    assert.deepEqual(result.events, ["preflight"]);
    assert.deepEqual(result.files, result.initial);
  });
}

test("normal success removes every payload before metadata, but preserves user files", () => {
  const result = simulate();
  assert.equal(result.exitCode, 0);
  assert.equal(result.metadata, false);
  assert.equal(result.shortcuts, false);
  assert.equal(result.recovery.size, 0);
  assert.deepEqual([...result.files.keys()], ["personal.txt", "assets\\custom.ini"]);
  assert.deepEqual(result.events, [
    "preflight",
    `backup:${main}`,
    `backup:${uninstaller}`,
    "prepare-registration-receipt",
    `delete:${resource}`,
    `delete:${sidecar}`,
    `delete:${main}`,
    `delete:${uninstaller}`,
    "retire-registration",
    "product-delete",
    "product-readback",
    "uninstall-delete",
    "uninstall-readback",
    "move-recovery-to-private-plugin-directory",
    "remove-owned-shortcuts",
  ]);
});

for (const failure of ["product-delete", "product-readback", "uninstall-delete", "uninstall-readback"]) {
  test(`post-payload ${failure} failure restores metadata and cleanup executables`, () => {
    const result = simulate({ retirementFailure: failure });
    preserved(result);
    assert.equal(result.productPath, true);
    assert.equal(result.uninstallRegistration, true);
    assert.ok(result.events.includes(`delete:${uninstaller}`));
    assert.ok(result.events.includes("restore-registration"));
    assert.ok(!result.events.includes("move-recovery-to-private-plugin-directory"));
    for (const name of cleanupFiles) {
      assert.equal(result.files.get(name), result.initial.get(name));
      assert.equal(result.recovery.get(name), result.initial.get(name));
    }
  });
}

test("registry rollback refusal retains a usable recovery helper and explicit failure", () => {
  const result = simulate({
    retirementFailure: "uninstall-readback", registryRollbackFails: true,
  });
  assert.equal(result.exitCode, 1);
  assert.equal(result.metadata, false);
  assert.ok(result.events.includes("restore-registration"));
  assert.equal(result.productPath, false);
  assert.equal(result.uninstallRegistration, false);
  for (const name of cleanupFiles) {
    assert.equal(result.files.get(name), result.initial.get(name));
    assert.equal(result.recovery.get(name), result.initial.get(name));
  }
  assert.ok(!result.events.includes("move-recovery-to-private-plugin-directory"));
});

test("post-retirement recovery restores both executable paths before registry ownership", () => {
  const result = simulate({ recoveryMoveFails: true });
  preserved(result);
  const registration = result.events.indexOf("restore-registration");
  for (const name of cleanupFiles) {
    assert.ok(result.events.indexOf(`restore:${name}`) < registration);
  }
});

test("registry recovery refuses an incomplete or unrelated executable replacement", () => {
  for (const failures of [
    { restoreCopyFailure: [main] },
    { replacementOnFailure: main },
    { restoreDirectoryFailure: true },
  ]) {
    const result = simulate({ recoveryMoveFails: true, ...failures });
    assert.equal(result.exitCode, 1);
    assert.equal(result.metadata, false);
    assert.equal(result.shortcuts, true);
    for (const name of cleanupFiles) {
      assert.equal(result.recovery.get(name), result.initial.get(name));
    }
  }
});

for (const name of cleanupFiles) {
  test(`cannot copy ${name} for recovery: no payload is removed`, () => {
    const result = simulate({ backupFailure: name });
    preserved(result);
    assert.deepEqual(result.files, result.initial);
    assert.ok(!result.events.some(event => event.startsWith("delete:")));
  });
}

test("a successful Delete with a remaining payload still fails readback", () => {
  const result = simulate({ readbackPresent: [resource] });
  preserved(result);
  assert.ok(result.files.has(resource));
});

test("an unreadable deletion readback is not mistaken for absence", () => {
  const result = simulate({ readbackDenied: [main] });
  preserved(result);
  assert.equal(result.files.get(main), result.initial.get(main));
});

test("a late uninstaller lock restores the main helper so another attempt can preflight", () => {
  const result = simulate({ locked: [uninstaller] });
  preserved(result);
  assert.ok(result.events.includes(`restore:${main}`));
  assert.ok(cleanupFiles.every(name => result.files.has(name)));
  const retry = simulate({ initialFiles: result.files });
  assert.equal(retry.exitCode, 0);
  assert.equal(retry.metadata, false);
  assert.deepEqual([...retry.files.keys()], ["personal.txt", "assets\\custom.ini"]);
});

test("failed restoration retains durable recovery bytes and ownership, not false success", () => {
  const result = simulate({ locked: [uninstaller], restoreFailure: [main] });
  preserved(result);
  assert.ok(!result.files.has(main));
  assert.equal(result.recovery.get(main), result.initial.get(main));
  assert.equal(result.recovery.get(uninstaller), result.initial.get(uninstaller));
  for (const [name, bytes] of result.recovery) {
    if (!result.files.has(name)) result.files.set(name, bytes);
  }
  const repaired = simulate({ initialFiles: result.files });
  assert.equal(repaired.exitCode, 0);
  assert.equal(repaired.metadata, false);
});

test("failure recovery leaves a concurrent replacement untouched", () => {
  const result = simulate({ locked: [uninstaller], replacementOnFailure: main });
  preserved(result);
  assert.equal(result.files.get(main), "concurrent unrelated replacement");
  assert.equal(result.recovery.get(main), result.initial.get(main));
  assert.ok(!result.events.includes(`restore:${main}`));
});

test("an incomplete restore copy never occupies the registered executable path", () => {
  const result = simulate({ locked: [uninstaller], restoreCopyFailure: [main] });
  preserved(result);
  assert.ok(!result.files.has(main));
  assert.equal(result.restoreStaging.get(main), "incomplete staged copy");
  assert.equal(result.recovery.get(main), result.initial.get(main));
});

test("C: temporary backups restore an E: installation through same-volume staging", () => {
  const result = simulate({
    locked: [uninstaller],
    temporaryVolume: "C:",
    installationVolume: "E:",
  });
  preserved(result);
  assert.equal(result.temporaryVolume, "C:");
  assert.equal(result.restoreVolume, "E:");
  assert.equal(result.installationVolume, "E:");
  for (const name of cleanupFiles) {
    assert.equal(result.files.get(name), result.initial.get(name));
    assert.equal(result.recovery.get(name), result.initial.get(name));
  }
  assert.equal(result.restoreStaging.size, 0);
  const retry = simulate({
    initialFiles: result.files,
    temporaryVolume: "C:",
    installationVolume: "E:",
  });
  assert.equal(retry.exitCode, 0);
  assert.equal(retry.metadata, false);
});

test("refused private staging preserves backups and never takes ownership of another path", () => {
  const result = simulate({ locked: [uninstaller], restoreDirectoryFailure: true });
  preserved(result);
  assert.equal(result.restoreDirectoryOwned, false);
  assert.ok(!result.files.has(main));
  assert.equal(result.restoreStaging.size, 0);
  assert.equal(result.recovery.get(main), result.initial.get(main));
});

test("a replacement appearing after restore staging is not overwritten by rename", () => {
  const result = simulate({ locked: [uninstaller], replacementBeforeRename: main });
  preserved(result);
  assert.equal(result.files.get(main), "concurrent unrelated replacement");
  assert.equal(result.recovery.get(main), result.initial.get(main));
});

test("cannot retire recovery copies: both cleanup files are restored before failure", () => {
  const result = simulate({ recoveryMoveFails: true });
  preserved(result);
  for (const name of cleanupFiles) {
    assert.equal(result.files.get(name), result.initial.get(name));
    assert.equal(result.recovery.get(name), result.initial.get(name));
  }
});

test("_?= in-place execution cannot pretend the original uninstaller was removed", () => {
  const result = simulate({ selfCopy: false });
  preserved(result);
  assert.ok(cleanupFiles.every(name => result.files.has(name)));
});

test("preflight refusal removes no payload, shortcuts, or installation metadata", () => {
  const result = simulate({ preflightFails: true });
  preserved(result);
  assert.deepEqual(result.events, ["preflight"]);
  assert.deepEqual(result.files, result.initial);
});

test("no reboot deletion, recursive removal, wildcard payload, or generic Run deletion", () => {
  assert.doesNotMatch(uninstall, /\/REBOOTOK|RMDir\s+\/r\b|Delete\s+"[^"]*[*?]/i);
  assert.doesNotMatch(uninstall, /DeleteReg(?:Key|Value)[^\r\n]*(?:StartupApproved|CurrentVersion\\Run)/);
  for (const { path } of removals) assert.doesNotMatch(path, /[*?]/);
  assert.match(uninstall, /RMDir "\$INSTDIR"/);
  assert.match(uninstall, /IsShortcutTarget "\$SMPROGRAMS/);
  assert.match(uninstall, /IsShortcutTarget "\$DESKTOP/);
});

test("the contract records partial-removal recovery and the live qualification gate", () => {
  assert.match(contract.integration.nsisUninstallRemoval ?? "", /recovery/i);
  assert.match(contract.integration.nsisUninstallRemoval ?? "", /metadata/i);
  assert.match(contract.integration.nsisUninstallRemoval ?? "", /missing/i);
  assert.match(contract.integration.nsisUninstallRemoval ?? "", /installation volume/i);
  assert.match(contract.integration.nsisRegistryRetirement ?? "", /receipt/i);
  assert.match(contract.integration.nsisRegistryRetirement ?? "", /readback/i);
  assert.match(contract.integration.nsisRegistryRetirement ?? "", /rollback/i);
  assert.ok(contract.releaseValidationGates.some(gate =>
    /NSIS uninstall/.test(gate) && /payload/.test(gate) && /recovery/.test(gate)));
});
