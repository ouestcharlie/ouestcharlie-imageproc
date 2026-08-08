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
            # (bundling deferred — editable installs use the developer's
            #  system libheif, so no bundling is needed below)
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
            if sys.platform.startswith("linux") and "heic" in features:
                self._bundle_linux_libs(dst, bin_dir)
            if sys.platform == "darwin" and "heic" in features:
                self._bundle_macos_libs(dst, bin_dir)

        # Mark the wheel as platform-specific (not pure Python)
        build_data["pure_python"] = False
        build_data["infer_tag"] = True

    def _bundle_linux_libs(self, binary: Path, bin_dir: Path) -> None:
        """Make the Linux image-proc binary self-contained.

        image-proc is a standalone executable, not a Python extension module,
        so `auditwheel repair` skips it — it only relinks .so extensions. Left
        alone, the wheel ships no libheif and the binary resolves libheif.so.1
        from the consumer's system. On distros whose stock libheif predates
        `heif_init` (e.g. Ubuntu 22.04's 1.12) this fails at runtime with
        `undefined symbol: heif_init`.

        Bundle the binary's non-system shared-library dependencies next to it
        and point its RUNPATH at $ORIGIN, mirroring the Windows DLL bundling.
        """
        if not shutil.which("patchelf"):
            print("warning: patchelf not found — cannot bundle libheif into the Linux wheel")
            return

        try:
            ldd = subprocess.run(
                ["ldd", str(binary)], capture_output=True, text=True, check=True
            ).stdout
        except subprocess.CalledProcessError as exc:
            print(f"warning: ldd failed on {binary}: {exc}")
            return

        # Bundle libheif and its codec deps; leave glibc/system libs to the host.
        # ldd output is transitive, so codec libs pulled in only by libheif
        # appear here too.
        bundle_prefixes = ("libheif", "libde265", "libx265", "libaom", "libdav1d")
        bundled: list[Path] = []
        for line in ldd.splitlines():
            if "=>" not in line:
                continue
            name = line.split("=>")[0].strip()
            resolved = line.split("=>")[1].strip().split(" (")[0].strip()
            if not resolved or not Path(resolved).exists():
                continue
            if not name.startswith(bundle_prefixes):
                continue
            dep = Path(resolved)
            copied = bin_dir / dep.name
            shutil.copy2(dep, copied)
            bundled.append(copied)

        # Point the binary and every bundled lib at $ORIGIN. DT_RUNPATH is not
        # transitive, so each bundled .so needs its own RUNPATH to resolve the
        # sibling codec libs it depends on.
        for target in (binary, *bundled):
            subprocess.run(["patchelf", "--set-rpath", "$ORIGIN", str(target)], check=True)

    def _bundle_macos_libs(self, binary: Path, bin_dir: Path) -> None:
        """Make the macOS image-proc binary self-contained.

        Like auditwheel on Linux, delocate only relinks Python extension
        modules — it leaves the standalone image-proc binary referencing its
        libheif by absolute path (e.g. /opt/homebrew/opt/libheif/lib/...). That
        path only exists on a machine with that exact Homebrew install, so the
        wheel fails with `dyld: Library not loaded` elsewhere.

        Copy the binary's non-system dylib dependencies (libheif and its codec
        libs, transitively) next to it, rewrite every install name to
        @loader_path/<name>, and re-sign — arm64 requires a valid signature and
        install_name_tool invalidates the ad-hoc one cargo produced.
        """

        def deps(path: Path) -> list[Path]:
            """Absolute, non-system dylib deps of a Mach-O file (excluding self)."""
            out = subprocess.run(
                ["otool", "-L", str(path)], capture_output=True, text=True, check=True
            ).stdout
            result = []
            for line in out.splitlines()[1:]:  # first line is the file's own path
                dep = line.strip().split(" (", 1)[0].strip()
                if not dep or dep.startswith(("/usr/lib/", "/System/", "@")):
                    continue
                if Path(dep).name == path.name:  # a dylib's own LC_ID_DYLIB line
                    continue
                if Path(dep).exists():
                    result.append(Path(dep))
            return result

        # Recursively collect and copy dependencies. Keyed by original absolute
        # path (the string that appears in load commands), so rewriting can
        # target the exact -change source below.
        copied: dict[str, Path] = {}
        queue = [binary]
        while queue:
            for dep in deps(queue.pop()):
                if str(dep) in copied:
                    continue
                dst = bin_dir / dep.name
                shutil.copy2(dep, dst)
                dst.chmod(dst.stat().st_mode | 0o200)  # Homebrew dylibs are read-only
                copied[str(dep)] = dst
                queue.append(dst)

        # Rewrite install names on the binary and every bundled dylib, then
        # re-sign. DT_RUNPATH has no macOS analogue that spans transitive deps,
        # so each file points at its siblings via @loader_path directly.
        for target in (binary, *copied.values()):
            if target is not binary:
                subprocess.run(
                    ["install_name_tool", "-id", f"@loader_path/{target.name}", str(target)],
                    check=True,
                )
            for original, dst in copied.items():
                subprocess.run(
                    ["install_name_tool", "-change", original, f"@loader_path/{dst.name}",
                     str(target)],
                    check=True,
                )
            subprocess.run(["codesign", "-f", "-s", "-", str(target)], check=True)

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
