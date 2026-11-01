import React from 'react';
import {
  Play,
  Pause,
  SkipBack,
  SkipForward,
  Shuffle,
  Repeat,
  Repeat1,
  Volume2,
  VolumeX,
  Disc,
} from 'lucide-react';
import { usePlayerStore } from '../stores/playerStore';

export const PlayerBar: React.FC = () => {
  const {
    current_track,
    status,
    position,
    volume,
    shuffle,
    repeat,
    togglePlay,
    nextTrack,
    prevTrack,
    seek,
    setVolume,
    toggleShuffle,
    toggleRepeat,
  } = usePlayerStore();

  const isPlaying = status === 'Playing';
  const duration = position.duration_ms || current_track?.duration_ms || 1;
  const progressPercent = Math.min(100, (position.position_ms / duration) * 100);

  const formatTime = (ms: number) => {
    const totalSecs = Math.floor(ms / 1000);
    const mins = Math.floor(totalSecs / 60);
    const secs = totalSecs % 60;
    return `${mins}:${secs.toString().padStart(2, '0')}`;
  };

  const handleSeekChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const percent = parseFloat(e.target.value);
    const newPos = (percent / 100) * duration;
    seek(newPos);
  };

  return (
    <footer className="h-24 bg-surface border-t border-border px-6 flex items-center justify-between select-none z-10">
      {/* Currently Playing Track Info */}
      <div className="w-1/4 flex items-center gap-3 min-w-0">
        <div className="w-14 h-14 rounded-lg bg-surfaceHover border border-border flex items-center justify-center text-secondary overflow-hidden shrink-0 shadow-md">
          <Disc className={`w-8 h-8 ${isPlaying ? 'animate-spin text-primary' : ''}`} style={{ animationDuration: '6s' }} />
        </div>
        <div className="min-w-0">
          <p className="text-sm font-semibold text-slate-100 truncate">
            {current_track ? current_track.title : 'No track playing'}
          </p>
          <p className="text-xs text-secondary truncate mt-0.5">
            {current_track ? `${current_track.artist} — ${current_track.album}` : 'Select a track to start playback'}
          </p>
        </div>
      </div>

      {/* Main Transport & Progress Bar */}
      <div className="w-2/4 max-w-xl flex flex-col items-center gap-2">
        <div className="flex items-center gap-6">
          <button
            onClick={toggleShuffle}
            className={`transition-colors ${shuffle ? 'text-primary' : 'text-secondary hover:text-slate-200'}`}
            title="Toggle Shuffle"
          >
            <Shuffle className="w-4 h-4" />
          </button>

          <button
            onClick={prevTrack}
            className="text-secondary hover:text-slate-100 transition-colors"
            title="Previous track"
          >
            <SkipBack className="w-5 h-5 fill-current" />
          </button>

          <button
            onClick={togglePlay}
            className="w-10 h-10 rounded-full bg-slate-100 text-black flex items-center justify-center hover:scale-105 active:scale-95 transition-all shadow-md shadow-white/10"
            title={isPlaying ? 'Pause' : 'Play'}
          >
            {isPlaying ? (
              <Pause className="w-5 h-5 fill-current" />
            ) : (
              <Play className="w-5 h-5 fill-current ml-0.5" />
            )}
          </button>

          <button
            onClick={nextTrack}
            className="text-secondary hover:text-slate-100 transition-colors"
            title="Next track"
          >
            <SkipForward className="w-5 h-5 fill-current" />
          </button>

          <button
            onClick={toggleRepeat}
            className={`transition-colors ${repeat !== 'Off' ? 'text-primary' : 'text-secondary hover:text-slate-200'}`}
            title={`Repeat: ${repeat}`}
          >
            {repeat === 'One' ? <Repeat1 className="w-4 h-4" /> : <Repeat className="w-4 h-4" />}
          </button>
        </div>

        {/* Scrubber */}
        <div className="w-full flex items-center gap-3">
          <span className="text-[11px] font-mono text-secondary w-10 text-right">
            {formatTime(position.position_ms)}
          </span>
          <div className="relative flex-1 flex items-center">
            <input
              type="range"
              min="0"
              max="100"
              step="0.1"
              value={progressPercent || 0}
              onChange={handleSeekChange}
              className="w-full h-1.5 bg-surfaceHover rounded-lg appearance-none cursor-pointer accent-primary focus:outline-none"
            />
          </div>
          <span className="text-[11px] font-mono text-secondary w-10">
            {formatTime(duration)}
          </span>
        </div>
      </div>

      {/* Volume and Extra Controls */}
      <div className="w-1/4 flex items-center justify-end gap-3">
        <button
          onClick={() => setVolume(volume > 0 ? 0 : 0.8)}
          className="text-secondary hover:text-slate-100 transition-colors"
        >
          {volume === 0 ? <VolumeX className="w-5 h-5" /> : <Volume2 className="w-5 h-5" />}
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={volume}
          onChange={(e) => setVolume(parseFloat(e.target.value))}
          className="w-24 h-1.5 bg-surfaceHover rounded-lg appearance-none cursor-pointer accent-primary focus:outline-none"
        />
      </div>
    </footer>
  );
};
