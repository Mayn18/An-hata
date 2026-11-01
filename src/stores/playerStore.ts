import { create } from 'zustand';
import { Track, PlayerState, RepeatMode } from '../types';
import { api } from '../services/api';

interface PlayerStore extends PlayerState {
  tracks: Track[];
  searchQuery: string;
  isLoading: boolean;
  activeView: 'library' | 'playlists' | 'albums' | 'settings';

  // Actions
  fetchTracks: () => Promise<void>;
  setSearchQuery: (query: string) => Promise<void>;
  setActiveView: (view: 'library' | 'playlists' | 'albums' | 'settings') => void;
  playTrack: (track: Track) => void;
  togglePlay: () => void;
  nextTrack: () => void;
  prevTrack: () => void;
  seek: (ms: number) => void;
  setVolume: (vol: number) => void;
  toggleShuffle: () => void;
  toggleRepeat: () => void;
}

export const usePlayerStore = create<PlayerStore>((set, get) => ({
  status: 'Stopped',
  current_track: null,
  position: { position_ms: 0, duration_ms: 0, playing: false },
  volume: 0.8,
  shuffle: false,
  repeat: 'Off',
  tracks: [],
  searchQuery: '',
  isLoading: false,
  activeView: 'library',

  fetchTracks: async () => {
    set({ isLoading: true });
    try {
      const tracks = await api.listTracks();
      set({ tracks, isLoading: false });
    } catch {
      set({ isLoading: false });
    }
  },

  setSearchQuery: async (query: string) => {
    set({ searchQuery: query });
    const tracks = await api.searchTracks(query);
    set({ tracks });
  },

  setActiveView: (activeView) => set({ activeView }),

  playTrack: (track: Track) => {
    set({
      current_track: track,
      status: 'Playing',
      position: {
        position_ms: 0,
        duration_ms: track.duration_ms,
        playing: true,
      },
    });
  },

  togglePlay: () => {
    const { status, current_track, tracks } = get();
    if (!current_track && tracks.length > 0) {
      get().playTrack(tracks[0]);
      return;
    }

    if (status === 'Playing') {
      set((s) => ({
        status: 'Paused',
        position: { ...s.position, playing: false },
      }));
    } else if (status === 'Paused' || status === 'Stopped') {
      set((s) => ({
        status: 'Playing',
        position: { ...s.position, playing: true },
      }));
    }
  },

  nextTrack: () => {
    const { tracks, current_track, shuffle, repeat } = get();
    if (tracks.length === 0) return;
    if (!current_track) {
      get().playTrack(tracks[0]);
      return;
    }

    if (repeat === 'One') {
      get().playTrack(current_track);
      return;
    }

    const currentIndex = tracks.findIndex((t) => t.id === current_track.id);
    let nextIndex = currentIndex + 1;

    if (shuffle) {
      nextIndex = Math.floor(Math.random() * tracks.length);
    } else if (nextIndex >= tracks.length) {
      if (repeat === 'All') {
        nextIndex = 0;
      } else {
        set({ status: 'Stopped', position: { position_ms: 0, playing: false } });
        return;
      }
    }

    get().playTrack(tracks[nextIndex]);
  },

  prevTrack: () => {
    const { tracks, current_track, repeat } = get();
    if (tracks.length === 0 || !current_track) return;

    const currentIndex = tracks.findIndex((t) => t.id === current_track.id);
    let prevIndex = currentIndex - 1;

    if (prevIndex < 0) {
      if (repeat === 'All') {
        prevIndex = tracks.length - 1;
      } else {
        prevIndex = 0;
      }
    }

    get().playTrack(tracks[prevIndex]);
  },

  seek: (ms: number) => {
    set((s) => ({
      position: { ...s.position, position_ms: ms },
    }));
  },

  setVolume: (volume: number) => {
    set({ volume: Math.max(0, Math.min(1, volume)) });
  },

  toggleShuffle: () => {
    set((s) => ({ shuffle: !s.shuffle }));
  },

  toggleRepeat: () => {
    const current = get().repeat;
    const nextRepeat: RepeatMode =
      current === 'Off' ? 'All' : current === 'All' ? 'One' : 'Off';
    set({ repeat: nextRepeat });
  },
}));
