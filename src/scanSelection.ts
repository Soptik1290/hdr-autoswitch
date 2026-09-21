import type { HdrApp, ScanGame } from './types.ts';
import type { ConfigClient, MutationOrigin } from './configState.ts';

export type ScanSelection = Record<string, boolean>;

export function detectionKey(game: ScanGame): string {
  const path = game.path?.replace(/\//g, '\\').toLowerCase()
    .replace(/^\\\\\?\\unc\\/, '\\\\').replace(/^\\\\\?\\/, '') ?? null;
  return JSON.stringify([
    game.launcher?.toLowerCase() ?? null, path, game.exe_name.toLowerCase(), game.steam_id ?? null,
  ]);
}

export function selectDetections(
  games: ScanGame[], selected: (game: ScanGame) => boolean = (game) => game.default_selected,
): ScanSelection {
  return Object.fromEntries(games.map((game) => [detectionKey(game), selected(game)]));
}

export function toggleDetection(selection: ScanSelection, game: ScanGame): ScanSelection {
  const key = detectionKey(game);
  return { ...selection, [key]: !selection[key] };
}

export function fromScanGame(game: ScanGame): HdrApp {
  return {
    name: game.name,
    exe_name: game.exe_name,
    hdr_type: game.hdr_type,
    path: game.path,
    alternate_exes: game.alternate_exes ? [...game.alternate_exes] : undefined,
    steam_id: game.steam_id,
    launcher: game.launcher,
    enabled: true,
  };
}

export function selectedDetections(games: ScanGame[], selection: ScanSelection): HdrApp[] {
  return games.filter((game) => selection[detectionKey(game)])
    .map(fromScanGame);
}

export async function importSelectedDetections(
  client: Pick<ConfigClient, 'mutate'>, games: ScanGame[], selection: ScanSelection,
  origin: MutationOrigin,
): Promise<number> {
  const detected = selectedDetections(games, selection);
  if (detected.length > 0) {
    await client.mutate('import_detected_games', { detected }, origin);
  }
  return detected.length;
}
