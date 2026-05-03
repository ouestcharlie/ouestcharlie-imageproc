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
| HEIC/HEIF | Enable with `IMAGE_PROC_FEATURE_HEIC=1` (requires system `libheif`) |

## Building

Requires Rust stable and `nasm`:

```bash
# macOS
brew install nasm inih

# Linux
sudo apt-get install nasm

# Windows
choco install nasm
```

```bash
# Install as editable (development)
pip install -e . --no-build-isolation

# Build release wheel
IMAGE_PROC_FEATURE_RAW=1 hatch build --target wheel
```

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
