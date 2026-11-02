import React, { useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { Header } from './components/Header';
import { TrackList } from './components/TrackList';
import { PlayerBar } from './components/PlayerBar';
import { AlbumGrid } from './components/AlbumGrid';
import { SettingsView } from './components/SettingsView';
import { usePlayerStore } from './stores/playerStore';

export const App: React.FC = () => {
  const { init, activeView, errorMessage, dismissError } = usePlayerStore();

  useEffect(() => {
    let off: (() => void) | undefined;
    let cancelled = false;
    init().then((unsubscribe) => {
      if (cancelled) unsubscribe();
      else off = unsubscribe;
    });
    return () => {
      cancelled = true;
      off?.();
    };
  }, [init]);

  return (
    <div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-slate-100">
      <div className="flex flex-1 overflow-hidden">
        <Sidebar />
        <main className="flex-1 flex flex-col min-w-0 bg-background overflow-hidden">
          <Header />
          {errorMessage && (
            <div role="alert" className="flex items-start justify-between gap-4 px-8 py-2 text-sm bg-red-500/10 text-red-200 border-b border-red-500/30">
              <span>{errorMessage}</span>
              <button onClick={dismissError} className="shrink-0 underline underline-offset-2">Dismiss</button>
            </div>
          )}
          {activeView === 'library' && <TrackList />}
          {activeView === 'albums' && <AlbumGrid />}
          {activeView === 'playlists' && (
            <div className="flex-1 flex items-center justify-center text-secondary">
              <p>Playlists are not built yet.</p>
            </div>
          )}
          {activeView === 'settings' && <SettingsView />}
        </main>
      </div>
      <PlayerBar />
    </div>
  );
};

export default App;
