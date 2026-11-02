import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import type {
  Album,
  Folder,
  PlaybackPosition,
  PlayerState,
  RepeatMode,
  ScanProgress,
  ScanSummary,
  Track,
} from '../types';

/** True when running inside the Tauri webview (false in a plain browser, e.g. `vite dev`). */
export const isTauri = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const api = {
  // Library
  listTracks: (limit = 200, offset = 0) => invoke<Track[]>('library_list_tracks', { limit, offset }),
  countTracks: () => invoke<number>('library_count_tracks'),
  searchTracks: (query: string, limit = 200) => invoke<Track[]>('library_search_tracks', { query, limit }),
  listAlbums: (limit = 200, offset = 0) => invoke<Album[]>('library_list_albums', { limit, offset }),
  listFolders: () => invoke<Folder[]>('library_list_folders'),
  /** Opens the native folder picker; resolves to null if the user cancels. */
  addFolder: () => invoke<Folder | null>('library_add_folder'),
  scanFolder: (folderId: number) => invoke<ScanSummary>('library_scan_folder', { folderId }),
  cancelScan: () => invoke<boolean>('library_cancel_scan'),

  // Player (tracks are always referred to by library id, never by path)
  playTracks: (trackIds: number[], startTrackId: number | null) =>
    invoke<void>('player_play_tracks', { trackIds, startTrackId }),
  toggle: () => invoke<void>('player_toggle'),
  next: () => invoke<void>('player_next'),
  previous: () => invoke<void>('player_previous'),
  seek: (positionMs: number) => invoke<void>('player_seek', { positionMs }),
  setVolume: (volume: number) => invoke<void>('player_set_volume', { volume }),
  setShuffle: (enabled: boolean) => invoke<void>('player_set_shuffle', { enabled }),
  setRepeat: (mode: RepeatMode) => invoke<void>('player_set_repeat', { mode }),
  getPlayerState: () => invoke<PlayerState>('player_get_state'),
};

export interface BackendListeners {
  onState: (s: PlayerState) => void;
  onPosition: (p: PlaybackPosition) => void;
  onError: (message: string) => void;
  onScanProgress: (p: ScanProgress) => void;
  onScanFinished: (s: ScanSummary) => void;
}

/** Subscribes to backend events. Returns a function that removes every subscription. */
export async function subscribe(l: BackendListeners): Promise<() => void> {
  const offs: UnlistenFn[] = await Promise.all([
    listen<PlayerState>('player://state', (e) => l.onState(e.payload)),
    listen<PlaybackPosition>('player://position', (e) => l.onPosition(e.payload)),
    listen<string>('player://error', (e) => l.onError(e.payload)),
    listen<ScanProgress>('library://scan-progress', (e) => l.onScanProgress(e.payload)),
    listen<ScanSummary>('library://scan-finished', (e) => l.onScanFinished(e.payload)),
  ]);
  return () => offs.forEach((off) => off());
}
