# ouestcharlie-imageproc

Image processing coprocessor for [OuEstCharlie](https://github.com/ouestcharlie/ouestcharlie) — a Rust CLI (`image-proc`) bundled with a Python subprocess wrapper.

Handles all pixel-level operations: decoding, EXIF orientation, resize, fit, and encoding to AVIF or JPEG. Used by `ouestcharlie-py-toolkit` for thumbnail grid assembly and on-demand preview generation.

## What's in this package

- **`image-proc`** — Rust CLI that reads JSON requests from stdin and writes JSON responses to stdout (newline-delimited, persistent coprocessor model).
- **`ouestcharlie_imageproc.image_proc`** — Python subprocess wrappers: `OneTimeImageProc` (fresh process per request) and `PersistentImageProc` (long-lived process with asyncio lock).

## Supported formats

| Format | Notes |
|--------|-------|
| JPEG, PNG, WebP, TIFF | Default, pure Rust, all platforms |
| RAW (CR2, NEF, ARW, DNG, RAF, ORF, RW2, PEF) | Enable with `IMAGE_PROC_FEATURE_RAW=1` |
| HEIC/HEIF | Enable with `IMAGE_PROC_FEATURE_HEIC=1` — bundled into published wheels by default; requires `libheif` at build time only, none at runtime (its shared library is vendored into the wheel) |

## Building

Requires Rust stable, `nasm`, and (for HEIC) `libheif`:

```bash
# macOS
brew install nasm inih libheif

# Linux
sudo apt-get install nasm libheif-dev

# Windows
choco install nasm
# libheif via vcpkg — see .github/workflows/_build.yml for the bootstrap steps
```

```bash
# Install as editable (development)
pip install -e . --no-build-isolation

# Build release wheel
IMAGE_PROC_FEATURE_RAW=1 IMAGE_PROC_FEATURE_HEIC=1 hatch build --target wheel
```

Published wheels set both `IMAGE_PROC_FEATURE_RAW` and `IMAGE_PROC_FEATURE_HEIC` via
CI — a bare `pip install -e .` or `hatch build` without the env vars excludes both,
same as `cargo build` without `--features`.

## Running tests

```bash
# Unit tests (no binary required)
.venv/bin/python -m pytest tests/ -v

# Integration tests (require compiled binary)
.venv/bin/python -m pytest tests_integration/ -v

# Rust tests
cd image-proc && cargo test
```

## Design

See [imageproc_LLD.md](imageproc_LLD.md) for the protocol specification, command reference, and build details.
