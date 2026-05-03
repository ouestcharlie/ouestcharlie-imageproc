"""OuEstCharlie image processing coprocessor — Rust binary and Python subprocess wrappers."""

from .image_proc import (
    IMAGE_PROC_PROTOCOL_MAJOR_VERSION,
    OneTimeImageProc,
    PersistentImageProc,
)

__version__ = "1.0.0"

__all__ = [
    "IMAGE_PROC_PROTOCOL_MAJOR_VERSION",
    "OneTimeImageProc",
    "PersistentImageProc",
]
