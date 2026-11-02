export interface Track {
  id: number;
  title: string;
  artist: string;
  album: string;
  duration_ms: number;
  rel_path: string;
  folder_location: string;
}

export interface Album {
  id: number;
  title: string;
  artist: string;
  track_count: number;
  year?: number | null;
}

export interface Folder {
  id: number;
  location: string;
  display_name: string;
  availability: 'available' | 'unavailable' | 'unknown';
  last_scan_at: number | null;
  track_count: number;
}

export interface ScanSummary {
  scan_id: number;
  files_seen: number;
  added: number;
  updated: number;
  moved: number;
  missing: number;
  duplicates: number;
  skipped: number;
  unsupported: number;
  errors: number;
  purged: number;
  cancelled: boolean;
}

export interface ScanProgress {
  phase: 'Walking' | 'Indexing' | 'Finishing';
  files_total: number;
  files_done: number;
}

export type PlaybackStatus = 'Stopped' | 'Loading' | 'Playing' | 'Paused' | 'Error';
export type RepeatMode = 'Off' | 'All' | 'One';

export interface PlaybackPosition {
  position_ms: number;
  duration_ms: number | null;
  playing: boolean;
}

/** Mirrors the Rust `QueueItem`. `track_id` is the library track id. */
export interface QueueItem {
  id: number;
  track_id: number;
  title: string;
  artist: string;
}

/** Mirrors the Rust `PlayerState` emitted on `player://state`. */
export interface PlayerState {
  status: PlaybackStatus;
  queue_revision: number;
  current_queue_index: number | null;
  current_item: QueueItem | null;
  position: PlaybackPosition;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
  error: string | null;
}
