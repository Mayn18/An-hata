# KNOWN_LIMITATIONS

Status: **Stage 3 draft** (2026-11-01). Updated as prototypes report. Nothing here is "tested" unless stated.

## 1. Verified limitations (from official docs / source)
| ID | Limitation | Source summary | Impact |
|---|---|---|---|
| L1 | Symphonia AAC-LC decoder and ISO/MP4 container are **not flagged gapless** | Symphonia README tables | M4A (AAC, ALAC) won't be gapless unless we add our own trimming |
| L2 | Symphonia **Opus decoder has no status** (in work / not started) | Symphonia README status definitions | Opus depends on `libopus` adapter (C code) or waits for upstream |
| L3 | Symphonia gapless is **off by default** and only trims; **joining tracks is the application's job** | `FormatOptions` docs; maintainer discussion | We implement the join (AUDIO_ENGINE §7) |
| L4 | Symphonia and CPAL provide **no resampling or mixing** | Library scopes | We build resampler integration, ring buffers, mixer |
| L5 | CPAL Android backend needs **Android 8.0 (API 26)+** | CPAL docs | Minimum supported Android = 8.0 |
| L6 | AAudio: stream must not be stopped/closed/reopened **inside** the error callback | Android/AAudio docs | Device-recovery runs on another thread |
| L7 | Android 14+: foreground services need a declared type + permission | Android docs | Manifest `mediaPlayback` + `FOREGROUND_SERVICE_MEDIA_PLAYBACK` |
| L8 | MusicBrainz: ≤ 1 req/s, identifying User-Agent, non-commercial free tier | MusicBrainz API docs | Artwork lookup is slow by design; commercial use needs a plan |
| L9 | SQLite FTS5 `unicode61` doesn't segment CJK | FTS5 design | CJK search limited to prefix/whole tokens unless trigram fallback added |
| L10 | souvlaki: Windows needs a window handle; macOS needs the main-thread event loop | souvlaki docs | Media controls initialised after window creation on those platforms |
| L11 | Tauri desktop and mobile differ (plugins, windowing, background execution) | Tauri docs | Two layouts; platform traits |

## 2. Design limitations accepted for v1.0 (architect decisions)
| ID | Limitation | Reason |
|---|---|---|
| D1 | Output is **stereo**; multichannel is downmixed | Simplicity |
| D2 | Files with **non-UTF-8 paths** are skipped (counted in scan report) | Portable path storage |
| D3 | Albums are grouped by album tag + album artist; untagged compilations may split | No fuzzy matching in v1 |
| D4 | Multi-artist tags are not split into separate artists (raw text stored as `artist_display`) | Scope |
| D5 | Duplicate detection uses a sampled content fingerprint (size + first/last 64 KiB) | Speed on 100k files |
| D6 | No ReplayGain/normalisation; no EQ/effects | Out of scope per requirements |
| D7 | Manual skips use a 100 ms quick fade, not the configured crossfade | Predictability |
| D8 | No folder monitoring on Android (rescan on launch/manual) | OS limits; owner-delegated decision |
| D9 | Crossfade unavailable across a boundary where track length is unknown | Avoids guessing |
| D10 | Artwork: no local-folder `cover.jpg` handling unless added in M11 (requirements list "existing local artwork cache" — interpreted as the app's own cache) | See open question in log |

## 3. Unverified — requires a prototype or check before relying on it
| ID | Question | Resolved by |
|---|---|---|
| U1 | Does the Tauri Rust runtime/our engine survive Activity destruction with a foreground service alive? | P6 (gate for plan A) |
| U2 | CPAL AAudio stream usage attributes (media vs call), volume slider, audio-focus interplay | P6 |
| U3 | MP3 gapless in current Symphonia release (one third-party report says 0.6 alpha demuxer lacks per-packet trims) | P2 |
| U4 | Resampler delay/flush correctness across gapless joins | P3 |
| U5 | Accurate seek on VBR MP3 without TOC; seek latency | P5 |
| U6 | `libopus` adapter builds on all four targets | P8 |
| U7 | Cargo workspace + Tauri mobile build | M2 |
| U8 | Custom URI scheme for artwork on Android | M2/M12 |
| U9 | SAF tree scanning performance vs MediaStore on large libraries | Android library spike (M13) |
| U10 | OFFSET pagination and FTS speed at 100k tracks | M12 benchmarks |
| U11 | `VACUUM INTO` and FTS5 `trigram` available in bundled SQLite | M4 CI tests |
| U12 | Cover Art Archive redirect hosts for the network allowlist | M11 |
| U13 | souvlaki maintenance status and Tauri window-handle integration | M7/M14 |
| U14 | Rust→TS type-generation tool compatibility | M2 |
| U15 | Dialog plugin API for Rust-side folder picking on desktop | M2 |
| U16 | Linux packaging/runtime audio dependencies (ALSA/PipeWire) | Stage 4 |
| U17 | Windows/macOS behaviour (WASAPI/CoreAudio, signing/notarisation) — cannot be tested from the owner's Linux machine; needs CI runners | Stage 4/M14 |

## 4. Assumptions
- A1 Local-file playback only; files are readable `std::fs::File`/seekable streams (also on Android via document-tree file descriptors).
- A2 A typical user library ≤ 100k tracks, mostly local SSD/HDD.
- A3 ~1.5 s ring buffers are enough to avoid underruns on typical hardware (tune after P1).
- A4 Jules runs on Linux in the cloud: it **cannot** test audio output, Android device behaviour, or macOS/Windows. Audio-quality, Android and OS-integration acceptance tests must be run by the owner (see roadmap in Stage 5).
