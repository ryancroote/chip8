//! Minimal windowed renderer for the CHIP-8 display.
//!
//! Reads `State` and draws its 64x32 monochrome pixel grid into a `minifb`
//! window, scaled up to scale_factor per CHIP-8 pixel. Kept a thin,
//! zero-emulator-coupling consumer: it only renders and reports whether the
//! window is still open.
//!
//! See `.kiro/specs/chip8-display/requirements.md`.

use minifb::{ScaleMode, Window, WindowOptions};

use crate::chip8::State;

const DISPLAY_WIDTH: usize = 64;
const DISPLAY_HEIGHT: usize = 32;

const PIXEL_ON: u32 = 0x00FF_FFFF; // opaque white
const PIXEL_OFF: u32 = 0x0000_0000; // opaque black

pub struct Renderer {
    window: Window,
    /// One 32-bit pixel per CHIP-8 pixel (64x32), scaled up by the window.
    buffer: Vec<u32>,
    width: usize,
    height: usize,
}

impl Renderer {
    /// Open a window sized `DISPLAY_WIDTH * scale_factor` wide and
    /// `DISPLAY_HEIGHT * scale_factor` tall, and pace it to 60 FPS.
    pub fn new(scale_factor: usize) -> Result<Self, String> {
        let mut window = Window::new(
            "CHIP-8",
            DISPLAY_WIDTH * scale_factor,
            DISPLAY_HEIGHT * scale_factor,
            WindowOptions {
                scale_mode: ScaleMode::Stretch,
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;

        window.set_target_fps(60);

        Ok(Self {
            window,
            buffer: vec![0; DISPLAY_WIDTH * DISPLAY_HEIGHT],
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
        })
    }

    /// Push the current framebuffer to the window. The window remains visible
    /// even when the pixels are unchanged (per Requirement 2.5).
    pub fn render(&mut self, state: &State) {
        for (row, display_row) in state.display.iter().enumerate() {
            for (column, &pixel) in display_row.iter().enumerate() {
                let index = row * self.width + column;
                self.buffer[index] = if pixel { PIXEL_ON } else { PIXEL_OFF };
            }
        }

        // Ignore the result: a dropped frame on window-close is not fatal.
        let _ = self
            .window
            .update_with_buffer(&self.buffer, self.width, self.height);
    }

    /// True while the user has not requested the window be closed.
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer::new(10).expect("failed to create window")
    }
}
