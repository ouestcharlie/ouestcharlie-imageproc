"""Generate the small format fixtures used by the image-proc decode tests.

Every image is a 64x48 gradient, so tests can check dimensions and that the
decoded pixels are not blank. Output goes next to this script.

The encoders are not dependencies of this package; run with:

    uv run --with pillow --with imagecodecs --with numpy \\
        python tests/sample-images/make_format_fixtures.py

10-bit AVIF additionally needs `heif-enc` (libheif) on PATH.
"""

from __future__ import annotations

import pathlib
import shutil
import subprocess
import tempfile

import imagecodecs
import numpy as np
from PIL import Image

OUT = pathlib.Path(__file__).parent
W, H = 64, 48


def gradient(depth: int = 8) -> np.ndarray:
    """RGB gradient: red grows left to right, green top to bottom."""
    top = (1 << depth) - 1
    x = np.linspace(0, top, W)
    y = np.linspace(0, top, H)
    r = np.tile(x, (H, 1))
    g = np.tile(y[:, None], (1, W))
    b = np.full((H, W), top / 2)
    dtype = np.uint8 if depth == 8 else np.uint16
    return np.stack([r, g, b], axis=-1).round().astype(dtype)


def rgb8() -> Image.Image:
    return Image.fromarray(gradient(8), "RGB")


def main() -> None:
    # TIFF variants
    rgb16 = gradient(16)
    imagecodecs.imwrite(OUT / "tiff_rgb16.tif", rgb16, codec="tiff", compression="adobe_deflate")
    imagecodecs.imwrite(OUT / "tiff_gray16.tif", rgb16[..., 1], codec="tiff")
    rgb8().save(OUT / "tiff_lzw.tif", compression="tiff_lzw")
    rgb8().save(OUT / "tiff_jpeg.tif", compression="jpeg")
    rgb8().convert("CMYK").save(OUT / "tiff_cmyk.tif")
    small = Image.new("RGB", (32, 32), (255, 0, 0))
    rgb8().save(OUT / "tiff_multipage.tif", save_all=True, append_images=[small])

    # WebP variants
    rgb8().save(OUT / "webp_lossless.webp", lossless=True)
    rgb8().save(
        OUT / "webp_animated.webp", save_all=True, append_images=[small.resize((W, H))]
    )

    # GIF (palette, animated) and BMP
    rgb8().save(OUT / "gif_animated.gif", save_all=True, append_images=[small.resize((W, H))])
    rgb8().save(OUT / "bmp_rgb.bmp")

    # AVIF: 8-bit through Pillow, 10-bit through heif-enc
    rgb8().save(OUT / "avif_8bit.avif", quality=80)
    heif_enc = shutil.which("heif-enc")
    if heif_enc:
        with tempfile.TemporaryDirectory() as tmp:
            png16 = pathlib.Path(tmp) / "in16.png"
            imagecodecs.imwrite(png16, rgb16, codec="png")
            subprocess.run(
                [heif_enc, "--avif", "-b", "10", "-q", "80", "-o", str(OUT / "avif_10bit.avif"),
                 str(png16)],
                check=True,
                capture_output=True,
            )
    else:
        print("heif-enc not found: avif_10bit.avif not regenerated")

    # JPEG XL: lossy 8-bit and lossless 16-bit
    imagecodecs.imwrite(OUT / "jxl_8bit.jxl", gradient(8), codec="jpegxl", level=90)
    imagecodecs.imwrite(OUT / "jxl_16bit.jxl", rgb16, codec="jpegxl", lossless=True)


if __name__ == "__main__":
    main()
