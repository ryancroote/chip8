//! Attach to a separately started emulator; optionally execute a chosen action.
use chip8::controller::{Session, Timeouts};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 && args.len() != 6 {
        return Err("usage: cargo run --example observe -- ENDPOINT initial.png [result.png HEX_KEYS|- FRAMES]".into());
    }
    // This example accepts comma-separated hexadecimal keypad values, not
    // host keyboard labels. The library itself accepts only numeric values.
    let action = if args.len() == 6 {
        let keys = if args[4] == "-" {
            Vec::new()
        } else {
            args[4]
                .split(',')
                .map(|key| u8::from_str_radix(key, 16))
                .collect::<Result<Vec<_>, _>>()?
        };
        Some((keys, args[5].parse::<u32>()?))
    } else {
        None
    };
    let mut session = Session::connect(&args[1], Timeouts::default())?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let frame = session.observation();
        println!("{}x{} {}", frame.width(), frame.height(), frame.format());
        frame.save_png(&args[2])?;
        if let Some((keys, frames)) = action {
            let confirmed = session.action(&keys, frames)?;
            // A save failure is a filesystem error after a confirmed action.
            // Do not retry the action to repair an image-output failure.
            confirmed.save_png(&args[3])?;
        }
        Ok(())
    })();
    let disconnected = session.disconnect();
    result?;
    disconnected?;
    Ok(())
}
