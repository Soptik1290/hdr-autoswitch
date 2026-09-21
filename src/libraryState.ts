import type { AppRowIdentity, HdrApp } from './types.ts';
import type { ConfigClient, MutationOrigin } from './configState.ts';

type LibraryIdentity = Pick<HdrApp, 'exe_name'>
  & Partial<Pick<HdrApp, 'name' | 'steam_id' | 'alternate_exes' | 'path'>>;

export function normalizeLibraryPath(path: string | null | undefined): string | null {
  if (!path) return null;
  const normalized = path.replace(/\//g, '\\').toLowerCase()
    .replace(/^\\\\\?\\unc\\/, '\\\\').replace(/^\\\\\?\\/, '');
  const drive = /^[a-z]:\\/.test(normalized);
  if (!drive && !normalized.startsWith('\\\\')) return null;
  const prefix = drive ? normalized.slice(0, 3) : '\\\\';
  const minimum = drive ? 0 : 2;
  const parts: string[] = [];
  for (const part of normalized.slice(prefix.length).split('\\')) {
    if (!part || part === '.') continue;
    if (part === '..') {
      if (parts.length <= minimum) return null;
      parts.pop();
    } else {
      if (/[<>:"|?*\u0000-\u001f]/.test(part)) return null;
      parts.push(part);
    }
  }
  return parts.length > minimum ? prefix + parts.join('\\') : null;
}

export function primaryExecutable(value: string): string {
  const exe = value.trim().toLowerCase();
  return exe && !exe.endsWith('.exe') ? `${exe}.exe` : exe;
}

export function pathAfterExecutableEdit(path: string, value: string): string {
  return normalizeLibraryPath(path)?.split('\\').pop() === primaryExecutable(value) ? path : '';
}

export function libraryRowIdentity(apps: HdrApp[], app: HdrApp): AppRowIdentity {
  const index = apps.indexOf(app);
  if (index < 0) throw new Error('This library row changed. Refresh before editing it.');
  return { index, exe_name: app.exe_name, path: app.path ?? null };
}

export function captureLibraryRow(
  client: Pick<ConfigClient, 'captureOrigin' | 'getView'>, apps: HdrApp[], app: HdrApp,
): { row: AppRowIdentity; origin: MutationOrigin } {
  const origin = client.captureOrigin();
  if (client.getView().snapshot?.settings.apps !== apps) {
    throw new Error('This library view changed. Wait for the current library before editing it.');
  }
  return { row: libraryRowIdentity(apps, app), origin };
}

function uniqueMatch(candidates: HdrApp[], item: LibraryIdentity): HdrApp | undefined {
  const path = normalizeLibraryPath(item.path);
  const exact = path == null ? [] : candidates.filter((app) =>
    app.exe_name.toLowerCase() === item.exe_name.toLowerCase() && normalizeLibraryPath(app.path) === path);
  const matches = exact.length ? exact : candidates;
  return matches.length === 1 ? matches[0] : undefined;
}

function conflictingIds(existing: HdrApp, item: LibraryIdentity): boolean {
  return existing.steam_id != null && item.steam_id != null && existing.steam_id !== item.steam_id;
}

function executableOverlap(existing: HdrApp, item: LibraryIdentity): boolean {
  const incoming = new Set([item.exe_name, ...(item.alternate_exes ?? [])].map((exe) => exe.toLowerCase()));
  return [existing.exe_name, ...(existing.alternate_exes ?? [])]
    .some((exe) => incoming.has(exe.toLowerCase()));
}

export function findLibraryApp(apps: HdrApp[], item: LibraryIdentity): HdrApp | undefined {
  return uniqueMatch(apps.filter((existing) => {
    if (existing.exe_name.toLowerCase() === item.exe_name.toLowerCase()) return true;
    if (conflictingIds(existing, item)) return false;
    return (item.name != null && existing.name.toLowerCase() === item.name.toLowerCase())
      || executableOverlap(existing, item);
  }), item);
}

export function findTrackedApp(apps: HdrApp[], item: LibraryIdentity): HdrApp | undefined {
  const found = uniqueMatch(apps.filter((existing) =>
    existing.exe_name.toLowerCase() === item.exe_name.toLowerCase()
      || (!conflictingIds(existing, item) && executableOverlap(existing, item))), item);
  return found?.enabled ? found : undefined;
}
