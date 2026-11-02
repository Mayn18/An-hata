import React from 'react';
import { Play, Pause, Clock } from 'lucide-react';
import { usePlayerStore } from '../stores/playerStore';
import { Track } from '../types';

export const TrackList: React.FC = () => {
  const { tracks, totalTracks, current_item, status, playTrack, togglePlay, isLoading, searchQuery, folders } =
    usePlayerStore();

  const formatDuration = (ms: number) => {
    const totalSecs = Math.floor(ms / 1000);
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  };

  return (
    <div className="flex-1 overflow-y-auto px-8 py-6">
      <div className="flex items-center justify-between mb-6">
        <div>
          <h2 className="text-2xl font-bold tracking-tight text-slate-100">Songs</h2>
          <p className="text-sm text-secondary mt-1">{searchQuery.trim() ? `${tracks.length} results` : `${totalTracks} tracks in your library${totalTracks > tracks.length ? ` (showing first ${tracks.length})` : ''}`}</p>
        </div>
      </div>

      {tracks.length === 0 && !isLoading && (
        <div className="border border-dashed border-border rounded-xl p-10 text-center text-secondary text-sm">
          {searchQuery.trim()
            ? 'No songs match your search.'
            : folders.length === 0
              ? 'Your library is empty. Use “Add Folder” to choose where your music lives.'
              : 'No playable songs found in your folders yet.'}
        </div>
      )}

      {tracks.length > 0 && (
      <div className="border border-border/60 rounded-xl overflow-hidden bg-surface/40">
        <table className="w-full text-left text-sm">
          <thead className="bg-surface border-b border-border text-secondary text-xs uppercase tracking-wider">
            <tr>
              <th className="py-3 px-4 w-12 text-center">#</th>
              <th className="py-3 px-4">Title</th>
              <th className="py-3 px-4">Artist</th>
              <th className="py-3 px-4">Album</th>
              <th className="py-3 px-4 w-24 text-right pr-6">
                <Clock className="w-4 h-4 inline" />
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border/30">
            {tracks.map((track: Track, index: number) => {
              const isCurrent = current_item?.track_id === track.id;
              const isPlaying = isCurrent && status === 'Playing';

              return (
                <tr
                  key={track.id}
                  onDoubleClick={() => playTrack(track)}
                  className={`group hover:bg-surfaceHover transition-colors cursor-pointer ${
                    isCurrent ? 'bg-primary/5 text-primary' : 'text-slate-200'
                  }`}
                >
                  <td className="py-3.5 px-4 text-center font-mono text-xs text-secondary">
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        if (isCurrent) {
                          togglePlay();
                        } else {
                          playTrack(track);
                        }
                      }}
                      className="w-6 h-6 rounded flex items-center justify-center text-slate-300 group-hover:text-primary transition-colors"
                    >
                      {isPlaying ? (
                        <Pause className="w-4 h-4 fill-current text-primary" />
                      ) : (
                        <Play className="w-4 h-4 fill-current opacity-0 group-hover:opacity-100" />
                      )}
                      {!isPlaying && !isCurrent && (
                        <span className="group-hover:hidden">{index + 1}</span>
                      )}
                    </button>
                  </td>
                  <td className="py-3.5 px-4 font-medium">
                    <span className={isCurrent ? 'text-primary font-semibold' : ''}>
                      {track.title}
                    </span>
                  </td>
                  <td className="py-3.5 px-4 text-secondary">{track.artist}</td>
                  <td className="py-3.5 px-4 text-secondary">{track.album}</td>
                  <td className="py-3.5 px-4 text-right font-mono text-xs text-secondary pr-6">
                    {formatDuration(track.duration_ms)}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      )}
    </div>
  );
};
