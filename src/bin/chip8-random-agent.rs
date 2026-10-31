//! A deliberately simple autonomous agent: seeded, random, and limited to
//! interactions that leave the loaded program in place.
//!
//! One JSON document per line on stdin (a turn with the current machine
//! snapshot), one JSON action document per line on stdout.

use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

const SEED_VARIABLE: &str = "CHIP8_RUN_SEED";

struct Random(u32);

impl Random {
    fn next(&mut self) -> u32 {
        // Same xorshift32 shape as the emulator's random bytes: never zero.
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound
    }
}

/// Pick one of `step`, `inspect`, `key`, and `tick`. Never loads, resets,
/// or reseeds: the ROM stays fixed for the whole run.
fn choose(random: &mut Random) -> Value {
    match random.below(100) {
        0..=69 => json!({"type": "step"}),
        70..=79 => json!({"type": "inspect"}),
        80..=94 => json!({
            "type": "key",
            "key": random.below(16),
            "pressed": random.below(2) == 0,
        }),
        _ => json!({"type": "tick"}),
    }
}

fn main() -> Result<(), String> {
    let seed = std::env::var(SEED_VARIABLE)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let mut random = Random(if seed == 0 { 1 } else { seed });
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        serde_json::from_str::<Value>(&line)
            .map_err(|e| format!("turn is not JSON: {e}: {line}"))?;
        writeln!(out, "{}", json!({"action": choose(&mut random)}))
            .and_then(|_| out.flush())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
