# Anāhata

A local-first music player. Rust engine (Symphonia decoding, CPAL output, SQLite library) with a React UI in a Tauri 2 shell.

**Status:** early development (milestone 1). Desktop playback, library scanning and search work. Playlists, gapless/crossfade, saved queue and Android are not built yet. See `docs/KNOWN_LIMITATIONS.md`.

## Layout

| Path | What it is |
| --- | --- |
| `crates/anahata-audio` | Decoder → resampler → ring buffer → real-time mixer → CPAL output |
| `crates/anahata-player` | Queue and playback state machine (`Player`), independent of any audio device |
| `crates/anahata-library` | SQLite schema, incremental scanner, search queries |
| `src-tauri` | Tauri shell: commands, events, capabilities |
| `src` | React UI (talks to the shell only through `src/services/api.ts`) |

## Prerequisites

- Rust (stable) and Node 22
- Linux: `libasound2-dev pkg-config libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev libgtk-3-dev librsvg2-dev libxdo-dev libssl-dev patchelf`
- macOS and Windows need no extra system libraries beyond the Tauri prerequisites.

## Run

```sh
npm install
npm run tauri dev          # desktop app
cargo run -p anahata-audio --example play_cli -- path/to/file.flac   # engine only
```

## Test

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm run build              # type-check + production bundle
```

The audio tests run without a sound card: they drive the decoder and mixer directly, and include a guard that fails if the audio callback path allocates or frees memory.

## Supported formats

MP3, FLAC, WAV, AAC/M4A, OGG Vorbis, AIFF. Opus files are not indexed (no Opus decoder in Symphonia 0.5).

## Security notes

The webview never supplies file paths: folders are chosen with a native dialog opened from Rust, scans are requested by folder id, and playback by track id. Commands are granted explicitly in `src-tauri/capabilities/default.json`, and a CSP is set in `tauri.conf.json`.
