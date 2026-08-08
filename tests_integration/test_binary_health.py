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
import subprocess
import sys
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
    bundled_names = {
        p.name for p in bin_dir.iterdir() if p.name.startswith(_BUNDLED_LIB_PREFIXES)
    }
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


@requires_binary
@pytest.mark.skipif(sys.platform != "win32", reason="Windows DLL check")
def test_windows_binary_has_bundled_dlls() -> None:
    """On Windows, libheif's runtime DLLs sit next to image-proc.exe.

    Windows resolves imported DLLs from the executable's own directory first, so
    _copy_windows_heic_dlls bundles libheif.dll and its transitive codec DLLs
    beside the .exe. This asserts a libheif DLL is present; the launch test
    covers whether every import actually resolves at load time. Skipped when no
    DLLs are bundled (e.g. a heic-less build).
    """
    binary = Path(_find_image_proc_binary()).resolve()
    bin_dir = binary.parent
    dlls = {p.name for p in bin_dir.iterdir() if p.suffix.lower() == ".dll"}
    if not dlls:
        pytest.skip("no DLLs bundled next to the binary (heic-less build)")

    assert any("heif" in name.lower() for name in dlls), (
        f"no libheif DLL bundled next to image-proc.exe; found: {sorted(dlls)}"
    )
