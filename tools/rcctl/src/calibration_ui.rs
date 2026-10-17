//! Physical raw-axis feedback and request-driven terminal input for calibration.
use std::io::{self, BufRead};
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Travel {
    pub current: u16,
    pub range: Option<(u16, u16)>,
}

impl Travel {
    pub fn observe(&mut self, raw: u16) {
        self.current = raw;
        self.range = Some(match self.range {
            Some((min, max)) => (min.min(raw), max.max(raw)),
            None => (raw, raw),
        });
    }

    pub fn line(&self, label: &str) -> String {
        let Some((min, max)) = self.range else {
            return format!("{label}: waiting for samples...");
        };
        let span = max - min;
        // Position uses the full ADC range, not the expanding learned range:
        // a stationary knob is never presented as having completed a sweep.
        let position = self.current as usize * 19 / u16::MAX as usize;
        let mut bar = [b'-'; 20];
        if span > 0 {
            let first = min as usize * 19 / u16::MAX as usize;
            let last = max as usize * 19 / u16::MAX as usize;
            bar[first..=last].fill(b'=');
        }
        bar[position] = b'|';
        format!(
            "{label} [{}] raw {:5} min {:5} max {:5} span {:5} ADC {:5.1}%",
            std::str::from_utf8(&bar).unwrap(),
            self.current,
            min,
            max,
            span,
            span as f64 * 100.0 / u16::MAX as f64
        )
    }
}

/// Avoid cursor redraw on terminals where rows could wrap. Unknown widths
/// use plain snapshots, as do redirected streams. `stty size` only reads size.
pub fn can_redraw() -> bool {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
        return false;
    }
    let width = std::env::var("COLUMNS").ok().and_then(|s| s.parse::<usize>().ok()).or_else(|| {
        let output = std::process::Command::new("stty")
            .arg("size")
            .stdin(std::process::Stdio::inherit())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout).ok()?.split_whitespace().nth(1)?.parse().ok()
    });
    width.is_some_and(|width| width >= 78)
}

/// Each request permits exactly one read. The worker never consumes the save
/// answer while the main thread is still learning or handling serial cleanup.
pub fn input_worker() -> (Sender<()>, Receiver<io::Result<Option<String>>>) {
    let (request_tx, request_rx) = mpsc::channel();
    let (line_tx, line_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        let mut stdin = stdin.lock();
        while request_rx.recv().is_ok() {
            let mut line = String::new();
            let result = stdin.read_line(&mut line).map(|n| if n == 0 { None } else { Some(line) });
            if line_tx.send(result).is_err() {
                break;
            }
        }
    });
    (request_tx, line_rx)
}

/// Always attempt both commands, even if stopping the stream fails.
/// Flags are set before setup commands, since a lost response can leave a
/// command active on the device despite reporting an error to the host.
pub fn cleanup(
    stream: &mut bool,
    learning: &mut bool,
    mut send: impl FnMut(&str) -> anyhow::Result<String>,
) -> anyhow::Result<Option<String>> {
    let mut first_error = None;
    if *stream {
        match send("STREAM OFF") {
            Ok(line) if crate::is_ok_response(&line) => *stream = false,
            Ok(line) => first_error = Some(anyhow::anyhow!("STREAM OFF failed: {line}")),
            Err(error) => first_error = Some(error),
        }
    }
    let mut stopped = None;
    if *learning {
        match send("CAL STOP") {
            Ok(line) => {
                // Both of these errors confirm learning has ended.
                if crate::is_ok_response(&line) || line == "ERR calibration" || line == "ERR not learning" {
                    *learning = false;
                } else if first_error.is_none() {
                    first_error = Some(anyhow::anyhow!("CAL STOP failed: {line}"));
                }
                stopped = Some(line);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(stopped),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn travel_expands_and_zero_span_is_not_completion() {
        let mut travel = Travel::default();
        assert!(travel.line("A0").contains("waiting"));
        travel.observe(30000);
        assert_eq!(travel.range, Some((30000, 30000)));
        assert!(travel.line("A0").contains("ADC   0.0%"));
        for raw in [40000, 20000, 35000] {
            travel.observe(raw);
        }
        assert_eq!(travel.range, Some((20000, 40000)));
        assert_eq!(travel.current, 35000);
    }
    #[test]
    fn position_bar_endpoints_and_full_adc_span() {
        let mut travel = Travel::default();
        travel.observe(0);
        assert!(travel.line("A0").contains("[|-------------------]"));
        travel.observe(u16::MAX);
        assert!(travel.line("A0").contains("[===================|]"));
        assert!(travel.line("A0").contains("ADC 100.0%"));
        assert!(travel.line("A0").len() <= 77);
    }
    #[test]
    fn observed_coverage_expands_inside_fixed_adc_scale() {
        let mut travel = Travel::default();
        travel.observe(20000); // cell 5, zero span: marker only.
        assert!(travel.line("A0").contains("[-----|--------------]"));
        travel.observe(40000); // cell 11: cells 5..11 observed.
        assert!(travel.line("A0").contains("[-----======|--------]"));
        travel.observe(30000); // cell 8: coverage unchanged, marker moves.
        assert!(travel.line("A0").contains("[-----===|===--------]"));
        travel.observe(10000); // cell 2: coverage widens left.
        assert!(travel.line("A0").contains("[--|=========--------]"));
    }

    #[test]
    fn cleanup_attempts_stop_after_stream_error() {
        let (mut stream, mut learning) = (true, true);
        let mut calls = vec![];
        let result = cleanup(&mut stream, &mut learning, |cmd| {
            calls.push(cmd.to_owned());
            if cmd == "STREAM OFF" {
                anyhow::bail!("disconnected")
            }
            Ok("ERR calibration".into())
        });
        assert!(result.is_err());
        assert_eq!(calls, ["STREAM OFF", "CAL STOP"]);
        assert!(!learning);
        assert!(stream);
    }
    #[test]
    fn cleanup_reports_unconfirmed_stop() {
        let (mut stream, mut learning) = (false, true);
        let result = cleanup(&mut stream, &mut learning, |_| Ok("ERR unknown command".into()));
        assert!(result.unwrap_err().to_string().contains("CAL STOP failed: ERR unknown command"));
        assert!(learning, "unexpected errors do not confirm learning ended");
    }

    #[test]
    fn cleanup_preserves_stop_response_and_clears_flags() {
        let (mut stream, mut learning) = (true, true);
        let result = cleanup(&mut stream, &mut learning, |cmd| {
            Ok(if cmd == "STREAM OFF" { "OK STREAM OFF" } else { "OK CAL 1 2000 1 2000 0 0 0 0" }.into())
        })
        .unwrap();
        assert!(result.unwrap().starts_with("OK CAL"));
        assert!(!stream && !learning);
    }
}
