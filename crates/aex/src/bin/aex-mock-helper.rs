//! Protocol stand-in for tests. Not an After Effects host.

use std::io::Write;

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        match effectcraft_aex::mock_reply(&line) {
            effectcraft_aex::MockAction::Reply(text) => {
                if writeln!(stdout, "{text}").is_err() || stdout.flush().is_err() {
                    break;
                }
            }
            effectcraft_aex::MockAction::Exit(Some(text)) => {
                let _ = writeln!(stdout, "{text}");
                let _ = stdout.flush();
                break;
            }
            effectcraft_aex::MockAction::Exit(None) => std::process::exit(2),
        }
    }
}
