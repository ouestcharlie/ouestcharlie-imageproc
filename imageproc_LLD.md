# ouestcharlie-imageproc — Low-Level Design

## Overview

`ouestcharlie-imageproc` is a standalone Python package that bundles the `image-proc` Rust CLI and a thin Python subprocess wrapper. It handles all pixel-level operations for OuEstCharlie: decoding, orientation, resize, fit, and encoding.

It has no Python package dependencies — only the standard library. Higher-level builders (`thumbnail_builder`, `preview_builder`) live in `ouestcharlie-py-toolkit` and depend on this package.

## Package Structure

Three layers, each in its own top-level directory:

- **Rust CLI** (`image-proc/`): a library crate wrapped by a thin binary. The binary
  only runs the stdin/stdout loop; everything else is in the library so it can be
  tested without a subprocess. The library is split by concern: protocol types,
  one module per command, format decoders, and pixel transforms (orientation,
  resize, fit) shared by the commands. Each optional format (`raw`, `heic`) is one
  decoder module compiled in by its Cargo feature, with a stub returning a
  "rebuild with --features …" error when the feature is off; adding a format means
  adding a decoder module and its extensions to the dispatch.
- **Python wrapper** (`src/ouestcharlie_imageproc/`): binary discovery and the
  subprocess wrappers below. The wheel bundles the compiled binary (and, for HEIC,
  libheif) next to it; the build hook compiles it.
- **Tests**: Rust unit tests sit next to the code they test; Rust black-box tests
  (protocol handling, decoding the shared sample images) use the library's public
  API. Python tests are split into mocked unit tests and integration tests that
  spawn the real binary.

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
  "protocol_version": 3,
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
  "protocol_version": 3,
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
| JPEG, PNG, WebP, TIFF, GIF, BMP | *(default)* | None (pure Rust) | ✅ | ✅ | ✅ |
| JPEG XL | *(default)* | None (pure Rust, jxl-oxide) | ✅ | ✅ | ✅ |
| RAW (see "RAW decoding" for the list) | `raw` | None (pure Rust) | ✅ | ✅ | ✅ |
| HEIC/HEIF (`.heic`, `.heif`, `.hif`), AVIF | `heic` | `libheif ≥ 1.18` with an HEVC decoder and an AV1 decoder (aom or dav1d) — build-time only, bundled into the wheel at runtime | ✅ | ✅ | ✅ (vcpkg build, see below) |

The decoder is chosen from the file extension; `image-proc --formats` prints the
extensions the build decodes (a JSON array, depending on the compiled features), and
the Python wrapper exposes it as `supported_extensions()`. For the `image`-crate
formats the content decides, so a mislabelled file still decodes. Multi-page TIFF
and animated WebP/GIF give their first image; 16-bit and CMYK TIFF are
converted to 8-bit RGB.

**Orientation.** HEIF-family (HEIC, AVIF) and JPEG XL decoders already apply the
file's own rotation, so image-proc ignores the request's `orientation` for these
extensions: applying it again would rotate twice. This matches the HLD "upright
convention", where such photos are stored with orientation 1. For every other
format the request's EXIF `orientation` is applied.

**Known gaps:** 32-bit float TIFF (not supported by the `image` crate); ICC profiles are ignored, so wide-gamut files look slightly
desaturated in thumbnails and previews.

RAW and HEIC are compile-time features; the binary returns a clear error if a format
is not compiled in. JPEG XL is always built in: it is pure Rust with no system
dependency, so an opt-in feature would add CI wiring for nothing. Both are Cargo-level opt-in (`cargo build --release` alone
excludes both), but **published wheels enable both by default** — CI sets both env
vars and bundles `libheif`'s shared library into the wheel, so end users need no
system `libheif` install, unlike a from-source build:

```bash
IMAGE_PROC_FEATURE_RAW=1 IMAGE_PROC_FEATURE_HEIC=1 hatch build
```

### RAW decoding (since 3.0.0)

RAW files are decoded with `rawler` (the dnglab library, pinned `=0.7.2`). Both
commands pass the size they need to the decoder: `tile_size` for `avif_grid`,
`max_long_edge` for `jpeg_preview`.

1. **Embedded JPEG first.** If the full-size JPEG the camera stored in the file has a
   long edge of at least the requested size, use it. Fast (under 0.15 s), and colours
   match the camera's own rendering.
2. **Develop otherwise.** No embedded JPEG (ORF, most phone DNGs) or one too small
   (PEF: 720 px) → demosaic with rawler's default pipeline, then `auto_expose`: a
   Reinhard exposure to a mid-grey key, gain capped at +2 EV, with highlight
   roll-off so input white stays white. Without it, developed images are 1–2 stops
   darker than the camera JPEG. 0.3–1 s per file.

Neither path rotates the image: the EXIF `orientation` sent in the request applies,
as for other formats. Decoder panics are caught and returned as an error for that
photo.

Embedded JPEG sizes measured on raw.pixls.us samples:

| Format | Embedded JPEG | Thumbnail (256) | Preview (1440) |
|---|---|---|---|
| CR2 (EOS 6D) | 5472×3648 | embedded | embedded |
| CR3 (EOS R6) | 3408×2272 | embedded | embedded |
| NEF (Z 6), ARW (A7 III) | ~1620×1080 | embedded | embedded |
| RAF (X-T2), RW2 (G90) | 1920 px | embedded | embedded |
| NRW (Coolpix P1000) | full size | embedded | embedded |
| RWL (Leica C, C-Lux, D-Lux 109) | 1920 px | embedded | embedded |
| PEF (K-1 II) | 720×480 | embedded | developed |
| FFF (H5D-40) | 1217×913 | embedded | developed |
| SR2 (DSC-R1) | 640×424 | embedded | developed |
| 3FR (H3D) | 320×240 | developed | developed |
| ORF (E-M1 II), DNG (iPhone, Pixel), SRW, CRW, SRF, ERF, IIQ, MOS | none | developed | developed |

Medium-format files (3FR, FFF, IIQ, MOS: 22–50 MP) take 0.7–1.6 s to develop; the
others stay under 1 s.

**Supported extensions:** CR2, CR3, CRW, NEF, NRW, ARW, SRF, SR2, DNG, RAF, ORF,
RW2, RWL, PEF, SRW, ERF, 3FR, FFF, IIQ, MOS. Support also depends on the camera
model: rawler keeps a per-model list, and a model it does not know fails with
"Unknown camera" even in a supported format (seen with the Leica D-Lux 6 RWL, the
Phase One P40+ IIQ, and the Galaxy S10 DNG). Such files get no thumbnail; a newer
rawler release may add them.

**Set aside** (not listed in `--formats`, so not decoded):

| Extension | Reason |
|---|---|
| MRW (Minolta) | rawler knows few Minolta models: 1 of 4 samples decoded |
| X3F (Sigma) | rawler cannot read the file layout ("Couldn't find tag Make") |
| KDC, DCR (Kodak) | decode, but with strong colour casts (wrong camera colour data) |
| ARI (ARRI) | cinema camera frames in a log encoding, not photos |
| K25, DCS (Kodak), MEF (Mamiya), BAY (Casio) | no CC0 sample on raw.pixls.us to test with |
| RAW | ambiguous: Panasonic and Contax, but also unrelated software |
| PRO | listed by RapidRAW as Apple ProRAW, but ProRAW files are `.dng`; no sample |

Tests: the black-box Rust tests decode a small CC0 DNG from raw.pixls.us kept in the
sample images. A heavier integration test downloads one real camera file per RAW
extension (pinned SHA-256, cached); it is opt-in with `IMAGE_PROC_RAW_SAMPLES=1`
because the set is about 500 MB.

**Windows HEIC build**: `libheif` has no `choco` package, so CI builds it from source
with `vcpkg`, together with its codecs (`libde265` and `x265` for HEIC, `dav1d` for
AVIF). It is reliable but
adds 11–12 minutes to each Windows build; caching the vcpkg tree would remove most of
that.

## Build

The `hatch_build.py` hook compiles the Rust binary and bundles it into the wheel. On
Linux and macOS it also bundles `libheif` and its codecs next to the binary. On Windows
nothing else is bundled: vcpkg's `x64-windows-static-md` triplet links `libheif` and its
codecs into `image-proc.exe`, which only needs Windows system DLLs and the Microsoft C++
runtime. The binary is build output and is not tracked in git:

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

### Wheel tags and platform floor

The wheel holds a standalone binary and a pure-Python wrapper, no CPython
extension, so `hatch_build.py` tags it `py3-none-<platform>` instead of
inferring a `cpXYZ` tag. One wheel per OS then installs on every supported
Python version.

| OS | Wheel platform tag | Floor set by |
|---|---|---|
| macOS arm64 | `macosx_14_0_arm64` | `macos-14` CI runner (Homebrew libheif and codecs are built for it) and `MACOSX_DEPLOYMENT_TARGET=14.0` (Rust binary) |
| Linux x86_64 | `manylinux_2_34_x86_64` | `ubuntu-22.04` runner, retagged by `auditwheel repair` |
| Windows x86_64 | `win_amd64` | — |

On macOS, `delocate-wheel --require-target-macos-version` and a CI step reading
each bundled Mach-O file's `minos` fail the build if anything needs a newer macOS
than the floor. Raising the floor means moving the runner and `MACOS_MIN_VERSION`
in `.github/workflows/_build.yml` together.

⚠️ **No matching wheel means a silent sdist build without HEIC/RAW.** When pip or
uv find no wheel for the platform, they build from the sdist, and the hook only
enables the features when `IMAGE_PROC_FEATURE_HEIC`/`IMAGE_PROC_FEATURE_RAW` are
set. The resulting binary then answers HEIC requests with "rebuild with
--features heic". Woof's CI installs with `--only-binary ouestcharlie-imageproc`
to catch a platform or Python version the published wheels don't cover.

## Version Bumping

When the JSON protocol changes in a breaking way, bump both:
1. `image-proc/Cargo.toml` → `version = "X.Y.Z"` (bump major)
2. `src/ouestcharlie_imageproc/image_proc.py` → `IMAGE_PROC_PROTOCOL_MAJOR_VERSION = X`

The major component must match — image-proc validates `protocol_version` in every incoming request.
