import React from 'react';
import { usePlayerStore } from '../stores/playerStore';

export const SettingsView: React.FC = () => {
  const { folders, lastScan } = usePlayerStore();
  return (
    <div className="flex-1 overflow-y-auto p-8 text-slate-300">
      <h3 className="text-xl font-bold mb-4 text-white">Library folders</h3>
      {folders.length === 0 ? (
        <p className="text-sm text-secondary">No folders yet. Use “Add Folder” in the header.</p>
      ) : (
        <ul className="space-y-2">
          {folders.map((f) => (
            <li key={f.id} className="rounded-lg border border-border/60 bg-surface/40 px-4 py-3">
              <p className="text-sm font-medium text-slate-100 truncate">{f.display_name}</p>
              <p className="text-xs text-secondary truncate">{f.location}</p>
              <p className="text-xs text-secondary/80 mt-1">
                {f.track_count} tracks ·{' '}
                {f.availability === 'unavailable' ? 'not reachable (drive unplugged?)' : f.availability}
                {f.last_scan_at ? ` · last scanned ${new Date(f.last_scan_at).toLocaleString()}` : ' · never scanned'}
              </p>
            </li>
          ))}
        </ul>
      )}

      {lastScan && (
        <div className="mt-8">
          <h3 className="text-xl font-bold mb-3 text-white">Last scan</h3>
          <p className="text-sm text-secondary">
            {lastScan.cancelled ? 'Cancelled. ' : ''}
            {lastScan.files_seen} files seen · {lastScan.added} added · {lastScan.updated} updated · {lastScan.moved} moved ·{' '}
            {lastScan.missing} missing · {lastScan.duplicates} duplicates · {lastScan.skipped} unchanged ·{' '}
            {lastScan.unsupported} unsupported · {lastScan.errors} errors
          </p>
        </div>
      )}
    </div>
  );
};
