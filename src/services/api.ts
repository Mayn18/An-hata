import { Track } from '../types';

const INITIAL_MOCK_TRACKS: Track[] = [
  {
    id: 1,
    title: "Symphony No. 5 in C Minor, Op. 67",
    artist: "Ludwig van Beethoven",
    album: "Masterpieces of Classical Music",
    duration_ms: 452000,
    rel_path: "beethoven_sym5.flac",
    folder_location: "/music/classical",
  },
  {
    id: 2,
    title: "Clair de Lune",
    artist: "Claude Debussy",
    album: "Suite bergamasque",
    duration_ms: 312000,
    rel_path: "debussy_clair_de_lune.flac",
    folder_location: "/music/classical",
  },
  {
    id: 3,
    title: "Solaris (Ambient Drift)",
    artist: "Anāhata Soundscapes",
    album: "Resonance Vol. 1",
    duration_ms: 284000,
    rel_path: "solaris.mp3",
    folder_location: "/music/ambient",
  },
  {
    id: 4,
    title: "Echoes of the High Peaks",
    artist: "Himalayan Drone",
    album: "Mountain Meditation",
    duration_ms: 540000,
    rel_path: "echoes.flac",
    folder_location: "/music/meditation",
  },
  {
    id: 5,
    title: "Kind of Blue (So What)",
    artist: "Miles Davis",
    album: "Kind of Blue",
    duration_ms: 562000,
    rel_path: "miles_davis_so_what.wav",
    folder_location: "/music/jazz",
  }
];

export const api = {
  async listTracks(limit = 50, offset = 0): Promise<Track[]> {
    // In Tauri desktop/mobile runtime, this calls window.__TAURI__.invoke('library_list_tracks')
    return INITIAL_MOCK_TRACKS.slice(offset, offset + limit);
  },

  async searchTracks(query: string): Promise<Track[]> {
    const q = query.toLowerCase().trim();
    if (!q) return INITIAL_MOCK_TRACKS;
    return INITIAL_MOCK_TRACKS.filter(
      (t) =>
        t.title.toLowerCase().includes(q) ||
        t.artist.toLowerCase().includes(q) ||
        t.album.toLowerCase().includes(q)
    );
  },

  async scanDirectory(_path: string): Promise<{ files_seen: number; added: number }> {
    return { files_seen: 42, added: 5 };
  },

  async togglePlayback(): Promise<void> {},
  async seek(_posMs: number): Promise<void> {},
  async setVolume(_volume: number): Promise<void> {},
};
