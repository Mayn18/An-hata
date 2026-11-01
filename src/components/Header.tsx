import React from 'react';
import { Search, FolderPlus, RefreshCw } from 'lucide-react';
import { usePlayerStore } from '../stores/playerStore';

export const Header: React.FC = () => {
  const { searchQuery, setSearchQuery, fetchTracks } = usePlayerStore();

  return (
    <header className="h-16 border-b border-border px-8 flex items-center justify-between bg-background/50 backdrop-blur-sm">
      <div className="relative w-96">
        <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-secondary" />
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search by title, artist, or album..."
          className="w-full bg-surface border border-border rounded-full pl-9 pr-4 py-2 text-sm text-slate-100 placeholder:text-secondary focus:outline-none focus:border-primary/50 transition-colors"
        />
      </div>

      <div className="flex items-center gap-3">
        <button
          onClick={() => fetchTracks()}
          className="flex items-center gap-2 px-3 py-1.5 rounded-lg border border-border text-secondary hover:text-slate-100 hover:bg-surface text-sm transition-colors"
        >
          <RefreshCw className="w-4 h-4" />
          <span>Rescan</span>
        </button>

        <button
          onClick={() => alert('Folder dialog picker will open via Tauri in desktop mode.')}
          className="flex items-center gap-2 px-3.5 py-1.5 rounded-lg bg-primary/10 text-primary border border-primary/30 hover:bg-primary/20 text-sm font-medium transition-colors"
        >
          <FolderPlus className="w-4 h-4" />
          <span>Add Folder</span>
        </button>
      </div>
    </header>
  );
};
