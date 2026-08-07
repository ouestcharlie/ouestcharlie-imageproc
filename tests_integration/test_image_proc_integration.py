"""Integration tests for image-proc via OneTimeImageProc and PersistentImageProc.

These tests spawn the real image-proc binary and exercise the full
Python↔Rust JSON protocol, including protocol version checking.

The binary is resolved via _find_image_proc_binary() — either IMAGE_PROC_BINARY
env var or the bundled bin/image-proc symlink. Tests are skipped automatically
when the binary cannot be found.
"""

from __future__ import annotations

import asyncio
import pathlib
import tempfile
from unittest.mock import patch

import pytest
import pytest_asyncio

from ouestcharlie_imageproc.image_proc import (
    IMAGE_PROC_PROTOCOL_MAJOR_VERSION,
    OneTimeImageProc,
    PersistentImageProc,
    _find_image_proc_binary,
)

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

SAMPLE_DIR = pathlib.Path(__file__).parent.parent / "tests" / "sample-images"
SAMPLE_JPEG_001 = SAMPLE_DIR / "001.jpg"
SAMPLE_HEIC_003 = SAMPLE_DIR / "003.heic"


def _binary_available() -> bool:
    try:
        _find_image_proc_binary()
        return True
    except FileNotFoundError:
        return False


requires_binary = pytest.mark.skipif(
    not _binary_available(),
    reason="image-proc binary not found (set IMAGE_PROC_BINARY or run cargo build --release)",
)


def _heic_supported() -> bool:
    """Probe whether the resolved binary was built with --features heic.

    A plain local `cargo build --release` (no env vars) excludes heic, same as
    `raw` — only CI-built wheels set IMAGE_PROC_FEATURE_HEIC=1. Skip the HEIC
    test in that case rather than failing on an expected "rebuild with
    --features heic" error.
    """
    if not _binary_available():
        return False

    async def _probe() -> bool:
        with tempfile.TemporaryDirectory() as tmpdir:
            proc = OneTimeImageProc()
            try:
                await proc.request(
                    {
                        "photo": {
                            "path": str(SAMPLE_HEIC_003),
                            "ext": ".heic",
                            "orientation": None,
                            "content_hash": "heic_probe",
                        },
                        "max_long_edge": 32,
                        "quality": 50,
                        "output": str(pathlib.Path(tmpdir) / "probe.jpg"),
                    }
                )
                return True
            except RuntimeError as e:
                if "rebuild with --features heic" in str(e):
                    return False
                return True  # some other error — let the real test surface it

    return asyncio.run(_probe())


requires_heic = pytest.mark.skipif(
    not _heic_supported(),
    reason="image-proc binary built without --features heic",
)


@pytest_asyncio.fixture
async def persistent_proc():
    async with PersistentImageProc() as proc:
        yield proc


# ---------------------------------------------------------------------------
# OneTimeImageProc
# ---------------------------------------------------------------------------


@requires_binary
@pytest.mark.asyncio
async def test_one_time_jpeg_preview_returns_dimensions(tmp_path: pathlib.Path) -> None:
    """OneTimeImageProc: real JPEG → image-proc returns valid width/height."""
    output = tmp_path / "preview.jpg"
    proc = OneTimeImageProc()
    result = await proc.request(
        {
            "photo": {
                "path": str(SAMPLE_JPEG_001),
                "ext": ".jpg",
                "orientation": None,
                "content_hash": "test001",
            },
            "max_long_edge": 800,
            "quality": 85,
            "output": str(output),
        }
    )
    assert isinstance(result["width"], int) and result["width"] > 0
    assert isinstance(result["height"], int) and result["height"] > 0
    assert max(result["width"], result["height"]) <= 800
    assert output.exists()


@requires_heic
@pytest.mark.asyncio
async def test_one_time_heic_preview_returns_dimensions(tmp_path: pathlib.Path) -> None:
    """OneTimeImageProc: real HEIC → image-proc decodes it and returns valid width/height."""
    output = tmp_path / "preview.jpg"
    proc = OneTimeImageProc()
    result = await proc.request(
        {
            "photo": {
                "path": str(SAMPLE_HEIC_003),
                "ext": ".heic",
                "orientation": None,
                "content_hash": "test_heic_003",
            },
            "max_long_edge": 800,
            "quality": 85,
            "output": str(output),
        }
    )
    assert result == {"width": 64, "height": 48}
    assert output.exists()


@requires_binary
@pytest.mark.asyncio
async def test_one_time_wrong_protocol_version_raises(tmp_path: pathlib.Path) -> None:
    """OneTimeImageProc: wrong protocol_version major → RuntimeError with clear message."""
    wrong_major = IMAGE_PROC_PROTOCOL_MAJOR_VERSION + 1
    proc = OneTimeImageProc()
    with (
        patch("ouestcharlie_imageproc.image_proc.IMAGE_PROC_PROTOCOL_MAJOR_VERSION", wrong_major),
        pytest.raises(RuntimeError, match="unsupported protocol version"),
    ):
        await proc.request(
            {
                "photo": {
                    "path": str(SAMPLE_JPEG_001),
                    "ext": ".jpg",
                    "orientation": None,
                    "content_hash": "test_wrong",
                },
                "max_long_edge": 800,
                "quality": 85,
                "output": str(tmp_path / "out.jpg"),
            }
        )


# ---------------------------------------------------------------------------
# PersistentImageProc
# ---------------------------------------------------------------------------


@requires_binary
@pytest.mark.asyncio
async def test_persistent_jpeg_preview_returns_dimensions(
    tmp_path: pathlib.Path, persistent_proc: PersistentImageProc
) -> None:
    """PersistentImageProc: real JPEG → image-proc returns valid width/height."""
    output = tmp_path / "preview.jpg"
    result = await persistent_proc.request(
        {
            "photo": {
                "path": str(SAMPLE_JPEG_001),
                "ext": ".jpg",
                "orientation": None,
                "content_hash": "test001_persistent",
            },
            "max_long_edge": 600,
            "quality": 80,
            "output": str(output),
        }
    )
    assert isinstance(result["width"], int) and result["width"] > 0
    assert isinstance(result["height"], int) and result["height"] > 0
    assert max(result["width"], result["height"]) <= 600
    assert output.exists()


@requires_binary
@pytest.mark.asyncio
async def test_persistent_multiple_requests_same_process(
    tmp_path: pathlib.Path, persistent_proc: PersistentImageProc
) -> None:
    """PersistentImageProc: multiple sequential requests reuse the same process."""
    for i in range(3):
        output = tmp_path / f"preview_{i}.jpg"
        result = await persistent_proc.request(
            {
                "photo": {
                    "path": str(SAMPLE_JPEG_001),
                    "ext": ".jpg",
                    "orientation": None,
                    "content_hash": f"multi_{i}",
                },
                "max_long_edge": 400,
                "quality": 75,
                "output": str(output),
            }
        )
        assert result["width"] > 0
        assert result["height"] > 0
        assert output.exists()


@requires_binary
@pytest.mark.asyncio
async def test_persistent_error_response_raises_runtime_error(
    tmp_path: pathlib.Path, persistent_proc: PersistentImageProc
) -> None:
    """PersistentImageProc: image-proc in-band error → RuntimeError, process stays alive."""
    with pytest.raises(RuntimeError, match="image-proc error"):
        await persistent_proc.request(
            {
                "photo": {
                    "path": str(tmp_path / "nonexistent.jpg"),
                    "ext": ".jpg",
                    "orientation": None,
                    "content_hash": "missing",
                },
                "max_long_edge": 800,
                "quality": 85,
                "output": str(tmp_path / "out.jpg"),
            }
        )

    # Process must still be alive and accept a new request after an error.
    output = tmp_path / "recovery.jpg"
    result = await persistent_proc.request(
        {
            "photo": {
                "path": str(SAMPLE_JPEG_001),
                "ext": ".jpg",
                "orientation": None,
                "content_hash": "recovery",
            },
            "max_long_edge": 400,
            "quality": 75,
            "output": str(output),
        }
    )
    assert result["width"] > 0
    assert output.exists()
