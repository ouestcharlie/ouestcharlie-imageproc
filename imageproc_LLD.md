# ouestcharlie-imageproc — Low-Level Design

## Overview

`ouestcharlie-imageproc` is a standalone Python package that bundles the `image-proc` Rust CLI and a thin Python subprocess wrapper. It handles all pixel-level operations for OuEstCharlie: decoding, orientation, resize, fit, and encoding.

It has no Python package dependencies — only the standard library. Higher-level builders (`thumbnail_builder`, `preview_builder`) live in `ouestcharlie-py-toolkit` and depend on this package.

## Package Structure

```
ouestcharlie-imageproc/
├── pyproject.toml                  # Build config; triggers hatch_build.py
├── hatch_build.py                  # Hatch hook: compiles image-proc and bundles binary into wheel
├── image-proc/                     # Rust CLI source
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── build.rs
│   └── src/main.rs
├── src/ouestcharlie_imageproc/
│   ├── __init__.py                 # Exports OneTimeImageProc, PersistentImageProc, IMAGE_PROC_PROTOCOL_MAJOR_VERSION
│   ├── image_proc.py               # Binary discovery + subprocess wrappers
│   └── bin/                        # Bundled image-proc binary (populated at build time)
├── tests/
│   ├── test_image_proc.py          # Unit tests (mocked subprocess)
│   └── sample-images/             # Test fixtures
└── tests_integration/
    └── test_image_proc_integration.py  # Integration tests (real binary required)
```

## Protocol — Persistent Newline-Delimited JSON

`image-proc` runs as a coprocessor: it reads one JSON request per line from stdin and writes one JSON response per line to stdout.

- Every request includes `"protocol_version": <major>` (integer). image-proc checks that `major == CARGO_PKG_VERSION_MAJOR`; a mismatch returns `{"error": "unsupported protocol version X, expected Y"}` without exiting.
- The constant `IMAGE_PROC_PROTOCOL_MAJOR_VERSION` in `image_proc.py` must equal the major component of `image-proc/Cargo.toml` `version`. Bump both together on any breaking protocol change (see [CLAUDE.md](image-proc/README.md)).
- Errors are returned in-band as `{"error": "…"}` — the process does not exit on error.
- The process exits when stdin is closed.

## Python Wrappers

Two classes in `image_proc.py` implement the protocol:

| Class | Strategy | Use case |
|---|---|---|
| `OneTimeImageProc` | Fresh process per `request()` call; uses `communicate()` | Batch workloads already parallelised at a higher level (e.g. AVIF grid assembly) |
| `PersistentImageProc` | One process kept alive with `asyncio.Lock` | Session-scoped HTTP server (e.g. Wally's preview middleware) |

`PersistentImageProc` restarts the process automatically on crash. Both expose the same interface:

```python
result: dict = await proc.request(payload_dict)
```

`PersistentImageProc` additionally implements `async def close()` and the async context manager protocol.

## `avif_grid` Command — Thumbnail AVIF Grid

Triggered by presence of `"photos"` array. Used by `thumbnail_builder.generate_partition_thumbnails()` via `OneTimeImageProc`.

**Request:**
```json
{
  "protocol_version": 2,
  "photos": [
    { "path": "/tmp/staged.jpg", "ext": ".jpg", "orientation": 6, "content_hash": "Kf3QzA2_nBcR8xYvLm1P9w" }
  ],
  "tile_size": 256,
  "fit": "crop",
  "quality": 55,
  "output": "/tmp/output.avif"
}
```

- `fit` — `"crop"` (center-crop to square) or `"pad"` (letterbox with black).
- Photos must be pre-sorted by `content_hash` for stable tile indices.

**Response:**
```json
{ "rows": 8, "tileSize": 256, "photoOrder": ["Kf3QzA2_nBcR8xYvLm1P9w", ...] }
```

**Rust pipeline (per chunk):**
```
rayon::par_iter  — decode → apply EXIF orientation → resize → fit to square
sequential       — YUV420 conversion → AVIF grid encoding (libavif, not thread-safe)
```

Grid layout: `cols = min(8, n)`, `rows = ceil(n / cols)` — always 8 columns (or fewer if n < 8), max 8×8 for 64 photos. Last row padded with black tiles. `cols` is not included in the response — callers compute it as `min(8, len(photoOrder))`.

## `jpeg_preview` Command — On-Demand Preview JPEG

Triggered by presence of `"photo"` object. Used by `preview_builder.generate_preview_jpeg()` via `PersistentImageProc`.

**Request:**
```json
{
  "protocol_version": 2,
  "photo": { "path": "/tmp/staged.cr2", "ext": ".cr2", "orientation": 1, "content_hash": "Kf3QzA2_nBcR8xYvLm1P9w" },
  "max_long_edge": 1440,
  "quality": 85,
  "output": "/tmp/preview.jpg"
}
```

**Response:**
```json
{ "width": 1440, "height": 960 }
```

## Format Support and Platform Matrix

| Format | Cargo feature | System dependency | Linux | macOS | Windows |
|--------|--------------|-----------|:-----:|:-----:|:-------:|
| JPEG, PNG, WebP, TIFF | *(default)* | None (pure Rust) | ✅ | ✅ | ✅ |
| RAW (CR2, NEF, ARW, DNG, RAF, ORF, RW2, PEF) | `raw` | None (pure Rust) | ✅ | ✅ | ✅ |
| HEIC/HEIF | `heic` | `libheif ≥ 1.18` — build-time only, bundled into the wheel at runtime | ✅ | ✅ | ⚠️ (vcpkg build, see below) |

RAW and HEIC are compile-time features; the binary returns a clear error if a format
is not compiled in. Both are Cargo-level opt-in (`cargo build --release` alone
excludes both), but **published wheels enable both by default** — CI sets both env
vars and bundles `libheif`'s shared library into the wheel, so end users need no
system `libheif` install, unlike a from-source build:

```bash
IMAGE_PROC_FEATURE_RAW=1 IMAGE_PROC_FEATURE_HEIC=1 hatch build
```

⚠️ **Windows HEIC caveat**: `libheif` isn't available via `choco`, so CI bootstraps
`libheif` via `vcpkg` (see `.github/workflows/_build.yml`) — this pulls in
`libde265`/`x265`/`aom` as transitive deps and is slower/less proven than the
Homebrew/apt paths. If it proves too unreliable in practice, the fallback is dropping
`IMAGE_PROC_FEATURE_HEIC` from the Windows CI leg only, as a follow-up.

## Build

The `hatch_build.py` hook compiles the Rust binary and bundles it into the wheel. On
Windows, when the `heic` feature is built, it also copies `libheif`'s runtime DLLs
(located via `VCPKG_ROOT`/`VCPKG_INSTALLATION_ROOT`) next to `image-proc.exe`, since
Windows has no `auditwheel`/`delocate` equivalent to vendor them into the wheel
automatically:

```bash
# Development (editable install — creates symlink, picks up subsequent cargo builds)
pip install -e . --no-build-isolation

# Release wheel (copies binary)
hatch build --target wheel
```

System dependencies required at build time:
- **macOS:** `brew install nasm inih libheif`
- **Linux:** `apt-get install nasm libheif-dev`
- **Windows:** `choco install nasm`; `libheif` via `vcpkg` (see CI workflow)

## Version Bumping

When the JSON protocol changes in a breaking way, bump both:
1. `image-proc/Cargo.toml` → `version = "X.Y.Z"` (bump major)
2. `src/ouestcharlie_imageproc/image_proc.py` → `IMAGE_PROC_PROTOCOL_MAJOR_VERSION = X`

The major component must match — image-proc validates `protocol_version` in every incoming request.
