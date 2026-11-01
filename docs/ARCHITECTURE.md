# ARCHITECTURE

Status: **Stage 3 draft** (2026-11-01). Owner approval pending.
Scope: system architecture, component boundaries, communication contract, threading, state ownership, lifecycle, repository layout.
Related: `AUDIO_ENGINE.md`, `DATABASE_DESIGN.md`, `TECHNICAL_DECISIONS.md`, `KNOWN_LIMITATIONS.md`.

Label legend used across all docs:
**[Verified]** confirmed in official docs/source during research. **[Limitation]** known constraint. **[Assumption]** reasonable but unproven. **[Unverified]** compatibility not confirmed; needs a check before use. **[Recommendation]** architectural choice made by the architect.

---

## 1. Guiding rules

1. **One source of truth per kind of data.** Playback state lives in Rust (`PlaybackManager`). Library data lives in SQLite. The React app only holds *copies* (caches) of both.
2. **The frontend never touches audio objects or file paths.** It sends commands ("play track 42") and receives state events.
3. **The audio engine and playback logic do not depend on Tauri.** They are plain Rust crates. This lets us (a) test them without a window, (b) run them from a Linux command-line harness during prototyping, and (c) host them inside an Android service that exists without the WebView.
4. **Real-time audio code never allocates, locks, does I/O or decodes.** Everything slow happens on other threads.
5. **Small surface, few dependencies.** Every new dependency must pass the dependency policy (exists, current API, maintained, licence, platform support, genuinely needed).

---

## 2. Component map

```
┌──────────────────────────── WebView (React + TS) ────────────────────────────┐
│ features/ (player, library, queue, playlists, settings)   stores/ (Zustand)   │
│ services/api.ts  ── typed wrappers over invoke() / listen() ──────────────┐   │
└───────────────────────────────────────────────────────────────────────────│───┘
                          Tauri commands (request/response) + events (push)  │
┌───────────────────────────────────────────────────────────────────────────▼───┐
│ src-tauri  (thin shell: NO business logic)                                    │
│  commands/  events bridge  platform/ (media controls, android bridge, dialogs)│
└───────┬───────────────────────────┬───────────────────────────┬───────────────┘
        │ PlayerHandle              │ LibraryHandle             │ trait impls
┌───────▼───────────┐       ┌───────▼───────────────┐   ┌───────▼──────────────┐
│ anahata-player    │       │ anahata-library       │   │ Android (Kotlin)     │
│ PlaybackManager   │◄──ids─┤ db · scanner ·        │   │ MediaSessionService  │
│ queue · shuffle · │       │ metadata · artwork    │   │ notification · focus │
│ repeat · state    │       │ (SQLite, Lofty, HTTP) │   │ ⇄ JNI ⇄ anahata-player│
└───────┬───────────┘       └───────────────────────┘   └──────────────────────┘
        │ EngineCommand (SPSC)
┌───────▼───────────┐
│ anahata-audio     │  decode (Symphonia) → resample → ring buffer → CPAL callback
│ voices · mixer ·  │
│ output            │
└───────────────────┘
```

### Responsibilities

| Component | Owns | Does NOT do |
|---|---|---|
| **React frontend** | UI, navigation, virtualized lists, optimistic UI only for cosmetic things, local interpolation of the progress bar | Decide what plays next, hold the full library, read files |
| **src-tauri (shell)** | Registering commands/events, wiring crates together, window lifecycle, system media controls, native dialogs, Android bridge, capability config | Business rules, SQL, audio |
| **anahata-player** | Queue, shuffle/repeat logic, playback state machine, "what plays next", persistence snapshots, error policy per track | Decoding, device I/O, SQL queries beyond restore/save |
| **anahata-audio** | Opening files, decoding, resampling, channel mapping, buffering, mixing, gapless join, crossfade, device output, device-change recovery | Queue logic, library, UI events |
| **anahata-library** | SQLite access, schema/migrations, scanner, metadata extraction (Lofty), artwork cache & optional online lookup, search | Playback |
| **Android Kotlin layer** | Foreground service, MediaSession, notification, audio focus, media buttons, permissions plumbing | Decoding or a second playback state |

---

## 3. Repository structure

Cargo **workspace** so the engine/player/library crates are separate from Tauri.

```
anahata/
├── Cargo.toml                    # [workspace]; shared lint + profile settings
├── package.json  vite.config.ts  tsconfig.json  tailwind config
├── crates/
│   ├── anahata-audio/            # engine; no tauri, no sqlite
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── source.rs         # Symphonia open/probe/decode/seek wrapper -> f32
│   │   │   ├── resample.rs       # sample-rate conversion (decode thread only)
│   │   │   ├── channels.rs       # channel mapping/downmix
│   │   │   ├── ring.rs           # SPSC f32 ring buffer wrapper
│   │   │   ├── voice.rs          # one track's decode worker + ring + counters
│   │   │   ├── mixer.rs          # callback-side mixing: gain ramps, gapless join, crossfade
│   │   │   ├── output.rs         # CPAL stream, device selection, error/disconnect handling
│   │   │   ├── engine.rs         # public Engine API (commands in, events out)
│   │   │   └── error.rs
│   │   ├── examples/play_cli.rs  # Linux command-line harness (prototype P1-P4)
│   │   └── tests/                # + tests/assets/ small licensed/generated samples
│   ├── anahata-player/           # playback manager; no tauri
│   │   └── src/ manager.rs state.rs queue.rs shuffle.rs persist.rs events.rs observer.rs
│   └── anahata-library/          # no tauri
│       ├── migrations/           # 0001_init.sql, ...
│       └── src/ db/ (writer.rs reader.rs migrate.rs queries/) scanner/ metadata/ artwork/ playlists/ settings/ search/ types.rs error.rs
├── src-tauri/
│   ├── Cargo.toml  tauri.conf.json
│   ├── capabilities/             # minimal permission sets per window/platform
│   └── src/
│       ├── lib.rs                # builder, state setup, plugin registration, shutdown hook
│       ├── commands/             # player.rs queue.rs library.rs playlists.rs settings.rs artwork.rs
│       ├── events.rs             # event names + payload types + emitters
│       ├── error.rs              # AppError -> serializable
│       └── platform/             # media_controls.rs  android.rs  dialogs.rs  (cfg-gated)
├── plugins/
│   └── tauri-plugin-anahata-android/   # Kotlin: PlaybackService, MediaSession, notification, focus, JNI glue
├── src/                          # React app
│   ├── components/ (ui/ = shadcn, shared)
│   ├── features/ player/ library/ albums/ artists/ playlists/ queue/ settings/
│   ├── hooks/  services/ (api.ts, events.ts)  stores/  types/ (generated + hand-written)  styles/
│   └── layouts/ desktop/ mobile/
├── tests/                        # Playwright e2e; cross-crate integration
├── docs/                         # all project docs
└── README.md
```

Why these directories exist:
- `crates/anahata-audio` is the highest-risk code; isolating it makes it testable and reusable from Android.
- `crates/anahata-player` separates *what to play* from *how to play*.
- `crates/anahata-library` keeps SQL/scanner testable without Tauri.
- `plugins/…android` keeps Kotlin in one reviewable place.
- `src/layouts/{desktop,mobile}` because Android is a separate layout, not a shrunk desktop.

**[Unverified]** Tauri 2 mobile builds with a Cargo workspace layout; verify in milestone M2 before committing to it. Fallback: crates live under `src-tauri/crates/` as path dependencies.

---

## 4. Threading model

| Thread(s) | Count | Owns | Rules |
|---|---|---|---|
| Tauri runtime / async tasks | managed by Tauri | command dispatch | Never block; heavy work goes to `spawn_blocking` or dedicated threads |
| **Player thread** | 1 | `PlaybackManager` state | Single owner → no locks on playback state. Loop: wait on command channel with ~20 ms timeout, then drain engine events |
| **Decode worker** | ≤ 2 active (current + preloaded next) | one `Source` each | Decodes, resamples, pushes to ring. Wrapped in `catch_unwind` so a panic on a malformed file fails that track, not the app |
| **Audio callback** | 1 (owned by OS/CPAL) | mixer state | Real-time rules (§1.4). Pops from rings, applies gain, writes output |
| **DB writer** | 1 | the only write connection | Receives batched write jobs |
| **DB readers** | pool of 4 (fixed) | read-only connections | Used from `spawn_blocking` |
| **Scanner** | 1 walker + N metadata workers (N = min(4, cores−1), configurable) | traversal & parsing | Bounded channels for back-pressure; cancellation token checked per file |
| **Artwork worker** | 1 | HTTP + image processing | Rate limiter ≥ 1.1 s between MusicBrainz calls; queue persisted via `artwork_lookup` |
| Android service thread | OS-managed | Kotlin service | Calls Rust via JNI; Rust calls Kotlin via attached thread |

**[Recommendation]** Do not set `panic = "abort"` in release profiles; `catch_unwind` on decode workers depends on unwinding.

---

## 5. Playback state: one source of truth

`PlaybackManager` (in `anahata-player`) owns:

```
PlayerState {
  status:        Stopped | Loading | Playing | Paused | Error
  queue_revision u64            // bumps on any queue change
  current:       Option<{ queue_pos: u32, queue_item_id: u64, track_id: i64 }>
  position:      { position_ms: u64, anchor_time_ms: u64 (monotonic), playing: bool }
  volume:        0.0..=1.0
  shuffle:       bool
  repeat:        Off | All | One
  crossfade:     { enabled: bool, seconds: u8 (1..=12) }
  error:         Option<{ track_id, code, message }>
  output:        { device_name, sample_rate, channels }   // informational
}
```

Rules:
1. Only the player thread mutates it. All mutations come from `PlayerCommand`s (from Tauri, Android service, media keys, or the engine's own events).
2. After each change it emits `PlayerEvent`s to registered **observers** (trait `PlayerObserver`): the Tauri bridge (→ `player:state`), the OS media-controls bridge, the Android bridge. All see the same events in the same order.
3. The engine reports *facts* (track started, track ended, underrun, device lost). The manager decides *consequences* (advance queue, skip bad track, retry device).
4. **No polling by the frontend.** Position is sent as an *anchor* (`position_ms` + monotonic time + playing flag) whenever it changes meaning (play, pause, seek, track change) plus a ~1 s drift-correction heartbeat. The UI interpolates the progress bar locally.

### Queue model
- Queue = ordered list of items `{ queue_item_id (unique u64), track_id }`. Up to 100k+ items (≈ 16 bytes each in memory ≈ 1.6 MB — acceptable).
- Shuffle keeps a separate `play_order` permutation; the current track is placed first when shuffle is enabled; "previous" walks back through play history within the permutation.
- Queue edits never interrupt the current track. If an edit changes the *next* track, the preloaded voice is discarded and a new preload begins.
- Play requests take a **source descriptor** (e.g. `Album{id}`, `AllSongs{sort}`, `Playlist{id}`, `Search{query}`); Rust resolves it to track ids. The frontend never sends 100k ids over IPC.
- Persistence: `queue_items` + `playback_state` tables (see `DATABASE_DESIGN.md`). Saved on change (debounced 2 s), position every 10 s while playing, and on pause/exit. A crash loses ≤ 10 s of position.
- Restore on launch: load queue and position, status = Paused/Stopped. **Never auto-play** unless the setting `playback.autoplay_on_launch` is on (default off).

---

## 6. Frontend ↔ backend contract

### 6.1 Principles
- Commands are named `domain_verb` (snake_case). Payloads are typed Rust structs; TypeScript types are **generated** from them (tool selection in `TECHNICAL_DECISIONS.md`, pending verification) so the two sides cannot drift.
- Commands return `Result<T, AppError>`. IDs are `i64` (safe as JS numbers; SQLite rowids stay far below 2^53).
- Long operations (scan, artwork) start a background task, return immediately, and report via events.
- Lists are **paged**: `{ items, total, offset, limit }`. Never return whole-library arrays.

### 6.2 Commands (v1 surface)

| Domain | Command | Purpose |
|---|---|---|
| player | `player_get_state` | Current snapshot (used on startup/reconnect) |
| | `player_play(source, start_index?)` | Replace queue from a source and start |
| | `player_pause` `player_resume` `player_toggle` `player_stop` | Transport |
| | `player_next` `player_previous` | Skip (uses short quick-fade, not long crossfade) |
| | `player_seek(position_ms)` | Seek in current track |
| | `player_set_volume(0..1)` `player_set_shuffle(bool)` `player_set_repeat(mode)` | Modes |
| queue | `queue_add(source, mode: Next \| End)` | Add tracks |
| | `queue_remove(queue_item_ids)` `queue_move(queue_item_id, to_pos)` `queue_clear` `queue_shuffle_now` | Edit |
| | `queue_get_page(offset, limit)` | Rows joined with track display data |
| | `queue_play_item(queue_item_id)` | Jump |
| library | `library_list_tracks(query, offset, limit)` | `query = {sort, filter, search?}` |
| | `library_list_albums` `library_list_artists` `library_list_genres` `library_list_recent` | Browse pages |
| | `library_get_album(id)` `library_get_artist(id)` | Detail with track pages |
| | `library_search(text, kinds, limit)` | FTS5-backed |
| | `library_pick_and_add_folder` | **Rust opens the native folder dialog itself** and adds the result; the frontend never supplies a path (path-injection defence) |
| | `library_remove_folder(id)` `library_list_folders` | Folder management |
| | `library_rescan(folder_id?)` `library_cancel_scan` | Scanning |
| playlists | `playlist_list` `playlist_create` `playlist_rename` `playlist_delete` `playlist_get_tracks_page` `playlist_add_tracks(source)` `playlist_remove_items` `playlist_move_item` | CRUD |
| settings | `settings_get` `settings_set(key, value)` | Typed keys only; unknown keys rejected |
| artwork | `artwork_clear_cache` `artwork_cache_stats` | Maintenance |

Images are not returned through commands. See §6.4.

### 6.3 Events (backend → frontend)

| Event | Payload | When |
|---|---|---|
| `player:state` | `PlayerState` (without live position) | Any state change |
| `player:position` | anchor `{position_ms, anchor_time_ms, playing}` | Play/pause/seek/track change + ~1 s heartbeat |
| `player:error` | `{track_id?, code, message}` | Track failed / device failed |
| `queue:changed` | `{revision, length}` | Queue edits (UI refetches the visible window) |
| `library:changed` | `{kinds: [...], reason}` | After scan batches / folder changes — **invalidation signal, not data** |
| `library:scan-progress` | `{folder_id, phase, files_seen, files_total?, added, updated, errors}` | Throttled to ≤ 4 Hz |
| `library:scan-finished` | `{scan_id, status, summary}` | End/cancel/fail |
| `library:folder-availability` | `{folder_id, availability}` | Drive appears/disappears |
| `artwork:ready` | `{artwork_id, album_id?}` | Async artwork arrived |
| `settings:changed` | `{key, value}` | Setting changed (also from other windows) |
| `device:changed` | `{device_name?, status}` | Output device changed/lost |

### 6.4 Artwork delivery
**[Recommendation]** A custom URI scheme handled in Rust, e.g. `anahata-art://<artwork_id>/<size>`, serves cached files from the app cache dir. Benefits: no base64 over IPC, browser caching works, and the frontend needs **no filesystem permission**. **[Unverified]** custom-protocol behaviour on Tauri Android — verify in M2/M12; fallback is the Tauri asset protocol scoped to the artwork cache directory only.

### 6.5 Error model
```
AppError { code: ErrorCode, message: String (user-presentable), detail: Option<String> (log only) }
ErrorCode: NotFound | InvalidInput | FileUnavailable | UnsupportedFormat | DecodeFailed
         | DeviceUnavailable | DatabaseError | ScanCancelled | NetworkDisabled
         | NetworkFailed | PermissionDenied | Internal
```
Rules: never swallow errors silently; log with context; user-visible messages are short and actionable; panics in workers become `Internal`/`DecodeFailed` for that operation only.

---

## 7. Lifecycle

### Startup (target: window visible fast; no blocking on library size)
1. Resolve app data dir (platform-appropriate). Initialise logging.
2. Open DB: run migrations (backup first if a migration is pending) → start writer + reader pool. If open/migrate fails → corruption recovery (see `DATABASE_DESIGN.md` §9).
3. Mark scans left in `running` as `interrupted`.
4. Create `PlayerRuntime` (player thread + engine). Restore queue/position (paused).
5. Register commands/events; show window. Frontend calls `player_get_state` and lists.
6. Background (low priority, after UI is up): `PRAGMA quick_check`, folder availability check, optional incremental rescan on launch if enabled, artwork queue resume.

### Shutdown
1. Tauri exit signal → `PlayerCommand::Shutdown`: 50 ms fade-out, stop stream, **synchronously** persist queue + position.
2. Cancel scanner and artwork worker; wait ≤ 2 s.
3. Writer drains queue, runs `wal_checkpoint(TRUNCATE)`, closes.
4. Desktop: closing the window while playing keeps playing only if tray/background mode is enabled (post-1.0); otherwise quit = stop.
5. **Android:** the *Activity* closing does not shut anything down; only the service stopping (user stops playback / swipes away notification) runs steps 1–3.

### Resource rules
- Open file handles: ≤ 2 audio files at once (current + preload).
- Rings: ≈ 1.5 s of audio per voice (~1 MB at 48 kHz stereo f32).
- Reader pool fixed at 4; writer 1.
- Scanner memory bounded by channel capacities (e.g. 256 pending files).

---

## 8. Platform boundary

Platform-specific code sits behind small traits in `src-tauri/platform/` or the Android plugin:
`SystemMediaControls` (souvlaki-backed on desktop, MediaSession on Android), `FolderPicker`, `AppDirs`, `LibrarySource` (path walker vs Android document tree/MediaStore). Everything else is shared. Detailed strategy: `PLATFORM_STRATEGY.md` (Stage 4).

**Android specifics that shape the architecture** (details in `AUDIO_ENGINE.md` §10):
- `PlayerRuntime` is created by the native library, **independently of the Activity/WebView**, as a process-wide singleton. The Tauri layer *attaches* to it.
- Kotlin service ⇄ Rust over JNI. **[Verified]** Tauri documents JNI calls into shared Rust code even when the WebView is suspended. **[Unverified]** that the Tauri runtime itself survives Activity destruction → Android spike (prototype P6) is a gate.

---

## 9. Security boundaries (summary; full list in `SECURITY_AND_RELIABILITY.md`)

- Frontend capabilities: only our own commands, events, and core window/event permissions. **No fs, shell, http or opener plugins exposed to the WebView.**
- Frontend never provides filesystem paths; playback and artwork are by id.
- All music metadata is untrusted: length-limited strings, sanitised before display (React escapes by default; never use `dangerouslySetInnerHTML` on tag data).
- Network access only inside the artwork worker; HTTPS only; host allowlist (`musicbrainz.org`, `coverartarchive.org`; **[Unverified]** Cover Art Archive redirects to `archive.org` image hosts — confirm and allow by suffix); response size and image dimension limits.
- FTS queries are built by quoting each token; user text is never concatenated into SQL.
- Strict CSP; no remote scripts.

---

## 10. Open items for the owner
See `DEVELOPMENT_LOG.md` → "Decisions awaiting owner".
