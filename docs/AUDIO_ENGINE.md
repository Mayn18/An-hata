# AUDIO_ENGINE

Status: **Stage 3 draft** (2026-11-01). Labels: [Verified] [Limitation] [Assumption] [Unverified] [Recommendation].
Crates: `anahata-audio` (engine) and `anahata-player` (manager). Neither depends on Tauri or SQLite.

---

## 1. Goals and non-goals

Goals: reliable playback, pause/resume, accurate seeking, gapless where the format allows, configurable crossfade, queue transitions, device-change recovery, no glitches, corrupt files never crash the app, runnable without a WebView (Android service).
Non-goals (v1): equalizer, effects, ReplayGain, exclusive/bit-perfect output, volume normalisation, multi-room, streaming.

## 2. What the libraries provide, and what we must build

| Need | Symphonia | CPAL | We build |
|---|---|---|---|
| Container demux + codec decode | **Yes** (MP3, FLAC, WAV, AIFF, Vorbis, ALAC, AAC-LC; Opus not ready) | – | Wrapper (`source.rs`) |
| Gapless trim data (delay/padding) | Yes for some formats (see §7) | – | Joining tracks seamlessly |
| Seeking | Yes (`FormatReader::seek`) | – | Seek orchestration |
| Output to hardware | – | **Yes** (callback model; AAudio on Android) | Device selection, recovery |
| Sample-rate conversion | **No** | No | Resampler |
| Channel mapping/downmix | No | No | `channels.rs` |
| Buffering between decode and output | – | – | Ring buffer |
| Mixing, gain ramps, crossfade | No | No | `mixer.rs` |
| Queue, shuffle, repeat | – | – | `anahata-player` |

**[Verified]** CPAL 0.18.x: AAudio on Android (API 26+), ALSA/WASAPI/CoreAudio on desktop, optional PipeWire/PulseAudio/JACK; stream callbacks run on a dedicated audio thread. **[Verified]** Symphonia README status table and gapless flags (see `KNOWN_LIMITATIONS.md`).
**Conclusion:** Symphonia + CPAL **can** provide everything *together with* about 1,500–2,500 lines of our own code (resample, mix, buffer, transition logic). No GStreamer/Rodio needed. Rodio would not help: it hides exactly the control (sample-accurate joins, crossfade) we need.

## 3. Data path

```
 file ──► Source (Symphonia) ──► trim (gapless) ──► channel map ──► resample ──► [Ring buffer SPSC f32]
            decode worker thread (per voice)                                          │
                                                                                      ▼
                                CPAL output callback (real-time):  Mixer ── master gain ── clamp ──► device
```

- **Internal format:** interleaved `f32`, output channel count (stereo in v1: mono is duplicated, >2 channels are downmixed with standard ITU coefficients), **device sample rate** (resampled in the decode worker; the callback never resamples).
- If the device only offers integer formats, conversion to `i16/i32` happens in the callback using a **preallocated** scratch buffer.
- **Ring buffer:** SPSC lock-free queue of `f32`, capacity ≈ 1.5 s of audio per voice. Decode worker keeps it topped up (target fill 0.5–1.0 s); on an empty ring the callback outputs silence and increments an underrun counter (never blocks).
  **[Unverified]** which ring-buffer crate (candidates: `rtrb`, `ringbuf`) — Jules verifies existence/maintenance/licence before use.
- **Resampler:** **[Unverified]** candidate `rubato` (high quality, offline-style API); must handle start-of-stream delay compensation and end-of-stream flush so no samples are lost or added (prototype P3). Skip entirely when file rate == device rate (common case: 44.1/48 kHz).

## 4. Voices

A **Voice** = one track being played or preloaded:
```
Voice { id, ring consumer (callback side), shared atomics {
    frames_played: AtomicU64, eof: AtomicBool, failed: AtomicBool, total_frames: Option<u64>,
    prime_ready: AtomicBool }, decode-worker handle }
```
- At most **two** voices exist: *current* and *next* (preloaded). A third exists only for ~100 ms during a quick-fade skip.
- **Preload policy:** start preloading the next track when remaining time of current ≤ `max(crossfade_seconds + 3 s, 8 s)`, or immediately after a queue edit changes "next". Preload opens the file, decodes until the ring holds a **prime** (≥ 250 ms), then idles. If the file is unavailable, the manager is told (`PreloadFailed`) and picks the following track (marking the bad one as errored).
- **Dropping a voice** must not free memory in the callback: the callback moves retired voices into a "graveyard" SPSC queue that the player thread empties.

## 5. Control path (player → engine) and event path (engine → player)

- **Commands** go through a **bounded lock-free SPSC queue** polled at the start of every callback:
  `Play(voice)`, `SetNext(voice | none)`, `Pause`, `Resume`, `SetVolume(f32)`, `SetTransition(Gapless | Crossfade{frames})`, `Stop`, `SwapVoice(voice)` (used by seek), `QuickFadeTo(voice)`.
- **Events** from the callback (`TrackStarted(voice)`, `TrackEnded(voice)`, `Underrun(n)`, `StreamError`) go through another SPSC queue read by the player thread each tick (~20 ms). The callback never uses channels with locks, never logs, never allocates.
- **Position:** each voice's `frames_played` (atomic) is the truth; the manager derives `position_ms = frames_played * 1000 / device_rate + seek_base`. Reported to the frontend as an anchor (see `ARCHITECTURE.md` §5).

## 6. Mixer state machine (inside the callback)

States: `Idle`, `Playing{cur}`, `Crossfading{cur→next, progress, len}`, `Pausing`, `Paused`, `QuickFading`.

Per callback:
1. Drain ≤ N pending commands.
2. Depending on state, pull `out.len()` samples from the current voice ring (and the next, if crossfading), apply per-sample gain from the active ramp, sum.
3. Apply master volume (smoothed over ~10 ms to avoid zipper noise; UI slider 0–1 maps to amplitude with a squared curve — **[Recommendation]**, tune by ear).
4. Clamp to [−1, 1] (soft limiter considered if prototype P4 shows clipping).
5. Update atomics; push events.

**Pause/resume:** gain ramps to 0 over ~15 ms, then the callback outputs silence; after ~2 s paused the stream itself is paused (saves power, especially on Android). Resume restarts the stream and ramps up. **[Unverified]** resume latency of CPAL `Stream::play()` on each platform → measure (P1/P6).

**Underrun:** silence + counter. If underruns persist (> N in 5 s) the manager shows "buffering" and may raise buffer targets.

## 7. Gapless playback

**What it is:** removing *encoder-added* silence (delay at start, padding at end) and joining the last sample of track A to the first sample of track B with no gap.
**What we do:**
1. Open Symphonia readers with gapless **enabled** (default is off). **[Verified]** when enabled, packet timestamps are relative to the end of encoder delay and packets carry trim counts that the decoder applies.
2. Preload B so its ring is primed *before* A's ring drains.
3. In the callback, when A's ring is empty **and** `A.eof` is set, continue filling the *same output buffer* from B — a **sample-accurate join inside one callback**, no stream restart, no device reopen. **[Verified]** (Symphonia maintainer guidance) device must not be closed/reopened between tracks, and joining is the application's job.
4. If B isn't primed in time: insert nothing artificial, play whatever is available, and count a `gap_violation` metric for tests/logging.

**Per-format support** (from Symphonia's own status table plus third-party reports; to be confirmed by prototype P2):

| Format | Gapless via Symphonia | Notes |
|---|---|---|
| FLAC, WAV, AIFF, Vorbis | **Yes** (flagged gapless) | Expected sample-exact |
| MP3 | Flagged gapless | **[Unverified]** a third-party project reports the 0.6 alpha MP3 demuxer reads the LAME tag but doesn't fill per-packet trims; test with current release. Fallback: apply `delay`/`padding` ourselves from track parameters |
| M4A AAC / ALAC | **No** (ISO/MP4 demuxer not flagged gapless) | Own trimming from iTunSMPB / edit list = stretch goal; otherwise document "not gapless" |
| Opus | Decoder not ready in Symphonia | Needs `libopus` adapter; pre-skip handling required |

Rule: **never advertise gapless for a format until its prototype test passes.** The UI says nothing about gapless per file; limits live in docs.

## 8. Crossfade

**Different mechanism from gapless:** both tracks play *simultaneously* for `D` seconds with opposing gain curves. When crossfade is enabled it replaces gapless joins.

1. Manager sets `SetTransition(Crossfade{frames = D × rate})` on the current voice when the next track is known.
2. The callback starts the fade when `frames_played ≥ total_frames − D_frames` (it compares atomics every callback; cheap and sample-accurate).
3. **Equal-power curves** (`cos`/`sin`) are the default; sum can exceed 1.0 for correlated material → final clamp, and prototype P4 decides whether a soft limiter is needed ("avoid clipping" requirement).
4. `D` is clamped to `min(D, len(A)/2, len(B)/2)`. If `total_frames` is unknown for A (some streams/VBR files), **no crossfade for that boundary** (falls back to gapless/plain join) rather than guessing.
5. Logical "current track" switches to B at the crossfade start (UI shows B immediately); A is the *outgoing voice*.
6. **Manual skip (Next/Previous/jump)** uses a **100 ms quick fade**, never the long crossfade. (*Change from the earlier note of an "optional manual crossfade": deferred out of v1 to keep behaviour predictable and settings minimal.*)
7. **Seek during a crossfade** → drop the outgoing voice (10 ms ramp), apply the seek to the incoming track.
8. **Repeat One:** crossfade/gapless apply to the *same track* restarting (needs a second voice of the same file — works because voices are independent). **Repeat All at end of queue:** next = first item. **Shuffle:** "next" is whatever the shuffle order says; if the user edits the queue mid-fade, the change takes effect after the fade.
9. Tracks shorter than ~2×D crossfade with a reduced D; very short tracks (<1 s) skip crossfade.

## 9. Seeking, errors, devices

**Seek:** `Source::seek(time)` (**[Assumption]** accurate seeking available for all supported formats; MP3 VBR without a TOC may be slow/imprecise → prototype P5) followed by decoder reset (**[Unverified]** exact API: verify the decoder-reset call after seek in the pinned Symphonia version). To avoid races between decode thread and callback, **seek creates a fresh Voice** (new ring) positioned at the target and swaps it in with a 10 ms fade (`SwapVoice`); the old voice goes to the graveyard. Target: seek-to-sound ≤ 150 ms for local files.

**Corrupt/unsupported files:**
- Packet decode errors: skip the packet; after K (≈ 50) consecutive failures, fail the track (`DecodeFailed`).
- Probe/open failure: `UnsupportedFormat`/`FileUnavailable`.
- Panics in the decode worker are caught (`catch_unwind`) → `failed` flag → manager skips to next, shows a non-modal error. After 3 consecutive failed tracks the manager pauses and reports (prevents infinite skip loops).
- Truncated file: treat end-of-stream early as normal EOF (log warning).

**Device changes/failures:**
- CPAL reports stream errors through the error callback. **[Verified for Android/AAudio]** on disconnect, do **not** stop/close/reopen inside the callback; signal another thread. Same rule adopted on all platforms.
- Manager reaction: `Pause` (position saved) → try to reopen the default device → on success rebuild voices at the saved position at the new sample rate and resume only if previously playing and the cause was a *default-device switch*. Headphones unplugged → **stay paused** (expected Android/desktop behaviour).
- No device at all: status `Error{DeviceUnavailable}`; retry on `device:changed` or user action.

## 10. Android

- **[Recommendation – plan A]** same engine, hosted by `PlayerRuntime`, a process-wide singleton created from native code, independent of the Activity/WebView. Kotlin `MediaSessionService` (foreground type **mediaPlayback**, **[Verified]** required on Android 14+ with `FOREGROUND_SERVICE_MEDIA_PLAYBACK`) owns notification, MediaSession, audio focus, becoming-noisy handling and media buttons; it sends `PlayerCommand`s through JNI and receives `PlayerEvent`s through a registered callback (Rust attaches a thread to the JVM to call Kotlin).
- Audio focus: loss → Pause (transient loss) or Pause+abandon (permanent); `CAN_DUCK` → temporary volume reduction through the same master-gain ramp; regain → resume only if we were playing before the transient loss.
- **[Unverified, gate]** (P6) engine keeps playing with Activity destroyed and screen off; CPAL AAudio stream usage attributes (music/media, not "call") and volume-slider behaviour; stream recovery after headphone unplug.
- **Fallback plan B** if P6 fails: Android plays via Media3/ExoPlayer in Kotlin driven by the same queue from Rust (two audio code paths; gapless/crossfade then follow Media3's capabilities — to be researched only if needed).

## 11. Real-time rules (checklist for code review)
Callback code: no `Vec::push`/`Box::new`/`String`, no `Mutex`/`RwLock`, no channels with locks, no I/O/logging, no `unwrap`/panicking indexing, no `Drop` of large structures. All buffers preallocated in `Engine::new`. Tests include a "callback-allocation guard" (a counting allocator in test builds asserting zero allocations during `render`).

## 12. Prototype plan (gates before building features on top)

| ID | Prototype (Linux CLI `examples/play_cli.rs` unless stated) | Passes when |
|---|---|---|
| **P1** | Play/pause/resume/stop FLAC + MP3 through CPAL | No clicks, no underruns over 10 min; pause→resume < 100 ms |
| **P2** | Gapless matrix: generated album (continuous sine split into tracks; sample-count known) per format | Output stream is bit-/sample-continuous across boundary (tolerance defined per format); report per format |
| **P3** | Resampling: 44.1→48 k and 96→48 k across a gapless boundary | No gap/overlap; total frames within ±1 of expected |
| **P4** | Crossfade between two tracks of different sample rates; loud correlated material | Smooth curve (analysed offline from captured output); no clipping beyond tolerance |
| **P5** | Seek stress: 1,000 random seeks per format | No panics; position error ≤ 50 ms (≤ 100 ms VBR MP3); latency ≤ 150 ms |
| **P6** | **Android spike:** foreground service + engine + JNI; Activity destroyed; screen locked | Audio continues 30 min; controls work from notification; unplug headphones pauses |
| **P7** | Corrupt files: truncated/garbage/zero-byte/huge-tag samples | No crash; skip with error; no infinite loop |
| **P8** | Opus build check: `libopus` adapter on Linux, Windows, macOS, Android NDK | Builds and decodes on all four, or Opus declared unsupported in 1.0 |
| **P9** | Device loss: unplug USB/BT device mid-play on Linux | Recovers per §9 without crash |
