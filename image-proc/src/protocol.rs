//! JSON request and response types of the stdin/stdout protocol.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct PhotoInput {
    pub path: PathBuf,
    pub ext: String,
    #[serde(default)]
    pub orientation: Option<u8>,
    pub content_hash: String,
}

/// Dispatch by shape: presence of "photos" (plural) → AvifGrid;
/// presence of "photo" (singular) → JpegPreview.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Request {
    AvifGrid(AvifGridInput),
    JpegPreview(JpegPreviewInput),
}

#[derive(Deserialize)]
pub struct AvifGridInput {
    pub photos: Vec<PhotoInput>,
    pub tile_size: u32,
    pub fit: String,
    pub quality: u8,
    pub output: PathBuf,
}

#[derive(Deserialize)]
pub struct JpegPreviewInput {
    pub photo: PhotoInput,
    pub max_long_edge: u32,
    pub quality: u8,
    pub output: PathBuf,
}

#[derive(Serialize)]
pub struct AvifGridOutput {
    pub rows: u32,
    #[serde(rename = "tileSize")]
    pub tile_size: u32,
    #[serde(rename = "photoOrder")]
    pub photo_order: Vec<String>,
}

#[derive(Serialize)]
pub struct JpegPreviewOutput {
    pub width: u32,
    pub height: u32,
}

/// Response envelope: either a successful result or an in-band error.
#[derive(Serialize)]
#[serde(untagged)]
pub enum Response {
    AvifGrid(AvifGridOutput),
    JpegPreview(JpegPreviewOutput),
    Error { error: String },
}
