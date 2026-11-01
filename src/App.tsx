import React, { useEffect } from 'react';
import { Sidebar } from './components/Sidebar';
import { Header } from './components/Header';
import { TrackList } from './components/TrackList';
import { PlayerBar } from './components/PlayerBar';
import { usePlayerStore } from './stores/playerStore';

export const App: React.FC = () => {
  const { fetchTracks, activeView } = usePlayerStore();

  useEffect(() => {
    fetchTracks();
  }, [fetchTracks]);

  return (
    <div className="flex flex-col h-screen w-screen overflow-hidden bg-background text-slate-100">
      <div className="flex flex-1 overflow-hidden">
        <Sidebar />
        <main className="flex-1 flex flex-col min-w-0 bg-background overflow-hidden">
          <Header />
          {activeView === 'library' && <TrackList />}
          {activeView === 'albums' && (
            <div className="flex-1 flex items-center justify-center text-secondary">
              <p>Albums grid view will appear here.</p>
            </div>
          )}
          {activeView === 'playlists' && (
            <div className="flex-1 flex items-center justify-center text-secondary">
              <p>Playlists manager will appear here.</p>
            </div>
          )}
          {activeView === 'settings' && (
            <div className="flex-1 p-8 text-slate-300">
              <h3 className="text-xl font-bold mb-4 text-white">Playback Settings</h3>
              <p className="text-sm text-secondary">Crossfade, audio output device selection, and library folders.</p>
            </div>
          )}
        </main>
      </div>
      <PlayerBar />
    </div>
  );
};

export default App;
