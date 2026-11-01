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
  year?: number;
}

export type PlaybackStatus = 'Stopped' | 'Loading' | 'Playing' | 'Paused' | 'Error';
export type RepeatMode = 'Off' | 'All' | 'One';

export interface PlaybackPosition {
  position_ms: number;
  duration_ms?: number;
  playing: boolean;
}

export interface PlayerState {
  status: PlaybackStatus;
  current_track: Track | null;
  position: PlaybackPosition;
  volume: number;
  shuffle: boolean;
  repeat: RepeatMode;
}
