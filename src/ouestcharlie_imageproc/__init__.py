"""OuEstCharlie image processing coprocessor — Rust binary and Python subprocess wrappers."""

from importlib.metadata import version

from .image_proc import (
    IMAGE_PROC_PROTOCOL_MAJOR_VERSION,
    OneTimeImageProc,
    PersistentImageProc,
)

__version__ = version("ouestcharlie-imageproc")

__all__ = [
    "IMAGE_PROC_PROTOCOL_MAJOR_VERSION",
    "OneTimeImageProc",
    "PersistentImageProc",
]
