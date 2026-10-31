//! Turning a machine snapshot's display into a viewable image.
//!
//! Images are 8-bit grayscale, one uniform block of `SCALE`x`SCALE` PNG pixels
//! per CHIP-8 pixel, so a viewer can tell individual pixels apart.

use serde_json::Value;
use std::path::Path;

pub const WIDTH: usize = 64;
pub const HEIGHT: usize = 32;
pub const SCALE: usize = 8;

const OFF: u8 = 0;
const ON: u8 = 255;

/// Read the display out of a machine snapshot, checking its shape.
pub fn display_from_snapshot(snapshot: &Value) -> Result<Vec<Vec<bool>>, String> {
    let rows = snapshot["display"]
        .as_array()
        .ok_or("snapshot has no display")?;
    if rows.len() != HEIGHT {
        return Err(format!("display has {} rows, expected {HEIGHT}", rows.len()));
    }
    rows.iter()
        .enumerate()
        .map(|(row, pixels)| {
            let pixels = pixels
                .as_array()
                .ok_or(format!("display row {row} is not an array"))?;
            if pixels.len() != WIDTH {
                return Err(format!(
                    "display row {row} has {} columns, expected {WIDTH}",
                    pixels.len()
                ));
            }
            pixels
                .iter()
                .map(|pixel| pixel.as_bool().ok_or("display pixel is not a boolean".into()))
                .collect()
        })
        .collect()
}

/// Write `display` to `path` as an enlarged monochrome PNG.
pub fn write_png(path: &Path, display: &[Vec<bool>]) -> Result<(), String> {
    if display.len() != HEIGHT || display.iter().any(|row| row.len() != WIDTH) {
        return Err("display is not 64x32".into());
    }
    let mut samples = vec![OFF; WIDTH * SCALE * HEIGHT * SCALE];
    for (row, columns) in display.iter().enumerate() {
        for (column, &pixel) in columns.iter().enumerate() {
            let value = if pixel { ON } else { OFF };
            for y in 0..SCALE {
                let line = row * SCALE + y;
                for x in 0..SCALE {
                    samples[line * WIDTH * SCALE + column * SCALE + x] = value;
                }
            }
        }
    }

    let file = std::fs::File::create(path).map_err(|e| format!("failed to create image: {e}"))?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        (WIDTH * SCALE) as u32,
        (HEIGHT * SCALE) as u32,
    );
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("failed to write image header: {e}"))?;
    writer
        .write_image_data(&samples)
        .map_err(|e| format!("failed to write image: {e}"))
}
