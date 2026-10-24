"""Binding health checks for the bundled image-proc binary.

These tests target the *binding* between the Python wrapper and the Rust
crate — that the compiled binary can actually be launched and that its native
dependencies resolve — as opposed to the functional protocol tests in
test_image_proc_integration.py.

Why this matters: image-proc is a standalone executable that dynamically links
libheif (and its codecs). The dynamic linker resolves those NEEDED libraries at
process start, *before* any request is sent, so a linking fault surfaces on the
very first launch as a non-zero exit (e.g. exit 127 with
``symbol lookup error: ... undefined symbol: heif_init`` when the host's libheif
is older than the one the binary was built against). Running ``--version`` is
the cheapest way to force that resolution and fail loudly with a readable
message instead of leaving it to be discovered by a downstream consumer.
"""

from __future__ import annotations

import os
import re
import struct
import subprocess
import sys
from importlib import metadata
from pathlib import Path

import pytest

from ouestcharlie_imageproc.image_proc import (
    IMAGE_PROC_PROTOCOL_MAJOR_VERSION,
    _find_image_proc_binary,
)


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

# Shared libraries the wheel is expected to ship next to the binary (Linux).
_BUNDLED_LIB_PREFIXES = ("libheif", "libde265", "libx265", "libaom", "libdav1d")


@requires_binary
def test_binary_launches_and_reports_version() -> None:
    """The binary starts and prints its version.

    Launching resolves every NEEDED shared library, so this fails with the
    linker's own diagnostic (captured from stderr) if a native dependency such
    as libheif is missing or ABI-incompatible — the exact failure mode that
    slips past a file-existence check.
    """
    binary = _find_image_proc_binary()
    result = subprocess.run(
        [binary, "--version"],
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, (
        f"image-proc --version exited {result.returncode}; "
        f"likely a native-linking fault.\nstdout: {result.stdout!r}\nstderr: {result.stderr!r}"
    )
    assert result.stdout.startswith("image-proc "), f"unexpected version output: {result.stdout!r}"


@requires_binary
def test_binary_major_version_matches_protocol() -> None:
    """The binary's major version matches the Python protocol constant.

    The wrapper stamps IMAGE_PROC_PROTOCOL_MAJOR_VERSION onto every request and
    the crate rejects a mismatched major, so drift between the installed binary
    and the Python module is a binding break. --version reports the Cargo
    version, whose major component tracks the protocol major.
    """
    binary = _find_image_proc_binary()
    out = subprocess.run(
        [binary, "--version"], capture_output=True, text=True, timeout=30, check=True
    ).stdout.strip()
    # Format: "image-proc X.Y.Z"
    version = out.split()[-1]
    major = int(version.split(".")[0])
    assert major == IMAGE_PROC_PROTOCOL_MAJOR_VERSION, (
        f"binary major version {major} (from {out!r}) != "
        f"IMAGE_PROC_PROTOCOL_MAJOR_VERSION {IMAGE_PROC_PROTOCOL_MAJOR_VERSION}"
    )


def test_installed_wheel_tag_is_python_independent() -> None:
    """The installed wheel is tagged py3-none-<platform>, not cpXYZ-cpXYZ.

    The wheel holds a standalone binary and a pure-Python wrapper, so it must
    install on any Python version. A CPython-specific tag makes pip/uv fall back
    to the sdist on every other version, and that local build silently lacks
    HEIC/RAW. Skipped when the package is not installed from a wheel.
    """
    try:
        wheel_meta = metadata.distribution("ouestcharlie-imageproc").read_text("WHEEL")
    except metadata.PackageNotFoundError:
        pytest.skip("ouestcharlie-imageproc is not installed")
    if not wheel_meta:
        pytest.skip("no WHEEL metadata (not installed from a wheel)")

    tags = [
        line.split(":", 1)[1].strip() for line in wheel_meta.splitlines() if line.startswith("Tag:")
    ]
    assert tags, f"no Tag in WHEEL metadata:\n{wheel_meta}"
    for tag in tags:
        assert tag.startswith("py3-none-"), f"wheel tag {tag!r} is tied to a Python version"


@requires_binary
@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="Linux dynamic-linking check")
def test_linux_binary_is_self_contained() -> None:
    """On Linux, bundled native libs resolve to the wheel, not the host.

    A repaired wheel ships libheif and its codecs next to the binary with
    RUNPATH=$ORIGIN so it does not depend on the host's system libheif. This
    asserts that self-containment directly: if libheif is bundled, ``ldd`` must
    resolve it (and every sibling codec lib) to a path inside the binary's own
    directory. Skipped for a plain local ``cargo build`` where nothing is
    bundled and the host's libraries are used by design.
    """
    binary = Path(_find_image_proc_binary()).resolve()
    bin_dir = binary.parent
    bundled_names = {p.name for p in bin_dir.iterdir() if p.name.startswith(_BUNDLED_LIB_PREFIXES)}
    if not bundled_names:
        pytest.skip("no bundled native libs next to the binary (dev/local build)")

    ldd = subprocess.run(
        ["ldd", str(binary)], capture_output=True, text=True, timeout=30, check=True
    ).stdout

    checked = 0
    for line in ldd.splitlines():
        if "=>" not in line:
            continue
        name = line.split("=>", 1)[0].strip()
        if not name.startswith(_BUNDLED_LIB_PREFIXES):
            continue
        resolved = line.split("=>", 1)[1].strip().split(" (", 1)[0].strip()
        assert resolved and Path(resolved).exists(), f"{name} did not resolve: {line!r}"
        assert os.path.samefile(Path(resolved).parent, bin_dir), (
            f"{name} resolved to {resolved}, outside the bundle {bin_dir} — "
            "the binary is loading a host library instead of the bundled one"
        )
        checked += 1

    assert checked, f"expected bundled libs {bundled_names} to appear in ldd output:\n{ldd}"


@requires_binary
@pytest.mark.skipif(sys.platform != "darwin", reason="macOS dynamic-linking check")
def test_macos_binary_is_self_contained() -> None:
    """On macOS, native deps load relative to the bundle, not an absolute path.

    delocate rewrites Python extension modules but leaves the standalone binary
    alone, so a self-contained wheel must rewrite the binary's install names to
    ``@loader_path``/``@rpath``/``@executable_path`` and ship the dylibs beside
    it. This asserts no libheif/codec dependency points at an absolute path such
    as ``/opt/homebrew/...`` (which only exists on a machine with that exact
    Homebrew install). System dylibs under /usr/lib and /System are always
    present, so they are exempt. Skipped for a plain local build where the
    dylibs are not bundled and Homebrew paths are used by design.
    """
    binary = Path(_find_image_proc_binary()).resolve()
    bin_dir = binary.parent
    if not any(p.suffix == ".dylib" for p in bin_dir.iterdir()):
        pytest.skip("no bundled .dylib next to the binary (dev/local build)")

    otool = subprocess.run(
        ["otool", "-L", str(binary)], capture_output=True, text=True, timeout=30, check=True
    ).stdout

    offenders = []
    # First line is the binary's own path; the rest are "  <path> (compat ...)".
    for line in otool.splitlines()[1:]:
        dep = line.strip().split(" (", 1)[0].strip()
        if not dep:
            continue
        name = dep.rsplit("/", 1)[-1]
        if not name.startswith(_BUNDLED_LIB_PREFIXES):
            continue
        if dep.startswith(("@loader_path", "@rpath", "@executable_path", "/usr/lib/", "/System/")):
            continue
        offenders.append(dep)

    assert not offenders, (
        "binary loads native libraries by absolute path instead of from the bundle: "
        f"{offenders}\nfull otool -L:\n{otool}"
    )


def _pe_imports(path: Path) -> list[str]:
    """Return the DLL names a PE32+ (64-bit Windows) executable imports.

    Reads the import directory (normal and delay-load) from the PE header with
    the standard library only, so the check runs without extra dependencies.
    """
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    sections, opt_size = struct.unpack_from("<HxxxxxxxxxxxxH", data, pe + 6)
    opt = pe + 24
    if struct.unpack_from("<H", data, opt)[0] != 0x20B:
        raise ValueError(f"{path} is not a PE32+ executable")
    data_dirs = opt + 112
    table = [struct.unpack_from("<8sIIII", data, opt + opt_size + i * 40) for i in range(sections)]

    def offset(rva: int) -> int:
        for _name, vsize, vaddr, rsize, raddr in table:
            if vaddr <= rva < vaddr + max(vsize, rsize):
                return rva - vaddr + raddr
        raise ValueError(f"RVA {rva:#x} outside any section")

    names = []
    # (data directory index, descriptor size, offset of the name RVA)
    for index, size, name_at in ((1, 20, 12), (13, 32, 4)):
        rva = struct.unpack_from("<I", data, data_dirs + index * 8)[0]
        if not rva:
            continue
        desc = offset(rva)
        while name_rva := struct.unpack_from("<I", data, desc + name_at)[0]:
            start = offset(name_rva)
            names.append(data[start : data.index(b"\0", start)].decode())
            desc += size
    return names


# Microsoft C++ runtime DLLs: often present in System32 (installed by the Visual
# C++ Redistributable, as on CI runners) but not part of Windows, so they must
# ship with the binary.
_VC_REDIST = re.compile(r"(vcruntime\d.*|msvcp[\d_].*|concrt\d.*|vcomp\d*.*)\.dll")


@requires_binary
@pytest.mark.skipif(sys.platform != "win32", reason="Windows dependency check")
def test_windows_binary_bundles_its_dlls() -> None:
    """On Windows, every DLL image-proc.exe imports is either part of Windows or bundled.

    CI runners have the Visual C++ Redistributable installed, so a missing
    msvcp140.dll would not show up as a launch failure there. This reads the
    exe's import table instead and requires the C++ runtime DLLs, and any DLL
    not found in System32, to sit next to the exe. Skipped for a dev build
    selected with IMAGE_PROC_BINARY, which is not bundled.
    """
    if os.environ.get("IMAGE_PROC_BINARY"):
        pytest.skip("IMAGE_PROC_BINARY points at a dev build (not bundled)")
    binary = Path(_find_image_proc_binary()).resolve()
    bundled = {p.name.lower() for p in binary.parent.iterdir()}
    system32 = Path(os.environ.get("SYSTEMROOT", r"C:\Windows")) / "System32"

    missing = []
    for dll in _pe_imports(binary):
        name = dll.lower()
        if name.startswith(("api-ms-win-", "ext-ms-win-")):
            continue  # Windows API sets, resolved by the loader itself
        part_of_windows = (system32 / dll).exists() and not _VC_REDIST.fullmatch(name)
        if not part_of_windows and name not in bundled:
            missing.append(dll)
    assert not missing, f"DLLs image-proc.exe needs but the wheel does not bundle: {missing}"
