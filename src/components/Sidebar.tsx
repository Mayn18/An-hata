import React from 'react';
import { Library, Disc3, ListMusic, Settings, Music2 } from 'lucide-react';
import { usePlayerStore } from '../stores/playerStore';

export const Sidebar: React.FC = () => {
  const { activeView, setActiveView } = usePlayerStore();

  const navItems = [
    { id: 'library', label: 'All Songs', icon: Library },
    { id: 'albums', label: 'Albums', icon: Disc3 },
    { id: 'playlists', label: 'Playlists', icon: ListMusic },
    { id: 'settings', label: 'Settings', icon: Settings },
  ] as const;

  return (
    <aside className="w-64 bg-surface border-r border-border flex flex-col justify-between p-4 select-none">
      <div>
        <div className="flex items-center gap-3 px-2 py-4 mb-6 border-b border-border/50">
          <div className="w-10 h-10 rounded-xl bg-gradient-to-tr from-sky-500 to-indigo-500 flex items-center justify-center text-white shadow-lg shadow-sky-500/20">
            <Music2 className="w-6 h-6" />
          </div>
          <div>
            <h1 className="font-bold text-lg tracking-wider text-slate-100">Anāhata</h1>
            <p className="text-xs text-secondary">Local Music Player</p>
          </div>
        </div>

        <nav className="space-y-1">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = activeView === item.id;
            return (
              <button
                key={item.id}
                onClick={() => setActiveView(item.id)}
                className={`w-full flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm font-medium transition-colors ${
                  isActive
                    ? 'bg-primary/10 text-primary border border-primary/20'
                    : 'text-secondary hover:text-slate-200 hover:bg-surfaceHover'
                }`}
              >
                <Icon className="w-5 h-5" />
                {item.label}
              </button>
            );
          })}
        </nav>
      </div>

      <div className="px-3 py-4 border-t border-border/40 text-xs text-secondary">
        <p>Lossless & Gapless Engine</p>
        <p className="text-[10px] text-secondary/60 mt-0.5">Symphonia + CPAL Powered</p>
      </div>
    </aside>
  );
};
