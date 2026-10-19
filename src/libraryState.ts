import type { HdrApp } from './types.ts';

type LibraryIdentity = Pick<HdrApp, 'exe_name'>
  & Partial<Pick<HdrApp, 'name' | 'steam_id' | 'alternate_exes' | 'path'>>;

function normalizePath(path?: string | null): string | null {
  if (!path || !path.trim()) return null;
  return path
    .replace(/\//g, '\\')
    .toLowerCase()
    .trim()
    .replace(/^\\\\\?\\unc\\/, '\\\\')
    .replace(/^\\\\\?\\/, '');
}

function conflictingIds(existing: HdrApp, item: LibraryIdentity): boolean {
  return existing.steam_id != null && item.steam_id != null && existing.steam_id !== item.steam_id;
}

function sameSteamId(existing: HdrApp, item: LibraryIdentity): boolean {
  return existing.steam_id != null && item.steam_id != null
    && existing.steam_id.trim() !== '' && existing.steam_id === item.steam_id;
}

function sameName(existing: HdrApp, item: LibraryIdentity): boolean {
  return item.name != null && existing.name.trim().toLowerCase() === item.name.trim().toLowerCase();
}

function differentGamesSharingExecutable(existing: HdrApp, item: LibraryIdentity): boolean {
  const existingPath = normalizePath(existing.path);
  const itemPath = normalizePath(item.path);
  if (existingPath != null && itemPath != null && existingPath === itemPath) {
    return false;
  }
  if (conflictingIds(existing, item)) {
    return true;
  }
  if (itemPath != null && item.name != null && !sameSteamId(existing, item) && !sameName(existing, item)) {
    return true;
  }
  return false;
}

function executableOverlap(existing: HdrApp, item: LibraryIdentity): boolean {
  const incoming = new Set([item.exe_name, ...(item.alternate_exes ?? [])].map((exe) => exe.toLowerCase()));
  return [existing.exe_name, ...(existing.alternate_exes ?? [])]
    .some((exe) => incoming.has(exe.toLowerCase()));
}

export function findLibraryApp(apps: HdrApp[], item: LibraryIdentity): HdrApp | undefined {
  return apps.find((existing) => {
    if (differentGamesSharingExecutable(existing, item)) return false;
    if (existing.exe_name.toLowerCase() === item.exe_name.toLowerCase()) return true;
    if (conflictingIds(existing, item)) return false;
    return (item.name != null && existing.name.toLowerCase() === item.name.toLowerCase())
      || executableOverlap(existing, item);
  });
}

export function findTrackedApp(apps: HdrApp[], item: LibraryIdentity): HdrApp | undefined {
  return apps.find((existing) => existing.enabled
    && !differentGamesSharingExecutable(existing, item)
    && (existing.exe_name.toLowerCase() === item.exe_name.toLowerCase()
      || (!conflictingIds(existing, item) && executableOverlap(existing, item))));
}
