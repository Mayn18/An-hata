# Anāhata — Repository Audit and First Implementation Plan

**Author:** Jules (Senior Rust / Audio Engine / Tauri Engineer)
**Date:** 2026-11-01
**Document Status:** Stage 3 Architecture Review & Milestone 1 Proposal (Awaiting Owner Approval)

---

## Executive Summary

Anāhata is conceived as a cross-platform (Desktop & Android), local-first music player prioritizing audio fidelity, gapless playback, clean UX, resilience against corrupt files, and performant handling of large libraries (100,000+ tracks).

An inspection of the repository and the architecture design documents (`ARCHITECTURE.md`, `AUDIO_ENGINE.md`, `DATABASE_DESIGN.md`, `TECHNICAL_DECISIONS.md`, `KNOWN_LIMITATIONS.md`, and `DEVELOPMENT_LOG.md`) reveals that the engineering design is mature, well-thought-out, and pragmatically structured. However, **no source code has yet been implemented**. The repository currently contains only initial Git history and design documentation.

This document serves as the repository audit and proposes a focused, low-risk **Milestone 1 (M1)**: building the standalone audio engine core (`anahata-audio`) and validating it via an interactive CLI test harness before introducing Tauri, React, SQLite, or Android complexities.

---

## 1. Repository Status: What Exists and What Is Missing

### 1.1 What Exists
- **Git State:** A clean git repository initialized on branch `jules-14957606563606566329-98be70c0` with a single initial commit.
- **Architectural Documentation:** Six design documents provided in `docs/`:
  - `ARCHITECTURE.md`: High-level system structure, threading, contracts, and lifecycle.
  - `AUDIO_ENGINE.md`: Detailed DSP pipeline, Symphonia/CPAL bridge, voices, and prototype plan.
  - `DATABASE_DESIGN.md`: SQLite schema, indexing strategy, FTS5 design, move/duplicate detection, and migrations.
  - `TECHNICAL_DECISIONS.md`: Technology stack evaluation, architectural decisions, and candidate dependencies.
  - `KNOWN_LIMITATIONS.md`: Verified, accepted, and unverified limitations and assumptions.
  - `DEVELOPMENT_LOG.md`: Project status tracker, self-check findings, and open questions awaiting owner decision.

### 1.2 What Is Missing
- **Source Code:** Zero application code exists:
  - No Cargo workspace (`Cargo.toml`).
  - No Rust crates (`crates/anahata-audio`, `crates/anahata-player`, `crates/anahata-library`).
  - No Tauri shell configuration (`src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`).
  - No React frontend (`package.json`, `src/`, UI components).
  - No Android Kotlin plugin (`plugins/tauri-plugin-anahata-android`).
- **Product Requirements Document:** `PRODUCT_REQUIREMENTS.md` was referenced in the brief and development log, but is not present as a separate file. Requirements are inferred from the architectural documents and self-check logs.
- **Platform Strategy & Detailed Guides:** Referenced documents (`PLATFORM_STRATEGY.md`, `SECURITY_AND_RELIABILITY.md`, `ROADMAP.md`, `JULES_IMPLEMENTATION_GUIDE.md`) have not yet been authored.
- **Environment & Build Tooling Checks:**
  - Rust compiler: `rustc 1.94.0`, `cargo 1.94.0` (Verified available).
  - Node.js runtime: `node v22.22.1` (Verified available).
  - Audio backend libraries: Linux ALSA runtime libraries (`libasound2t64`) are installed; `libasound2-dev` development headers are not currently installed, which will be needed when compiling CPAL's ALSA backend on Linux.
  - Tauri CLI: Not globally installed (can be invoked via `cargo tauri` or `npx @tauri-apps/cli` once package.json is initialized).

---

## 2. Architecture Assessment: Practicality and Feasibility

The proposed architecture adopts a strict layered separation:
```
React Frontend (UI only) ──[IPC]──► Tauri Shell ──► anahata-player (Queue / Logic)
                                                          │
                                                    anahata-audio (DSP / CPAL / Symphonia)
```

### 2.1 Strengths
1. **Separation of Concerns:** Isolating `anahata-audio` (DSP/decoding/output) from Tauri and SQLite ensures the engine can be unit-tested headlessly, run in a Linux CLI prototype, and embedded in an Android Kotlin foreground service without WebView dependencies.
2. **Single Playback State Owner:** Running `PlaybackManager` on a dedicated thread with observer dispatch eliminates mutex lock contention across asynchronous UI commands.
3. **Real-Time Audio Safety:** Strict rules prohibiting memory allocations, system calls, I/O, locks, and panic-unwinding inside the CPAL audio callback guarantee glitch-free playback.
4. **Resilient Database Concurrency:** 1 dedicated writer thread + 4 reader connections using SQLite WAL mode ensures scans never block UI browsing.

### 2.2 Critical Feasibility Observations & Recommendations
1. **Real-time Ring Buffer Architecture:**
   - *Design Proposal:* SPSC lock-free ring buffer of `f32` interleaved frames between the decode worker thread and the CPAL callback.
   - *Evaluation:* Excellent choice. Candidates `rtrb` and `ringbuf` are proven, lock-free, and allocation-free once initialized.
2. **Sample Rate Conversion (SRC):**
   - *Design Proposal:* Resampling in the decode worker thread (never in the callback) using `rubato`.
   - *Evaluation:* Feasible. Resampling variable-sample-rate audio files to match the active output device stream rate avoids runtime audio stream re-initialization per track. However, start-of-stream delay compensation and end-of-stream flushing must be tested carefully (Prototype P3) to avoid micro-gaps.
3. **Voice Swapping during Seeks:**
   - *Design Proposal:* Spawning a new Voice with a fresh ring buffer and swapping it in with a 10 ms quick fade (`SwapVoice`).
   - *Evaluation:* Pragmatic. Swapping voices avoids race conditions where the decoder flushes a ring buffer that the audio callback is actively draining.

---

## 3. Documentation Inconsistencies and Gaps

During our audit across the documents, the following discrepancies and gaps were uncovered:

1. **`PRODUCT_REQUIREMENTS.md` Missing:**
   - The brief states to treat `PRODUCT_REQUIREMENTS.md` as the source of truth, but this file is absent from the repository. The scope and features must currently be derived from the architectural drafts.
2. **Crossfade on Manual Skips:**
   - In `DEVELOPMENT_LOG.md` (§Decisions made #4) and `AUDIO_ENGINE.md` (§8.6), manual skips (Next/Previous/Queue jump) use a 100 ms quick fade rather than the configured crossfade. Earlier design discussions proposed an "optional manual crossfade". We confirm that standardizing on a 100 ms quick fade for manual skips is sound and simplifies state transitions.
3. **Folder Monitoring Configuration:**
   - `DATABASE_DESIGN.md` includes a `watch INTEGER DEFAULT 0` column in `library_folders`, while `DEVELOPMENT_LOG.md` notes that `library.monitor_folders` is a global setting and per-folder watching is reserved for post-v1. On Android, folder monitoring is explicitly disabled (rescan on launch/manual only).
4. **Artwork Cache Definition:**
   - The brief mentions "existing local artwork cache". `DEVELOPMENT_LOG.md` (Q4) questions whether this includes folder images (`cover.jpg`/`folder.jpg`). Architectural docs note this is pending owner approval.
5. **SQLite FTS5 Tokenizer:**
   - `DATABASE_DESIGN.md` specifies `unicode61 remove_diacritics 2`. It notes that CJK character segmentation is not natively supported by `unicode61`. A fallback trigram tokenizer is mentioned as unverified for the bundled SQLite build.

---

## 4. Technical Risks and Feasibility Analysis

### 4.1 Audio Engine Risks
| Risk Area | Description | Severity | Mitigation Strategy |
|---|---|---|---|
| **CPAL Linux Dependencies** | Compiling CPAL on Linux requires ALSA development headers (`libasound2-dev` or pkg-config setup). | Medium | Ensure system dev headers are documented/installed or provide mock audio sink for headless CI testing. |
| **Real-time Allocations** | Any heap allocation (`Vec`, `Box`, `String`) inside CPAL's render callback causes audio dropouts/glitches. | High | Enforce preallocated buffers; implement a custom counting allocator in unit tests to assert 0 allocations during callback execution. |
| **Symphonia Gapless & MP3** | Symphonia's gapless trimming is disabled by default and MP3 VBR trim handling has known nuances across revisions. | High | Dedicated Prototype P2 with synthetic sine-wave albums to verify frame-exact sample continuity across transitions. |
| **Crossfade Clipping** | Equal-power crossfade curves on loud, correlated audio can sum to > 1.0 peak amplitude. | Medium | Benchmark output with loud tracks (Prototype P4); implement master gain clamping and evaluate a soft-knee peak limiter. |
| **Device Disconnect Handling** | AAudio/ALSA/WASAPI device disconnections must never be handled inside the audio callback. | High | Stream error callback only dispatches an atomic signal to the player thread; device teardown/rebuild runs asynchronously. |

### 4.2 Android Integration Risks
| Risk Area | Description | Severity | Mitigation Strategy |
|---|---|---|---|
| **WebView Suspension vs Rust Engine** | Android aggressively suspends or destroys WebViews when the app is backgrounded or screen is off. | Critical | Prototype P6: Implement a native Kotlin foreground service (`MediaSessionService` with type `mediaPlayback`) hosting the Rust engine via JNI, completely independent of the Tauri Activity. |
| **Audio Focus & Headphone Disconnect** | Handling incoming calls, notifications (`CAN_DUCK`), and Bluetooth/headphone unplug (`BECOMING_NOISY`). | High | Implement standard Android BroadcastReceiver and AudioManager focus callbacks in Kotlin; dispatch atomic volume/pause commands over JNI to the Rust engine. |
| **Storage Access Framework (SAF)** | Android 10+ scoped storage restricts raw path access. | Medium | Use document-tree URIs; retrieve file descriptors via Android SAF and pass seekable FDs to Symphonia. |

### 4.3 Database and Library Risks
| Risk Area | Description | Severity | Mitigation Strategy |
|---|---|---|---|
| **100,000+ Track Performance** | Unindexed queries or naïve `OFFSET` pagination could cause UI lag during scrolling. | High | Use partial indexes (`WHERE is_visible = 1`); test query plans with `EXPLAIN QUERY PLAN`; virtualize frontend lists. |
| **External Drive Disconnection** | Disconnecting a USB drive must not delete tracks and wipe playlist associations. | High | If folder root is unreachable during scan, mark folder `unavailable` without mutating track statuses; enforce a 30-day grace period for missing files. |

---

## 5. Unresolved Owner Decisions and Blockers

To avoid unauthorized architecture changes, decisions are explicitly grouped by status:

### 5.1 Decisions Already Made (Architect Delegated)
1. **Android Audio Architecture:** Plan A (single Rust engine inside a Kotlin foreground service) with prototype gate P6; fallback is Plan B (Media3/ExoPlayer).
2. **Early Android Spike:** Prototype P6 is scheduled before full frontend feature development.
3. **Format-Gated Gapless:** Gapless playback is only advertised for formats that pass verification tests.
4. **Manual Skip Fade:** Manual skips use a 100 ms quick fade rather than the user-configured crossfade duration.
5. **No Android Folder Monitoring:** Rescan on launch or manual rescan only.
6. **Workspace Structure:** Modular split into `anahata-audio`, `anahata-player`, `anahata-library`, and `src-tauri`.

### 5.2 Decisions Requiring Owner Approval
- **Q1: Online Artwork Lookup Default:** Architect recommends **OFF** by default (preserves local-first privacy, no surprise outbound network requests; user toggles on in Settings).
- **Q2: Shuffle Behavior Setting:** Architect recommends **dropping complex shuffle settings for v1** (always reshuffle on toggle; current track remains at position 0).
- **Q3: Frontend Library Additions:** Owner approval requested for adding `@tanstack/react-query` (paged list caching) and `@tanstack/react-virtual` (list virtualization).
- **Q4: Local Folder Artwork:** Should the scanner check for `cover.jpg` / `folder.jpg` / `front.png` in the album folder as fallback priority 2? Architect recommends **Yes**.
- **Q5: Opus Codec Support in v1:** Keep Opus subject to prototype P8 (`libopus` C adapter cross-compilation), or omit from v1 to avoid C dependencies? Architect recommends **gating on P8**.
- **Q6: Minimum Android Version:** Accept Android 8.0 (API 26) as minimum OS version (required by CPAL AAudio backend)? Architect recommends **Yes**.

### 5.3 Technical Assumptions Requiring Prototypes (P1–P9)
- **P1:** Clean playback/pause/resume of FLAC & MP3 via CPAL without clicks or underruns.
- **P2:** Gapless transition verification across formats with synthetic split-sine waveforms.
- **P3:** Resampling continuity and delay compensation with `rubato`.
- **P4:** Crossfade dynamics, equal-power curve summing, and clipping mitigation.
- **P5:** Seek accuracy and latency on VBR MP3 and FLAC.
- **P6:** Android foreground service, JNI bridge, and background playback durability under screen lock.
- **P7:** Corrupted/truncated audio file resilience (no panics, clean track skip).
- **P8:** Opus build portability across Linux, Windows, macOS, and Android NDK.
- **P9:** Audio output device unplug and hotplug recovery.

---

## 6. First Implementation Milestone: Milestone 1 (M1)

### 6.1 Goal & Scope
The objective of Milestone 1 is to establish a **working, testable audio foundation** without any Tauri, UI, database, or Android overhead.

**In Scope for M1:**
1. Setup Cargo workspace with `crates/anahata-audio`.
2. Symphonia decoding wrapper (`source.rs`) supporting MP3, FLAC, WAV, and OGG Vorbis.
3. Lock-free SPSC ring buffer (`ring.rs`) connecting the decode thread to the audio callback.
4. CPAL output stream integration (`output.rs`) with stereo f32 interleaved output.
5. Basic voice lifecycle (`voice.rs`) and callback mixer (`mixer.rs`).
6. Core transport commands: `play(path)`, `pause()`, `resume()`, `stop()`, `seek(position_ms)`, and switching tracks.
7. Graceful error handling for invalid/corrupted files and missing files.
8. Interactive Linux CLI validation harness (`examples/play_cli.rs`).

**Out of Scope for M1:**
- React frontend, Vite, Tailwind, shadcn/ui.
- Tauri IPC commands and window management.
- SQLite database, migrations, and library scanner.
- Android plugins, Kotlin code, and JNI.
- Complex crossfade and gapless joining (reserved for M2/P2/P4 once single-track playback is rock-solid).

---

## 7. Testing and Verification Plan for Milestone 1

1. **Automated Unit Tests (`cargo test -p anahata-audio`):**
   - **Decode Test:** Probe and decode headers/packets from synthetic or test audio fixtures.
   - **Ring Buffer Test:** Verify zero-loss push and pop across thread boundaries with SPSC queues.
   - **Corrupt File Test:** Pass truncated, zero-byte, and garbage files; verify that decoding returns `AudioError::DecodeFailed` or `UnsupportedFormat` and does not panic.
   - **Real-Time Callback Allocation Guard:** In test configurations, run the callback mixer render loop through a counting allocator to assert zero heap allocations.
2. **Interactive CLI Verification (`cargo run --example play_cli <path-to-file>`):**
   - Commands to test interactively:
     - `p` -> Pause / Resume toggle (verifying smooth gain ramp without clicks).
     - `s <seconds>` -> Seek to timestamp (verifying audio resumes promptly without artifacts).
     - `n <path>` -> Switch to a new track.
     - `v <0.0 - 1.0>` -> Adjust master volume.
     - `q` -> Stop and quit.

---

## 8. Expected File Changes for Milestone 1

The following files will be created in M1:

```
anahata/
├── Cargo.toml                              # Workspace manifest
├── crates/
│   └── anahata-audio/
│       ├── Cargo.toml                      # Dependencies: symphonia, cpal, rtrb/ringbuf, thiserror
│       ├── src/
│       │   ├── lib.rs                      # Public exports
│       │   ├── error.rs                    # AudioError enum
│       │   ├── source.rs                   # Symphonia decoder & seeker wrapper
│       │   ├── channels.rs                 # Stereo channel mapping & downmixing
│       │   ├── ring.rs                     # SPSC audio ring buffer abstraction
│       │   ├── voice.rs                    # Decode worker thread & voice handle
│       │   ├── mixer.rs                    # Real-time audio render callback & gain ramps
│       │   ├── output.rs                   # CPAL device setup and stream runner
│       │   └── engine.rs                   # High-level AudioEngine facade
│       ├── examples/
│       │   └── play_cli.rs                 # Terminal audio playback harness
│       └── tests/
│           ├── decode_tests.rs
│           └── ring_tests.rs
```

No existing architectural documentation files will be overwritten or altered.

---

## 9. Objective Acceptance Criteria for Milestone 1

Milestone 1 will be considered complete when all of the following conditions are met:

1. `cargo check --workspace` and `cargo test --workspace` pass with zero compiler warnings and zero test failures.
2. The CLI harness (`play_cli`) successfully opens and decodes at least MP3, FLAC, and WAV files.
3. Audio plays smoothly through the default audio output device.
4. Transport controls (pause, resume, seek, stop, track change) execute reliably without deadlocks, panics, or audio glitches.
5. Attempting to play an unreadable or corrupt file yields a descriptive `AudioError` and does not crash the application.
6. The audio callback loop performs no heap allocations or blocking synchronization during active playback.

---

*This concludes the initial repository audit and implementation roadmap. We await owner review and approval on Section 5.2 before beginning code implementation.*
