import { create } from 'zustand';
import { api, isTauri, subscribe } from '../services/api';
import type {
  Folder,
  PlaybackPosition,
  PlayerState,
  RepeatMode,
  ScanProgress,
  ScanSummary,
  Track,
} from '../types';

export type View = 'library' | 'playlists' | 'albums' | 'settings';

const PAGE = 200;

interface PlayerStore extends PlayerState {
  /** Wall-clock time (ms) at which `position.position_ms` was true. Used to interpolate between backend updates. */
  positionAnchorAt: number;
  tracks: Track[];
  totalTracks: number;
  folders: Folder[];
  searchQuery: string;
  isLoading: boolean;
  activeView: View;
  backendAvailable: boolean;
  scanning: boolean;
  scanProgress: ScanProgress | null;
  lastScan: ScanSummary | null;
  errorMessage: string | null;

  init: () => Promise<() => void>;
  fetchTracks: () => Promise<void>;
  setSearchQuery: (query: string) => Promise<void>;
  setActiveView: (view: View) => void;
  addFolder: () => Promise<void>;
  rescanAll: () => Promise<void>;
  cancelScan: () => Promise<void>;
  dismissError: () => void;
  playTrack: (track: Track) => void;
  togglePlay: () => void;
  nextTrack: () => void;
  prevTrack: () => void;
  seek: (ms: number) => void;
  setVolume: (vol: number) => void;
  toggleShuffle: () => void;
  toggleRepeat: () => void;
}

const idleState: PlayerState = {
  status: 'Stopped',
  queue_revision: 0,
  current_queue_index: null,
  current_item: null,
  position: { position_ms: 0, duration_ms: null, playing: false },
  volume: 0.8,
  shuffle: false,
  repeat: 'Off',
  error: null,
};

/** Latest search/list request; responses from older requests are dropped. */
let requestSeq = 0;

export const usePlayerStore = create<PlayerStore>((set, get) => {
  const report = (e: unknown) => set({ errorMessage: String(e) });

  /** Runs a backend call; surfaces its failure to the user instead of swallowing it. */
  const run = (p: Promise<unknown>) => {
    p.catch(report);
  };

  return {
    ...idleState,
    positionAnchorAt: Date.now(),
    tracks: [],
    totalTracks: 0,
    folders: [],
    searchQuery: '',
    isLoading: false,
    activeView: 'library',
    backendAvailable: false,
    scanning: false,
    scanProgress: null,
    lastScan: null,
    errorMessage: null,

    init: async () => {
      if (!isTauri()) {
        set({
          backendAvailable: false,
          errorMessage: 'Anāhata is running outside its desktop shell, so there is no audio or library backend.',
        });
        return () => {};
      }
      set({ backendAvailable: true });

      const off = await subscribe({
        onState: (s) => set({ ...s, positionAnchorAt: Date.now() }),
        onPosition: (position: PlaybackPosition) => set({ position, positionAnchorAt: Date.now() }),
        onError: (message) => set({ errorMessage: message }),
        onScanProgress: (scanProgress) => set({ scanProgress }),
        onScanFinished: (lastScan) => set({ lastScan }),
      });

      try {
        const [state, folders] = await Promise.all([api.getPlayerState(), api.listFolders()]);
        set({ ...state, folders, positionAnchorAt: Date.now() });
      } catch (e) {
        report(e);
      }
      await get().fetchTracks();
      return off;
    },

    fetchTracks: async () => {
      const seq = ++requestSeq;
      set({ isLoading: true });
      try {
        const [tracks, totalTracks, folders] = await Promise.all([
          api.listTracks(PAGE, 0),
          api.countTracks(),
          api.listFolders(),
        ]);
        if (seq === requestSeq) set({ tracks, totalTracks, folders, isLoading: false });
      } catch (e) {
        if (seq === requestSeq) set({ isLoading: false });
        report(e);
      }
    },

    setSearchQuery: async (query: string) => {
      set({ searchQuery: query });
      if (!query.trim()) return get().fetchTracks();
      const seq = ++requestSeq;
      try {
        const tracks = await api.searchTracks(query, PAGE);
        if (seq === requestSeq) set({ tracks });
      } catch (e) {
        report(e);
      }
    },

    setActiveView: (activeView) => set({ activeView }),

    addFolder: async () => {
      try {
        const folder = await api.addFolder();
        if (!folder) return; // picker cancelled
        await get().fetchTracks();
        await runScan(folder.id);
      } catch (e) {
        report(e);
      }
    },

    rescanAll: async () => {
      try {
        const folders = await api.listFolders();
        for (const f of folders) {
          if (get().lastScan?.cancelled) break;
          await runScan(f.id);
        }
      } catch (e) {
        report(e);
      }
    },

    cancelScan: async () => {
      try {
        await api.cancelScan();
      } catch (e) {
        report(e);
      }
    },

    dismissError: () => set({ errorMessage: null }),

    playTrack: (track: Track) => {
      // The visible list becomes the queue; the backend resolves paths from ids.
      const ids = get().tracks.map((t) => t.id);
      run(api.playTracks(ids, track.id));
    },

    togglePlay: () => {
      const { current_item, tracks } = get();
      if (!current_item && tracks.length > 0) {
        get().playTrack(tracks[0]);
        return;
      }
      run(api.toggle());
    },

    nextTrack: () => run(api.next()),
    prevTrack: () => run(api.previous()),

    seek: (ms: number) => {
      set((s) => ({ position: { ...s.position, position_ms: ms }, positionAnchorAt: Date.now() }));
      run(api.seek(Math.max(0, Math.round(ms))));
    },

    setVolume: (volume: number) => {
      const v = Math.max(0, Math.min(1, volume));
      set({ volume: v });
      run(api.setVolume(v));
    },

    toggleShuffle: () => run(api.setShuffle(!get().shuffle)),

    toggleRepeat: () => {
      const current = get().repeat;
      const next: RepeatMode = current === 'Off' ? 'All' : current === 'All' ? 'One' : 'Off';
      run(api.setRepeat(next));
    },
  };

  async function runScan(folderId: number) {
    if (get().scanning) return;
    set({ scanning: true, scanProgress: null });
    try {
      const lastScan = await api.scanFolder(folderId);
      set({ lastScan });
    } catch (e) {
      report(e);
    } finally {
      set({ scanning: false, scanProgress: null });
      await get().fetchTracks();
    }
  }
});

/** Current playback position in ms, interpolated between backend updates while playing. */
export function interpolatedPositionMs(s: Pick<PlayerStore, 'position' | 'status' | 'positionAnchorAt'>): number {
  const base = s.position.position_ms;
  if (s.status !== 'Playing') return base;
  const elapsed = Date.now() - s.positionAnchorAt;
  const pos = base + Math.max(0, elapsed);
  return s.position.duration_ms != null ? Math.min(pos, s.position.duration_ms) : pos;
}
