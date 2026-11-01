# TECHNICAL_DECISIONS

Status: **Stage 3 draft** (2026-11-01). Each entry: decision, alternatives, reasoning, status.
Status values: **Decided** (architect decision, within the brief) · **Proposed** (needs owner approval) · **Gate** (decided provisionally; a prototype can overturn it) · **Verify** (dependency must pass the dependency policy before use).
Labels: [Verified] [Limitation] [Assumption] [Unverified] [Recommendation].

---

## A. Stack evaluation

| # | Component | Verdict | Reasoning / findings |
|---|---|---|---|
| A1 | Tauri 2 | **Decided** | Required by the brief. [Verified] Android plugins can be written in Kotlin with `@TauriPlugin`/`@Command` and can call shared Rust via JNI even when the WebView is suspended. [Unverified] that the Tauri runtime stays alive without the Activity → prototype P6. Desktop and mobile do **not** have identical capabilities (see `PLATFORM_STRATEGY.md`, Stage 4) |
| A2 | React + TS + Vite + Tailwind + shadcn/ui + Lucide + Zustand | **Decided** (with two proposed additions, C1–C2) | Common, well-supported, fits Tauri. shadcn/ui is copy-in components, so no runtime dependency lock-in |
| A3 | Rust + rusqlite (`bundled`) | **Decided** | Own SQLite on every platform; FTS5 compiled in [Verified from build script history and a current-version report]; CI test proves it per target |
| A4 | Lofty | **Decided** | [Verified] v0.24.0 covers MP3/ID3, FLAC, MP4, Ogg (Vorbis/Opus/Speex), WAV/AIFF (RIFF/IFF), AAC, APE, Musepack, WavPack. [Unverified] bit depth for ALAC and artwork extraction per format → covered by sample-file tests in M6 |
| A5 | Symphonia (decode) | **Decided** | Pure safe Rust, fuzz-tested, no C dependencies, MPL-2.0. Gaps: M4A gapless, Opus decoder (see A8, A9). Features to enable explicitly: `flac`, `mp3` (`mpa`), `wav`, `aiff`, `ogg`, `vorbis`, `isomp4`, `aac`, `alac`, `pcm`. Pin exact version in M1 |
| A6 | CPAL (output) | **Decided** | [Verified] v0.18.2: AAudio on Android (API 26+), MSRV 1.85; desktop ALSA/WASAPI/CoreAudio. Gaps (SRC, mixing, buffering) are ours to build. Rejected: **Rodio** (hides sample-accurate control needed for gapless/crossfade); **GStreamer** (heavy, hard to ship on Android); **Oboe/AAudio directly** (only if P6 shows CPAL's Android backend inadequate — [Unverified] its stream usage attributes) |
| A7 | Custom playback manager + mixer | **Decided** | See `AUDIO_ENGINE.md` |
| A8 | Opus | **Gate** | [Limitation] Symphonia's Opus decoder is listed without a status (in work). Option: `symphonia-adapter-libopus` (C `libopus`) — [Unverified] builds on Android NDK/Windows/macOS → prototype P8. If it fails on any target, Opus is "not supported in 1.0" and removed from the supported-format list |
| A9 | M4A AAC/ALAC gapless | **Decided: stretch goal** | [Limitation] Symphonia's MP4 demuxer isn't gapless. Plain M4A playback is supported; gapless for M4A requires our own trimming (iTunSMPB/edit list). Documented as limitation until done |
| A10 | Android audio path | **Gate (plan A)** | One Rust engine hosted in a Kotlin foreground service. Fallback plan B = Media3/ExoPlayer on Android. Decision point: P6 |

## B. Architecture decisions

| # | Decision | Alternatives considered | Reasoning |
|---|---|---|---|
| B1 | Cargo workspace with `anahata-audio`, `anahata-player`, `anahata-library`, thin `src-tauri` | Everything inside `src-tauri/src/*` (the brief's suggested layout) | The suggested layout is preserved logically (same module names) but split into crates so the engine has **no Tauri dependency**: testable, CLI-prototypable, hostable by an Android service. [Unverified] Tauri mobile + workspace → checked in M2, fallback path-crates under `src-tauri/` |
| B2 | Single `PlaybackManager` thread owns playback state; observers receive events | Mutex-shared state; frontend-owned state | No lock contention, one ordering of events for UI/OS/Android |
| B3 | Frontend sends *source descriptors*, not id lists or paths | Frontend sends ids/paths | Avoids 100k-element IPC payloads and path-injection |
| B4 | `library_pick_and_add_folder` opens the dialog in Rust | Frontend dialog plugin → passes path | Frontend never supplies filesystem paths; smaller attack surface. [Unverified] exact dialog-plugin API; Android uses a document-tree picker |
| B5 | Offset pagination + virtualised lists | Keyset pagination; load-all | Random scroll access; measured at 100k in M12 (fallback keyset) |
| B6 | Explicit FTS maintenance in scanner transactions | Triggers; external-content FTS | Indexed text spans several tables |
| B7 | Artwork via custom URI scheme `anahata-art://` | base64 over IPC; asset protocol with scope | No fs permission for WebView, cacheable. [Unverified] on Android |
| B8 | DB: 1 writer thread + 4 readers (WAL) | Single shared connection; r2d2 pool | Simple, no extra dependency, concurrent UI reads during scans |
| B9 | Missing files kept 30 days before purge; unavailable folder ≠ deleted files | Delete immediately | External drives and temporary mounts must not destroy playlists |
| B10 | No auto-play on launch (setting, default off); restore paused | – | Required by brief |
| B11 | Manual skip uses 100 ms quick fade, long crossfade only on natural transitions | Crossfade on every change | Predictable; fewer settings |
| B12 | Stereo-only output in v1 (downmix multichannel) | Native multichannel output | Simplicity; consumer headphones/speakers. Documented limitation |

## C. Proposed additions (owner approval needed; each also needs dependency-policy verification)

| # | Proposal | Why | Cost / risk |
|---|---|---|---|
| C1 | `@tanstack/react-query` for paged library queries and invalidation on `library:changed` | Handles caching/eviction/refetch for paged lists; hand-rolling this in Zustand is error-prone | One more dependency (widely used) |
| C2 | `@tanstack/react-virtual` (or `react-window`) for virtualised lists | Required by the brief ("virtualized lists"); needed at 100k rows | Small |
| C3 | Rust→TypeScript type generation (candidates: `ts-rs`, `tauri-specta`) | Prevents the command/event types drifting | [Unverified] compatibility with Tauri 2 commands/events; Jules must verify and propose before adopting |

## D. Dependencies to verify before use (candidates — not yet verified)

| Need | Candidates | Notes |
|---|---|---|
| Ring buffer | `rtrb`, `ringbuf` | SPSC, wait-free; check maintenance/licence |
| Resampler | `rubato` | Must support delay compensation and flush (P3) |
| Hash for fingerprints & artwork dedupe | a fast 128-bit+ hash crate (e.g. BLAKE3 family) or std-only alternative | Pick one; record here |
| Directory walking | `std::fs` or `walkdir` | std is enough; add crate only if justified |
| Folder monitoring (desktop only, later) | `notify` | Deferred to a late milestone; not on Android |
| HTTP client for artwork | blocking client on the artwork thread (e.g. `ureq`) with **rustls** | Avoid OpenSSL on Android; verify |
| Image decode/resize | `image` | For thumbnails; verify features to limit size |
| Desktop media controls | `souvlaki` | [Verified] Linux MPRIS, Windows (needs HWND), macOS (needs main-thread event loop); last release I could confirm: 0.7.3 (Jan 2024) → re-check maintenance; wrap behind our own trait so it's replaceable |
| Android file access | `tauri-plugin-android-fs` (third-party) | [Verified] provides tree picker, persistent permission, directory listing, and opens files as `std::fs::File`. Third-party risk → alternatively our own Kotlin plugin; Stage 4 |
| Android foreground service / media session | **Own plugin** (`plugins/tauri-plugin-anahata-android`) | Existing third-party plugins don't match (background-service declares `DATA_SYNC`, not media playback; media-session plugin separate from service/focus). Use only as reference |
| Migrations | own runner (~60 lines) or `rusqlite_migration` | Either acceptable |
| Opus | `symphonia-adapter-libopus` | Gate A8 |

## E. External service constraints
- **MusicBrainz** [Verified]: ≤ 1 request/second per client, meaningful `User-Agent` required, no API key, **free for non-commercial use** (commercial use needs a plan) → note in `KNOWN_LIMITATIONS.md` if the app is ever monetised.
- **Cover Art Archive**: no key; [Unverified] redirect hosts → allowlist by suffix after checking.
