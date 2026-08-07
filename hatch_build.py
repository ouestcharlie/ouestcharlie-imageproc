"""Hatch build hook: compile the image-proc Rust binary and bundle it into the wheel."""

import os
import shutil
import subprocess
import sys
from pathlib import Path

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version: str, build_data: dict) -> None:  # type: ignore[override]
        if self.target_name == "sdist":
            return  # sdist is source-only — no binary needed

        image_proc_dir = Path(__file__).parent / "image-proc"

        # Determine features to enable
        features = []
        if os.environ.get("IMAGE_PROC_FEATURE_RAW"):
            features.append("raw")
        if os.environ.get("IMAGE_PROC_FEATURE_HEIC"):
            features.append("heic")

        cmd = ["cargo", "build", "--release"]
        if features:
            cmd += ["--features", ",".join(features)]

        subprocess.run(cmd, cwd=image_proc_dir, check=True)

        # image-proc.exe on Windows, image-proc elsewhere
        binary_name = "image-proc.exe" if sys.platform == "win32" else "image-proc"
        src = image_proc_dir / "target" / "release" / binary_name

        bin_dir = Path(__file__).parent / "src" / "ouestcharlie_imageproc" / "bin"
        bin_dir.mkdir(exist_ok=True)
        dst = bin_dir / binary_name

        if sys.platform == "win32" and "heic" in features:
            self._copy_windows_heic_dlls(bin_dir)

        if self.target_name == "editable" and sys.platform != "win32":
            # Editable install: create a symlink so that subsequent
            # `cargo build --release` runs are picked up immediately
            # without reinstalling the wheel.
            if dst.exists() or dst.is_symlink():
                dst.unlink()
            dst.symlink_to(src.resolve())
        else:
            shutil.copy2(src, dst)
            if sys.platform != "win32":
                dst.chmod(dst.stat().st_mode | 0o111)

        # Mark the wheel as platform-specific (not pure Python)
        build_data["pure_python"] = False
        build_data["infer_tag"] = True

    def _copy_windows_heic_dlls(self, bin_dir: Path) -> None:
        """Copy libheif's runtime DLLs next to image-proc.exe.

        libheif-sys links libheif via vcpkg on Windows (dynamic by default), so
        the compiled binary needs libheif.dll and its transitive dependency DLLs
        (libde265, x265, aom, ...) alongside it — Windows resolves DLLs from the
        executable's own directory first. Runs for editable/dev installs too,
        not just CI wheels, so `pip install -e .` on Windows also works.
        """
        vcpkg_root = os.environ.get("VCPKG_INSTALLATION_ROOT") or os.environ.get("VCPKG_ROOT")
        if not vcpkg_root:
            print(
                "warning: heic feature built but no VCPKG_INSTALLATION_ROOT/VCPKG_ROOT set — "
                "cannot locate libheif DLLs to bundle; image-proc.exe will fail to start"
            )
            return

        triplet = os.environ.get("VCPKG_DEFAULT_TRIPLET", "x64-windows")
        dll_dir = Path(vcpkg_root) / "installed" / triplet / "bin"
        if not dll_dir.is_dir():
            print(f"warning: expected vcpkg DLL directory not found: {dll_dir}")
            return

        for dll in dll_dir.glob("*.dll"):
            shutil.copy2(dll, bin_dir / dll.name)
