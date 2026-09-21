import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

// Optional read-only baseline mode: git blobs stay in memory; no checkout, cache, or network I/O.
const sourceAt = (path) => process.env.CATALOG_TEST_REF
  ? execFileSync('git', ['show', `${process.env.CATALOG_TEST_REF}:${path}`], {
    cwd: fileURLToPath(new URL('..', import.meta.url)), encoding: 'utf8',
  })
  : readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const embeddedText = sourceAt('src-tauri/catalog.json');
const publishedText = sourceAt('database/hdr_games.json');
const databaseSource = sourceAt('src-tauri/src/database.rs').replace(/\r\n/g, '\n');
const catalog = JSON.parse(embeddedText);
const section = (start, end) => {
  const from = databaseSource.indexOf(start);
  assert.notEqual(from, -1, `Missing catalog boundary: ${start}`);
  const to = databaseSource.indexOf(end, from + start.length);
  assert.ok(to > from, `Missing end of catalog boundary: ${end}`);
  return databaseSource.slice(from, to);
};
const inOrder = (source, ...tokens) => {
  let previous = -1;
  for (const token of tokens) {
    const at = source.indexOf(token, previous + 1);
    assert.ok(at > previous, `Missing or reordered catalog operation: ${token}`);
    previous = at;
  }
};
const key = (name) => name.toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');
const executable = (name) => {
  assert.equal(typeof name, 'string');
  assert.equal(name, name.trim());
  assert.match(name, /^[^\\/:]+\.exe$/i);
  assert.ok(name.length > 4);
  return name.toLowerCase();
};

// These are equivalent names of the same product, not guesses based on a shared basename.
// BG3's authored notes explicitly cover Vulkan and DX11; the dated remake rows carry the
// product IDs and remake-specific HDR references. Keep their original authority intact.
const canonicalProducts = [
  ["Baldur's Gate 3", "Baldur's Gate 3 (DX11)", '1086940', 'bg3.exe', 'bg3_dx11.exe'],
  ['Dead Space (2023)', 'Dead Space Remake', '1693980', 'deadspace.exe', 'deadspace.exe'],
  ['Resident Evil 2 (2019)', 'Resident Evil 2 Remake', '883710', 're2.exe', 're2.exe'],
  ['Resident Evil 3 (2020)', 'Resident Evil 3 Remake', '952060', 're3.exe', 're3.exe'],
  ['Resident Evil 4 (2023)', 'Resident Evil 4 Remake', '2050650', 're4.exe', 're4.exe'],
];

// Shared engines/basenames and collection/component overlap are not equivalent identities.
// Their declarations must continue to fail closed without unambiguous provider evidence.
const reviewedOverlaps = {
  'cod.exe': ['Call of Duty: Modern Warfare II (2022)', 'Call of Duty: Warzone'],
  'hitman.exe': ['Hitman', 'Hitman World of Assassination'],
  'masseffect1.exe': ['Mass Effect 1 (LE)', 'Mass Effect Legendary Edition'],
  'masseffect2.exe': ['Mass Effect 2 (LE)', 'Mass Effect Legendary Edition'],
  'masseffect3.exe': ['Mass Effect 3 (LE)', 'Mass Effect Legendary Edition'],
  'tll.exe': ['Uncharted: Legacy of Thieves Collection', 'Uncharted: The Lost Legacy'],
};

test('embedded and published full catalogs are synchronized', () => {
  assert.equal(embeddedText, publishedText);
  assert.ok(catalog.length > 1000, 'validate the full catalog, not a small fixture');
});

test('all catalog identities, executable lists, and provider products are valid and unique', () => {
  const identities = new Map();
  const products = new Map();
  const tiers = new Set(['native', 'limited', 'always_on', 'manual_fix', 'autohdr', 'media']);
  for (const entry of catalog) {
    assert.ok(tiers.has(entry.support_tier), entry.name);
    assert.ok(['native', 'custom', 'autohdr', 'media'].includes(entry.hdr_type), entry.name);
    assert.ok(entry.notes === null || typeof entry.notes === 'string', entry.name);
    for (const name of [entry.name, ...(entry.name_aliases ?? [])]) {
      assert.ok(key(name), entry.name);
      assert.ok(!identities.has(key(name)), `${name} overlaps ${identities.get(key(name))}`);
      identities.set(key(name), entry.name);
    }
    const exes = [entry.exe_name, ...entry.alternate_exes].map(executable);
    assert.equal(new Set(exes).size, exes.length, entry.name);
    assert.ok(!('storefront_authority' in entry), entry.name);
    assert.ok(!('executable_authority' in entry), entry.name);
    const bindings = [...(entry.storefronts ?? [])];
    if (entry.steam_id !== null) {
      assert.match(entry.steam_id, /^[1-9]\d*$/, entry.name);
      if (!bindings.some(({ provider }) => provider === 'steam')) {
        bindings.push({ provider: 'steam', product_id: entry.steam_id, game_executables: [entry.exe_name] });
      }
    }
    for (const binding of bindings) {
      assert.ok(['steam', 'xbox'].includes(binding.provider), entry.name);
      const games = binding.game_executables.map(executable);
      const excluded = (binding.excluded_executables ?? []).map(executable);
      assert.equal(new Set([...games, ...excluded]).size, games.length + excluded.length, entry.name);
      if (binding.product_id !== undefined) {
        assert.ok(binding.product_id.trim(), entry.name);
        const identity = `${binding.provider}:${binding.product_id}`;
        assert.ok(!products.has(identity), `${identity} overlaps ${products.get(identity)}`);
        products.set(identity, entry.name);
      }
    }
  }
});

test('only explicitly proven name pairs are canonicalized, preserving authored executables and IDs', () => {
  assert.equal(catalog.filter(({ name_aliases }) => name_aliases?.length).length, canonicalProducts.length);
  for (const [name, alias, id, primary, executableAlias] of canonicalProducts) {
    const entries = catalog.filter((entry) => entry.name === name);
    assert.equal(entries.length, 1, name);
    const [entry] = entries;
    assert.deepEqual(entry.name_aliases, [alias]);
    assert.equal(entry.steam_id, id);
    assert.equal(entry.exe_name, primary);
    assert.ok([primary, ...entry.alternate_exes].includes(executableAlias));
    assert.ok(!catalog.some((candidate) => candidate.name === alias), alias);
  }
});

test('full executable overlap audit preserves true ambiguity and catches new unaudited collisions', () => {
  const nominations = new Map();
  for (const entry of catalog) {
    for (const exe of [entry.exe_name, ...entry.alternate_exes].map(executable)) {
      nominations.set(exe, [...(nominations.get(exe) ?? []), entry.name]);
    }
  }
  const overlaps = Object.fromEntries(
    [...nominations].filter(([, names]) => names.length > 1).map(([exe, names]) => [exe, names.sort()]),
  );
  assert.deepEqual(overlaps, reviewedOverlaps);
  assert.notEqual(
    catalog.find(({ name }) => name === reviewedOverlaps['cod.exe'][0]).steam_id,
    catalog.find(({ name }) => name === reviewedOverlaps['cod.exe'][1]).steam_id,
  );
});

// Source-level regressions supplement, rather than replace, the isolated Rust behavioral tests.
test('cache publication and disk reload share the same display-only merge policy', () => {
  const metadata = section('struct DisplayMetadata {', 'struct DisplayOverlay {');
  assert.match(metadata, /support_tier: String/);
  assert.match(metadata, /notes: Option<String>/);
  assert.doesNotMatch(metadata, /exe_name|steam_id|hdr_type|storefront|name_aliases/);
  const merge = section('fn merge_catalog(', 'fn restrict_storefront_authority(');
  assert.match(merge, /embedded\[index\]\.support_tier = metadata\.support_tier/);
  assert.match(merge, /embedded\[index\]\.notes = metadata\.notes/);
  assert.doesNotMatch(merge, /embedded\[index\]\.(?:exe_name|alternate_exes|steam_id|hdr_type|storefronts|name_aliases)\s*=/);
  assert.match(section('fn with_embedded_authority(', 'pub fn save_to_cache('),
    /merge_catalog\(embedded_catalog\(\), entries\)/);
  assert.match(section('    fn snapshot_with(', '    fn publish('),
    /merge_catalog\(embedded_catalog\(\), load\(path\)\)/);
  assert.match(section('    fn publish_with(', '\nfn get_cache_path('),
    /with_embedded_authority\(entries\.to_vec\(\)\)/);
});

test('cache publications serialize disk and memory and fence concurrent or stale synchronization', () => {
  const publication = section('    fn publish_with(', '\nfn get_cache_path(');
  inOrder(publication, 'self.state.write()', 'expected_revision.is_some_and',
    'write(path, &json)?', 'state.entries = Some(entries.clone())', 'state.revision = revision');
  const coldLoad = section('    fn snapshot_with(', '    fn publish(');
  inOrder(coldLoad, 'self.state.read()', 'self.state.write()', 'if state.entries.is_none()', 'load(path)');
  assert.match(section('    fn begin_sync(', '    fn snapshot('), /compare_exchange\(false, true/);
  assert.match(section("impl Drop for CatalogSync<'_>", 'impl CatalogCache {'), /store\(false, Ordering::Release\)/);
  const fetch = section('pub async fn fetch_online_database(', 'fn valid_catalog_entry(');
  inOrder(fetch, 'begin_sync()?', 'snapshot(&cache_path)?', 'reqwest::Client::builder()',
    'publish(&cache_path, &result, Some(revision))');
});

test('cache writes flush an exclusive sibling stage before atomic replacement', () => {
  const writer = section('fn write_cache_atomically_with(', '#[cfg(windows)]');
  inOrder(writer, 'path.with_file_name(', 'create_new(true)', 'file.write_all(json)',
    'file.sync_all()', 'drop(file)', 'before_replace(&stage.0)?', 'replace_cache_file(&stage.0, path)');
  assert.doesNotMatch(writer, /fs::write\((?:path|&?cache_file)/);
  const replace = section('fn replace_cache_file(stage:', '#[cfg(not(windows))]');
  assert.match(replace, /MoveFileExW\(/);
  assert.match(replace, /MOVEFILE_REPLACE_EXISTING \| MOVEFILE_WRITE_THROUGH/);
  assert.doesNotMatch(replace, /MOVEFILE_COPY_ALLOWED|remove_file/);
  assert.match(section('impl Drop for CacheStage {', 'fn write_cache_atomically('), /fs::remove_file\(&self\.0\)/);
});

test('default Rust test access cannot reach a user cache or online source', () => {
  const path = section('fn get_cache_path(', 'pub fn get_full_catalog(');
  assert.match(path, /#\[cfg\(test\)\]\s*\{\s*Err\("Catalog I\/O requires an explicitly injected path/);
  assert.match(path, /#\[cfg\(not\(test\)\)\][\s\S]*std::env::var_os\("APPDATA"\)/);
  assert.match(path, /!app_data\.is_absolute\(\)/);
  assert.doesNotMatch(path, /unwrap_or_else.*"\."/);
  const catalog = section('pub fn get_full_catalog(', 'fn read_cached_catalog(');
  assert.match(catalog, /#\[cfg\(test\)\]\s*\{\s*merge_catalog\(embedded_catalog\(\), Vec::new\(\)\)\s*\}/);
  assert.match(section('pub fn save_to_cache(', 'struct CacheStage('), /get_cache_path\(\)\?/);
  inOrder(section('pub async fn fetch_online_database(', 'fn valid_catalog_entry('),
    'get_cache_path()?', 'begin_sync()?', 'snapshot(&cache_path)?', 'reqwest::Client::builder()');
  assert.match(databaseSource, /fn default_test_catalog_access_never_uses_appdata_or_the_network\(\)/);
});
