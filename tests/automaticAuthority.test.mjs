import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = (name) => readFileSync(new URL(`../src-tauri/src/${name}.rs`, import.meta.url), 'utf8');

test('all automatic discovery paths use shared authority, not catalog/title/stem heuristics', () => {
  const scanner = source('scanner');
  const automaticScanner = scanner.slice(scanner.indexOf('fn match_and_insert_game('), scanner.indexOf('// String & Title Helpers'));
  assert.match(automaticScanner, /automatic_authority::resolve/);
  assert.doesNotMatch(automaticScanner, /find_in_catalog|is_title_match|pick_best|collect_exes/);
  assert.doesNotMatch(scanner, /fn pick_best_primary_exe|fn collect_exes/);
  const monitor = source('monitor_hook');
  const enrollment = monitor.slice(monitor.indexOf('    fn enroll('), monitor.indexOf('    fn ensure_watcher('));
  assert.match(enrollment, /automatic_authority::resolve\(&crate::database::get_full_catalog\(\), None\)/);
  assert.doesNotMatch(enrollment, /find_in_catalog|is_title_match/);
});

test('startup enrichment and scanner defaults cannot bypass provider authority or auto-detect', () => {
  const background = source('background');
  assert.match(background, /enrich_verified_aliases\(settings, &detected\.verified\)/);
  assert.match(background, /enrich_verified_metadata/);
  assert.match(background, /scan_installed_games_with_authority\(auto_detect\)/);
  assert.match(background, /enrich_verified_metadata\(settings, &detected\.verified\)/);
  assert.doesNotMatch(background, /library::enrich_existing/);
  assert.match(background, /mutate_if_changed/);
  assert.match(source('commands'), /scan_installed_games\(auto_detect\)/);
  assert.match(source('scanner'), /game.default_selected &= auto_detect/);
  assert.doesNotMatch(source('scanner'), /game\.is_hdr_supported\s*[&|]?=\s*auto_detect/);
  const library = source('library');
  const enrichment = library.slice(library.indexOf('fn verified_association('), library.indexOf('pub fn validate_app('));
  assert.doesNotMatch(enrichment, /merge_aliases|same_game|\.enabled\s*=|\.path\s*=|\.exe_name\s*=/);
  assert.match(enrichment, /existing.exe_name.eq_ignore_ascii_case/);
  assert.match(enrichment, /detected: &\[ResolvedGame\]/);
  assert.doesNotMatch(enrichment, /item.enabled/);
  assert.match(enrichment, /resolved\.executables/);
  assert.match(enrichment, /is_quarantined\(existing\)/);
  assert.match(enrichment, /candidate_counts/);
});

test('verified alias enrichment is separate from exact-primary missing metadata updates', () => {
  const library = source('library');
  const aliases = library.slice(library.indexOf('pub fn enrich_verified_aliases('), library.indexOf('pub fn enrich_verified_metadata('));
  assert.match(aliases, /detected: &\[ResolvedGame\]/);
  assert.match(aliases, /unique_verified_associations/);
  assert.match(aliases, /existing\.alternate_exes\.push/);
  assert.doesNotMatch(aliases, /\.(?:enabled|path|exe_name|name|hdr_type|steam_id|launcher)\s*=|clone_from/);
  const metadata = library.slice(library.indexOf('pub fn enrich_verified_metadata('), library.indexOf('fn validate_executable('));
  assert.match(metadata, /existing\.exe_name\.eq_ignore_ascii_case\(&resolved\.executables\[0\]\.basename\)/);
  assert.match(metadata, /existing\.steam_id\.is_none\(\)/);
  assert.match(metadata, /existing\.launcher\.is_none\(\)/);
  assert.doesNotMatch(metadata, /alternate_exes|merge_aliases/);
});

test('shared permanent runtime denial protects provider nominations and explicit additions', () => {
  const authority = source('automatic_authority');
  const implementation = authority.slice(0, authority.indexOf('#[cfg(test)]'));
  assert.match(implementation, /runtime_policy::permanently_excluded/);
  assert.match(implementation, /fn nominated[\s\S]*?permanently_excluded/);
  assert.match(implementation, /fn binding_matches[\s\S]*?!permanently_excluded/);
  assert.match(implementation, /fn resolve_declared[\s\S]*?permanently_excluded/);
  assert.doesNotMatch(implementation, /gamelaunchhelper\.exe/);
  const scanner = source('scanner');
  const inspect = scanner.slice(scanner.indexOf('pub fn inspect_exe_path('), scanner.indexOf('fn scan_steam_manifests('));
  assert.match(inspect, /runtime_policy::permanently_excluded\(&exe_name\)/);
  const library = source('library');
  const validation = library.slice(library.indexOf('pub fn validate_app('), library.indexOf('pub fn add_app('));
  assert.match(validation, /once\(&app.exe_name\)\.chain\(&app.alternate_exes\)/);
  assert.match(validation, /validate_executable\(exe\)/);
  const executableValidation = library.slice(library.indexOf('fn validate_executable('), library.indexOf('pub fn validate_app('));
  assert.match(executableValidation, /permanently_excluded\(exe\)/);
});

test('scan observations retain resolved authority separately from manual-only suggestions', () => {
  const scanner = source('scanner');
  assert.match(scanner, /pub verified: Vec<ResolvedGame>/);
  const matcher = scanner.slice(scanner.indexOf('fn match_and_insert_game('), scanner.indexOf('fn parsed_launch('));
  assert.match(matcher, /Authority::Resolved\(resolved\)[\s\S]*?map\.verified\.push\(resolved\)/);
  const suggestions = matcher.slice(matcher.indexOf('Authority::Unresolved if'));
  assert.doesNotMatch(suggestions, /verified\.push/);
  assert.match(suggestions, /is_hdr_supported: false/);
  assert.match(suggestions, /default_selected: false/);
});

test('explicit repair replaces quarantined bindings and scopes paths to the retained primary', () => {
  const library = source('library');
  const repair = library.slice(library.indexOf('fn apply_explicit_executables('), library.indexOf('pub fn import_games('));
  assert.match(repair, /is_quarantined\(existing\)/);
  assert.match(repair, /existing\.exe_name = item\.exe_name/);
  assert.match(repair, /existing\.alternate_exes\.clear\(\)/);
  assert.match(repair, /existing\.exe_name\.eq_ignore_ascii_case\(&item\.exe_name\) && item\.path\.is_some\(\)/);
  const veto = library.slice(library.indexOf('pub fn automatic_enrollment_veto('), library.indexOf('fn same_game('));
  assert.match(veto, /app\.steam_id, &candidate\.steam_id/);
  assert.match(veto, /claims_overlap\(app, candidate\)/);
  assert.match(source('runtime_policy'), /left\.path\.as_deref\(\)\.and_then\(normalize_windows_path\)/);
  assert.match(source('runtime_policy'), /right\.path\.as_deref\(\)\.and_then\(normalize_windows_path\)/);
  assert.doesNotMatch(veto, /same_game|merge_aliases|\.push\(/);
});

test('targeted quarantine repair uses exact unique identity without changing user metadata', () => {
  const library = source('library');
  const start = library.indexOf('pub fn repair_executable(');
  const repair = library.slice(start, library.indexOf('#[cfg(test)]', start));
  assert.match(repair, /row_index\(config, row\)\?/);
  assert.match(repair, /reject_other_claims\(config, Some\(index\), &existing\)\?/);
  assert.match(library, /app.exe_name != row.exe_name \|\| app.path != row.path/);
  assert.match(repair, /is_quarantined\(&config\.apps\[index\]\)/);
  assert.match(repair, /validate_executable\(&selected_exe\)/);
  assert.match(repair, /existing\.alternate_exes\.clear\(\)/);
  assert.doesNotMatch(repair, /same_game|\.enabled\s*=|\.name\s*=|\.hdr_type\s*=|\.steam_id\s*=|\.launcher\s*=/);
});
