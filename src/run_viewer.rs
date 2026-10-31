//! Optional live window for an exploratory run.
//!
//! Fed from the post-action machine snapshots the run already receives, so it
//! shows exactly what the run observed. The status strip beneath the display
//! tracks progress through the action budget, and the title carries the action
//! count and the action just taken.

use minifb::{ScaleMode, Window, WindowOptions};

use crate::display_image::{HEIGHT, WIDTH};

const PIXEL_ON: u32 = 0x00FF_FFFF;
const PIXEL_OFF: u32 = 0x0000_0000;
const STRIP_BACKGROUND: u32 = 0x0020_2020;
const STRIP_DONE: u32 = 0x0000_C000;
const STRIP_REMAINING: u32 = 0x0040_4040;
/// Height of the status strip, in screen pixels.
const STRIP_HEIGHT: usize = 20;

pub struct RunViewer {
    window: Window,
    buffer: Vec<u32>,
    width: usize,
    height: usize,
    display_height: usize,
}

impl RunViewer {
    /// Open a window `scale` times the CHIP-8 display, plus a status strip.
    pub fn new(scale: usize, title: &str) -> Result<Self, String> {
        let width = WIDTH * scale;
        let display_height = HEIGHT * scale;
        let height = display_height + STRIP_HEIGHT;
        let mut window = Window::new(
            title,
            width,
            height,
            WindowOptions {
                scale_mode: ScaleMode::Stretch,
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?;
        window.set_target_fps(60);
        Ok(Self {
            window,
            buffer: vec![STRIP_BACKGROUND; width * height],
            width,
            height,
            display_height,
        })
    }

    /// Draw `display` and progress through the action budget.
    pub fn show(
        &mut self,
        display: &[Vec<bool>],
        completed: usize,
        total: usize,
        latest: &str,
    ) -> Result<(), String> {
        let scale = self.display_height / HEIGHT;
        for (row, columns) in display.iter().enumerate().take(HEIGHT) {
            for (column, &pixel) in columns.iter().enumerate().take(WIDTH) {
                let value = if pixel { PIXEL_ON } else { PIXEL_OFF };
                for y in 0..scale {
                    let line = row * scale + y;
                    for x in 0..scale {
                        self.buffer[line * self.width + column * scale + x] = value;
                    }
                }
            }
        }

        let filled = if total == 0 {
            0
        } else {
            self.width * completed.min(total) / total
        };
        for y in self.display_height..self.height {
            for x in 0..self.width {
                self.buffer[y * self.width + x] = if x < filled {
                    STRIP_DONE
                } else {
                    STRIP_REMAINING
                };
            }
        }

        self.window
            .set_title(&format!("CHIP-8 run - action {completed}/{total} {latest}"));
        self.window
            .update_with_buffer(&self.buffer, self.width, self.height)
            .map_err(|error| error.to_string())
    }

    /// True while the user has not closed the window.
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }
}
