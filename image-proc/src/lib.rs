//! image-proc — image processing coprocessor for OuEstCharlie.
//!
//! Reads a JSON object from stdin and dispatches to one of two commands
//! based on the shape of the input:
//!
//! # Command: avif_grid (when "photos" plural field is present)
//!
//! Decodes photos and assembles them into an AVIF grid container.
//!
//! ```json
//! {
//!   "photos": [
//!     { "path": "/tmp/staged.jpg", "ext": ".jpg", "orientation": 6, "content_hash": "Kf3QzA2nBcR8xYvLm1P9w"},
//!     ...
//!   ],
//!   "tile_size": 256,
//!   "fit": "crop",
//!   "quality": 55,
//!   "output": "/tmp/output.avif"
//! }
//! ```
//! `fit` is `"crop"` (center-crop to square) or `"pad"` (letterbox with black).
//!
//! Output:
//! ```json
//! {"cols": 32, "rows": 4, "tileSize": 256, "photoOrder": ["Kf3QzA2nBcR8xYvLm1P9w", ...]}
//! ```
//!
//! # Command: jpeg_preview (when "photo" singular field is present)
//!
//! Decodes a single photo, applies EXIF orientation, resizes to max_long_edge,
//! and saves as JPEG.
//!
//! ```json
//! {
//!   "photo": { "path": "/tmp/staged.cr2", "ext": ".cr2", "orientation": 6, "content_hash": "Kf3QzA2nBcR8xYvLm1P9w"},
//!   "max_long_edge": 1440,
//!   "quality": 85,
//!   "output": "/tmp/preview.jpg"
//! }
//! ```
//!
//! Output:
//! ```json
//! {"width": 1440, "height": 960}
//! ```
//!
//! # Grid layout (avif_grid)
//!
//! Photos are always arranged in 8-column rows: `cols = 8`,
//! `rows = ceil(n / 8)`.  The last row is padded with blank black tiles.
//!
//! The caller is responsible for ordering photos by content_hash before
//! passing them here, to ensure stable tile indices.

pub mod commands;
pub mod decode;
pub mod protocol;
pub mod transform;

use commands::{avif_grid::run_avif_grid, jpeg_preview::run_jpeg_preview};
use protocol::{Request, Response};

/// Process one JSON line and return the response to serialise.
/// Kept out of `main` so it can be tested without a stdin loop.
pub fn process_line(line: &str, expected_major: u64) -> Response {
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Response::Error {
                error: format!("invalid JSON input: {e}"),
            }
        }
    };
    match value.get("protocol_version").and_then(|v| v.as_u64()) {
        None => Response::Error {
            error: "missing or invalid protocol_version field".to_string(),
        },
        Some(proto) if proto != expected_major => Response::Error {
            error: format!("unsupported protocol version {proto}, expected {expected_major}"),
        },
        Some(_) => match serde_json::from_value::<Request>(value) {
            Err(e) => Response::Error {
                error: format!("invalid request: {e}"),
            },
            Ok(Request::AvifGrid(input)) => match run_avif_grid(input) {
                Ok(out) => Response::AvifGrid(out),
                Err(e) => Response::Error {
                    error: e.to_string(),
                },
            },
            Ok(Request::JpegPreview(input)) => match run_jpeg_preview(input) {
                Ok(out) => Response::JpegPreview(out),
                Err(e) => Response::Error {
                    error: e.to_string(),
                },
            },
        },
    }
}
