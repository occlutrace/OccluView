# ADR 01: Dental texture channel correction

Date: 2026-07-25
Status: accepted

## Context

HPS raw textures are DirectX surfaces (`D3DFMT_A8R8G8B8` / `D3DFMT_R8G8B8`) whose
little-endian memory order is `B,G,R,A`. Decoding as `R,G,B,A` turns warm dental
whites into blue. The default raw layout in `texture.rs` correctly maps to
`BGR`/`BGRA`.

Some exporter-authored HPS JPEG atlases also carry swapped chroma (Cb/Cr
transposed before compression). A standards-compliant JPEG decode of a
mis-authored file still comes out blue. The raw-path fix does not cover this
case.

## Decision

- Keep `default_raw_layout` (`BGR`/`BGRA`) for format-less raw textures.
- Apply a whole-texture heuristic **after** decoding: if the sampled texture is
  implausibly blue-biased, swap R/B per pixel.
- Sampling: stride to ~4096 pixels, skip `alpha < 8` and near-gray (`|B-R| < 16`),
  require mean blue > red + margin (`max(red/4, 24)`) **and** 90% of sampled
  pixels individually blue-biased. Calibration: swapped atlas measured
  mean R107/B150 uniformly; corrected R150/B107.
- Log via `tracing::debug` when correction fires; metrics via caller if needed.

## Consequences

- Uniformly blue scans from a single exporter are fixed. Real blue materials
  (anti-glare spray, silicone) cover only part of the scan and do not trip the
  90% per-pixel threshold.
- Legitimately blue whole-texture scans (future exporter) would be mis-corrected;
  gate behind exporter metadata if such a case appears. Corpus tests cover known
  vendors.

## Alternatives considered

- Exporter/version metadata routing: ideal but not available in current HPS
  schema without version trail.
- User-facing toggle: rejected for v1; can be added as a compatibility mode
  if needed.
