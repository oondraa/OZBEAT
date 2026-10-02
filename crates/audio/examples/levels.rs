//! Prints live analysis levels, to check a capture actually hears the music.
//!
//! cargo run -p mv-audio --example levels -- [mic|loopback] [seconds]

use std::time::{Duration, Instant};

use mv_audio::{Capture, Input};

fn main() {
    let mut args = std::env::args().skip(1);
    let input = match args.next().as_deref() {
        Some("loopback") => Input::Loopback,
        _ => Input::Microphone,
    };
    let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(6);

    let capture = match Capture::start(input) {
        Ok(capture) => capture,
        Err(e) => {
            eprintln!("cannot open {input:?}: {e}");
            std::process::exit(1);
        }
    };
    println!("capturing {input:?} from: {}", capture.device());
    let bar = |v: f32| "█".repeat((v * 20.0).round() as usize);
    let mut peak_rms = 0.0f32;
    let started = Instant::now();
    let mut beats = 0;
    let mut previous_beat = 0.0;
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_millis(250));
        let l = capture.levels();
        peak_rms = peak_rms.max(l.rms);
        if l.beat > previous_beat + 0.3 {
            beats += 1;
        }
        previous_beat = l.beat;
        println!(
            "signal={:<5} bass {:<20} mid {:<20} treble {:<20} beat {:.2}",
            l.signal,
            bar(l.bass),
            bar(l.mid),
            bar(l.treble),
            l.beat
        );
    }
    println!(
        "{input:?}: ~{beats} beats seen in {seconds}s, peak rms {peak_rms:.6} (silence threshold 0.0001)"
    );
}
