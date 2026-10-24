"""RAW decoding against real camera files from raw.pixls.us.

One CC0 sample per supported RAW extension, downloaded on first run and checked
against a pinned SHA-256. The files are 2–70 MB each (~500 MB in total), so the
test is opt-in: set IMAGE_PROC_RAW_SAMPLES=1 to run it. Files are cached in
IMAGE_PROC_RAW_SAMPLES_DIR (default: ~/.cache/ouestcharlie/raw-samples).

Each sample is decoded at thumbnail size (256) and preview size (1440). Which
decode path runs depends on the embedded JPEG, measured with rawler 0.7.2:

- embedded JPEG >= 1440 px: CR2, CR3, NEF, ARW, RAF, RW2 (both sizes from the JPEG)
- embedded JPEG of 720 px: PEF (thumbnail from the JPEG, preview developed)
- no embedded JPEG: ORF, phone DNGs (both sizes developed)
- medium format and older makers mostly have no usable embedded JPEG and are
  developed at both sizes; NRW embeds a full-size JPEG
"""

from __future__ import annotations

import hashlib
import os
import pathlib
import urllib.parse
import urllib.request

import pytest

from ouestcharlie_imageproc.image_proc import OneTimeImageProc

PIXLS = "https://raw.pixls.us/getfile.php/"

# (file name, path under PIXLS, sha256, expected aspect ratio w/h)
SAMPLES = [
    (
        "canon_eos6d.cr2",
        "799/nice/Canon - EOS 6D - RAW.CR2",
        "360842779d08fe4805b4a2fe4979f06fdc6da982c85c3acb181f25bcf4c9f536",
        3 / 2,
    ),
    (
        "canon_eosr6.cr3",
        "4659/nice/Canon - EOS R6 - 3:2.CR3",
        "74abb0a113d075ad9887a058082f40dd2a938c4813a08474d82356f11a027778",
        3 / 2,
    ),
    (
        "nikon_z6.nef",
        "3587/nice/Nikon - Z 6 - 12bit 12bit compressed (3:2).NEF",
        "f0ab08da469ee205f2bc08efd9005778c64cdcb064893cee03768e32e5da1a36",
        3 / 2,
    ),
    (
        "sony_ilce7m3.arw",
        "2415/nice/Sony - ILCE-7M3 - 12bit 12bit compressed (3:2).ARW",
        "3ae2ff75cfa7240d0b4458f662afd28665c5fd1e83d230ac73f5ad69d3df8592",
        3 / 2,
    ),
    (
        "fujifilm_xt2.raf",
        "865/nice/Fujifilm - X-T2 - 14bit 14bit compressed (3:2).RAF",
        "f756ad5be551a8b3e1b503a8a6f7e8e637fba519c0b0f8c79dd58e94b0287602",
        3 / 2,
    ),
    (
        "olympus_em1mk2.orf",
        "1993/nice/Olympus - E-M1MarkII - 16bit (4:3).ORF",
        "0b433019e5fa61548d7eb0eb7bb90894251542c056da72355202382696d0ceb8",
        4 / 3,
    ),
    (
        "panasonic_dcg90.rw2",
        "4151/nice/Panasonic - DC-G90 - 4:3.RW2",
        "41de770701618b7276871fd1566da08782bf1009977c5bea91c0cc300bb8ed65",
        4 / 3,
    ),
    (
        "pentax_k1mk2.pef",
        "7133/nice/Pentax - K-1 Mark II - 14bit (3:2).PEF",
        "fe2330254c5313d4dde97cbd01e2987f6f1137f1c9c687d30e951f4668979381",
        3 / 2,
    ),
    (
        "apple_iphone6splus.dng",
        "915/nice/Apple - iPhone 6s Plus - 16bit (4:3).DNG",
        "845fee4f09c832af3728778737f5ae9b99124debb821b6e2066b94aed1daa91c",
        4 / 3,
    ),
    (
        "google_pixel4xl.dng",
        "3502/nice/Google - Pixel 4 XL - 16bit (4:3).dng",
        "0c03a6c93b4b8593cd0e4426166e6389710007238b82eb23861ef191322f0c54",
        4 / 3,
    ),
    # Formats added after the first set; MOS decodes portrait (stored rotated).
    (
        "nikon_coolpix_p1000.nrw",
        "3381/nice/Nikon - COOLPIX P1000 - 12bit 12bit uncompressed (4:3).NRW",
        "1d9470ee083f914e51aa0be782161b432f27f87dbef551a59ec435f5dbc289dd",
        4 / 3,
    ),
    (
        "samsung_nx500.srw",
        "3668/nice/Samsung - NX500 - 12bit 12bit normal compression (3:2).SRW",
        "156e43118811bcecb6fcc600b5f41ee1773c4f00b6a5b2dd72e177e35f6596ab",
        3 / 2,
    ),
    (
        "leica_c_typ112.rwl",
        "3017/nice/Leica - C (Typ 112) - 4:3.RWL",
        "081a6cf55204297f213e50b3398ff43e902dd684154ea79d1efea6503538176a",
        4 / 3,
    ),
    (
        "phaseone_iq180.iiq",
        "6394/nice/Phase One - IQ180 - IIQ L (4:3).IIQ",
        "cd852a703d076cf4d6744bff04409392aac273539595728b6e39ce964d0b32dd",
        4 / 3,
    ),
    (
        "hasselblad_h3d.3fr",
        "2851/nice/Hasselblad - H3D - 16bit (4:3).3FR",
        "34fdb93cb215e4f419ae0f7252c57b881fa381003124b4f9c143d27741f211b3",
        4 / 3,
    ),
    (
        "hasselblad_h5d40.fff",
        "1640/nice/Hasselblad - H5D-40 - 16bit (4:3).fff",
        "209d464e13702de5f4fdb830ac26ac140c98a8d0dc3e81c5ba61d93677330832",
        4 / 3,
    ),
    (
        "canon_eosd30.crw",
        "1307/nice/Canon - EOS D30 - RAW (3:2).CRW",
        "4ee482f2b6f9f1a8d370703ea04e9f2abaa49356973a7f56b229af93a78e8a2e",
        3 / 2,
    ),
    (
        "sony_dscf828.srf",
        "1351/nice/Sony - DSC-F828 - 4:3.SRF",
        "07739f94bcab0c7e198d418ac65172d0dce04bb84a7683ecffd0bfe4b32f6d79",
        4 / 3,
    ),
    (
        "sony_dscr1.sr2",
        "3221/nice/Sony - DSC-R1 - 14bit 14bit uncompressed (3:2).SR2",
        "921c5f2513dd1671a9089e73fc778fc3573bc4cb70a3a555d0414661d0e62274",
        3 / 2,
    ),
    (
        "epson_rd1.erf",
        "2680/nice/Epson - R-D1 - 12bit (3:2).ERF",
        "09d8e533d93116294a9f3e161ed868e929454c7b946e083b644bd6bb75bcb5e4",
        3 / 2,
    ),
    (
        "leaf_aptus22.mos",
        "3196/nice/Leaf - Aptus 22 - 16bit (4:3).mos",
        "8a1a101cb3940b0648af54a721d92e6fb8612dd29a0909036ea2753196a00ec2",
        3 / 4,
    ),
]

pytestmark = pytest.mark.skipif(
    not os.environ.get("IMAGE_PROC_RAW_SAMPLES"),
    reason="set IMAGE_PROC_RAW_SAMPLES=1 to download and test raw.pixls.us samples",
)


def _cache_dir() -> pathlib.Path:
    default = pathlib.Path.home() / ".cache" / "ouestcharlie" / "raw-samples"
    path = pathlib.Path(os.environ.get("IMAGE_PROC_RAW_SAMPLES_DIR", default))
    path.mkdir(parents=True, exist_ok=True)
    return path


def _sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _fetch(name: str, url: str, sha256: str) -> pathlib.Path:
    path = _cache_dir() / name
    if path.exists() and _sha256(path) == sha256:
        return path
    parts = urllib.parse.urlsplit(PIXLS + url)
    quoted = parts._replace(path=urllib.parse.quote(parts.path)).geturl()
    tmp = path.with_suffix(path.suffix + ".part")
    try:
        urllib.request.urlretrieve(quoted, tmp)
    except OSError as e:
        pytest.skip(f"cannot download {name}: {e}")
    if _sha256(tmp) != sha256:
        tmp.unlink()
        pytest.fail(f"checksum mismatch for {name}")
    tmp.replace(path)
    return path


@pytest.mark.asyncio
@pytest.mark.parametrize("max_long_edge", [256, 1440])
@pytest.mark.parametrize(("name", "url", "sha256", "aspect"), SAMPLES, ids=[s[0] for s in SAMPLES])
async def test_raw_sample_decodes(
    tmp_path: pathlib.Path, name: str, url: str, sha256: str, aspect: float, max_long_edge: int
) -> None:
    src = _fetch(name, url, sha256)
    output = tmp_path / "preview.jpg"
    result = await OneTimeImageProc().request(
        {
            "photo": {
                "path": str(src),
                "ext": src.suffix,
                "orientation": None,
                "content_hash": name,
            },
            "max_long_edge": max_long_edge,
            "quality": 85,
            "output": str(output),
        }
    )
    w, h = result["width"], result["height"]
    assert max(w, h) == max_long_edge
    assert abs(w / h - aspect) < 0.02, f"{name}: {w}x{h} is not {aspect:.3f}"
    assert output.stat().st_size > 0
