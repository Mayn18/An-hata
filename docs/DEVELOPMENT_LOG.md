# DEVELOPMENT_LOG

## Status (2026-11-01)

| Stage | State |
|---|---|
| 1. Requirements review & questions | Done |
| 2. Technology & audio feasibility research | Done (first pass; Tauri security config, resampler, Windows/macOS, Media3 fallback, Linux packaging still open) |
| 3. Architecture & database | **Draft written** — awaiting owner review |
| 4. Platform integration | Not started |
| 5. Roadmap & Jules contract | Not started |
| 6. Review for contradictions | Not started (a self-check pass was done on Stage 3 docs, see below) |

Documents present: ARCHITECTURE, DATABASE_DESIGN, AUDIO_ENGINE, TECHNICAL_DECISIONS, KNOWN_LIMITATIONS, DEVELOPMENT_LOG.
Still to write: PLATFORM_STRATEGY, SECURITY_AND_RELIABILITY (expanded), ROADMAP, JULES_IMPLEMENTATION_GUIDE.

## Decisions made (architect, delegated by owner on 2026-11-01)
1. Android audio: plan A (single Rust engine in a Kotlin foreground service) behind prototype gate P6; fallback plan B (Media3/ExoPlayer).
2. Early Android spike (P6) is scheduled before heavy feature work.
3. Gapless is promised only for formats whose prototype passes; others documented.
4. Crossfade applies to natural transitions; manual skips use a 100 ms quick fade. (Earlier chat text mentioned an "optional manual crossfade" — withdrawn for v1.)
5. Android: no folder monitoring; rescan on launch/manual.
6. Workspace split into three Rust crates + thin Tauri shell.

## Decisions awaiting owner
| # | Question | My recommendation |
|---|---|---|
| Q1 | **Online artwork lookup default**: ON or OFF at first launch? (Requirements say "optional"; do not specify default.) | **OFF** — fits local-first/no-surprise-network; one toggle in Settings enables it |
| Q2 | **"Shuffle behavior" setting** is named in the requirements but not defined. Proposal: a single option "Shuffle: reshuffle each time shuffle is turned on / keep the same order until the queue changes" — or drop the setting and always reshuffle | Drop the setting for v1 (always reshuffle on enable; current track stays first). Fewer options, matches "keep settings straightforward" |
| Q3 | Approve proposed additions C1–C3 (TanStack Query, TanStack Virtual, type generation)? | Approve C1 and C2; C3 pending Jules' verification |
| Q4 | **"Existing local artwork cache"** (requirements §7): does it mean only our own cache, or also `cover.jpg`/`folder.jpg` files next to the music? | Include folder images (`cover/folder/front.jpg|png`) read-only as priority 2 — common expectation, tiny cost |
| Q5 | Opus: keep in v1 subject to P8, or drop it now to simplify? | Keep, gated by P8 |
| Q6 | Minimum Android version 8.0 (API 26) acceptable (forced by CPAL's AAudio backend)? | Yes |

## Self-check findings (Stage 3 drafts)
- Schema supports every command in ARCHITECTURE §6.2 (queue persistence, playlists, settings, scan history, artwork lookups). Added beyond the brief's table list: `queue_items`, `track_genres`, `artwork_lookup`.
- "Detect moved files" uses fingerprints; "avoid duplicates" uses `duplicate_of`; both are consistent with `is_visible` partial indexes.
- Potential inconsistency fixed: `library.monitor_folders` setting vs per-folder `watch` column → setting is the global toggle; per-folder column is reserved for future per-folder control (unused in v1).
- Known weak spot: the engine/mixer design assumes one resample rate per output stream; changing device mid-playback rebuilds voices (a short interruption is accepted).
- Risk concentration: P6 (Android) and P2 (gapless) — if either fails, ROADMAP ordering and the plan-B fallback change.
