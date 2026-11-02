import React, { useEffect, useState } from 'react';
import { Disc3 } from 'lucide-react';
import { api } from '../services/api';
import type { Album } from '../types';

export const AlbumGrid: React.FC = () => {
  const [albums, setAlbums] = useState<Album[] | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .listAlbums()
      .then((a) => alive && setAlbums(a))
      .catch((e) => alive && setFailed(String(e)));
    return () => {
      alive = false;
    };
  }, []);

  if (failed) return <p className="p-8 text-sm text-red-300">Could not load albums: {failed}</p>;
  if (!albums) return <p className="p-8 text-sm text-secondary">Loading albums…</p>;
  if (albums.length === 0) return <p className="p-8 text-sm text-secondary">No albums yet. Add a folder to get started.</p>;

  return (
    <div className="flex-1 overflow-y-auto px-8 py-6">
      <h2 className="text-2xl font-bold tracking-tight text-slate-100 mb-6">Albums</h2>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(180px,1fr))] gap-5">
        {albums.map((a) => (
          <div key={a.id} className="rounded-xl border border-border/60 bg-surface/40 p-4">
            <div className="aspect-square rounded-lg bg-surfaceHover flex items-center justify-center text-secondary mb-3">
              <Disc3 className="w-10 h-10" />
            </div>
            <p className="text-sm font-semibold text-slate-100 truncate">{a.title}</p>
            <p className="text-xs text-secondary truncate">{a.artist}</p>
            <p className="text-xs text-secondary/70 mt-1">
              {a.track_count} {a.track_count === 1 ? 'track' : 'tracks'}
              {a.year ? ` · ${a.year}` : ''}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
};
