//! Attach to an emulator started separately; never launches or advances it.
use chip8::controller::{Session, Timeouts};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        return Err("usage: cargo run --example observe -- ws://127.0.0.1:PORT display.png".into());
    }
    let session = Session::connect(&args[1], Timeouts::default())?;
    let frame = session.observation();
    println!("{}x{} {}", frame.width(), frame.height(), frame.format());
    // A save failure is a filesystem error, not an uncertain emulator action.
    let saved = frame.save_png(&args[2]);
    let disconnected = session.disconnect();
    saved?;
    disconnected?;
    Ok(())
}
