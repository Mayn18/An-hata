use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use anahata_audio::AudioEngine;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    println!("=== Anāhata Audio Engine CLI Prototype ===");

    let mut engine = AudioEngine::new()?;
    if let Some(dev) = engine.device_name() {
        println!("Output device: {}", dev);
    } else {
        println!("No audio output device found (running in headless/silent mode).");
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let initial_file = PathBuf::from(&args[1]);
        println!("Loading initial file: {}", initial_file.display());
        if let Err(e) = engine.play_file(&initial_file) {
            eprintln!("Error playing file: {e}");
        }
    }

    println!("\nCommands:");
    println!("  p           : Pause / Resume toggle");
    println!("  s <seconds> : Seek to timestamp");
    println!("  v <0.0-1.0> : Set volume");
    println!("  l <file>    : Load and play new file");
    println!("  x           : Stop playback");
    println!("  q           : Quit");
    println!();

    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        let pos = engine.position_secs();
        let dur = engine.duration_secs().unwrap_or(0.0);
        let playing = if engine.is_playing() { "PLAYING" } else { "PAUSED" };
        print!("\r[{}] {:02.1}s / {:02.1}s > ", playing, pos, dur);
        io::stdout().flush()?;

        let line = match lines.next() {
            Some(Ok(l)) => l.trim().to_string(),
            _ => break,
        };

        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts[0] {
            "q" | "quit" | "exit" => {
                let _ = engine.stop();
                println!("\nExiting.");
                break;
            }
            "p" => {
                if engine.is_playing() {
                    let _ = engine.pause();
                    println!("\nPaused.");
                } else {
                    let _ = engine.resume();
                    println!("\nResumed.");
                }
            }
            "x" => {
                let _ = engine.stop();
                println!("\nStopped.");
            }
            "s" if parts.len() > 1 => {
                if let Ok(sec) = parts[1].parse::<f64>() {
                    let _ = engine.seek(sec);
                    println!("\nSeeked to {:.2}s.", sec);
                } else {
                    println!("\nInvalid seconds format.");
                }
            }
            "v" if parts.len() > 1 => {
                if let Ok(vol) = parts[1].parse::<f32>() {
                    let _ = engine.set_volume(vol);
                    println!("\nVolume set to {:.2}.", vol);
                } else {
                    println!("\nInvalid volume format.");
                }
            }
            "l" if parts.len() > 1 => {
                let path = PathBuf::from(parts[1]);
                println!("\nOpening {} ...", path.display());
                if let Err(e) = engine.play_file(&path) {
                    eprintln!("Failed to play: {e}");
                }
            }
            other => {
                println!("\nUnknown command: '{}'", other);
            }
        }
    }

    Ok(())
}
