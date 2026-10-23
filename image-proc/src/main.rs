//! image-proc binary: reads one JSON request per line on stdin and writes one
//! JSON response per line on stdout. See the library crate for the protocol.

use std::io::{self, BufRead, Write};

use image_proc::process_line;

fn main() {
    // Support `--version` for version negotiation with the Python toolkit.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("image-proc {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let expected_major: u64 = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(1);

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("image-proc: failed to read line: {e}");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = process_line(&line, expected_major);
        let json = serde_json::to_string(&response).unwrap();
        if let Err(e) = writeln!(out, "{json}") {
            eprintln!("image-proc: failed to write response: {e}");
            break;
        }
        if let Err(e) = out.flush() {
            eprintln!("image-proc: failed to flush: {e}");
            break;
        }
    }
}
