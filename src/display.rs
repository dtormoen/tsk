/// Formats a duration in seconds into a human-readable string.
///
/// - Negative or zero: `"0s"`
/// - Less than 60s: `"{s}s"` (e.g., `"45s"`)
/// - 60s to 3599s: `"{m}m {s}s"` (e.g., `"12m 5s"`)
/// - 3600s or more: `"{h}h {m}m"` (e.g., `"2h 30m"`)
pub fn format_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0s".to_string();
    }

    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;

    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Returns the visible character count, ignoring ANSI CSI escape sequences
/// (e.g. `\x1b[32m`). Does not handle OSC sequences.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            len += 1;
        }
    }
    len
}

/// Returns the ANSI color code for a status label, or `None` for unknown statuses.
///
/// Accepts both uppercase (`"QUEUED"`) and lowercase (`"queued"`) labels.
pub fn status_color(status: &str) -> Option<&'static str> {
    match status {
        "QUEUED" | "queued" => Some("\x1b[33m"),
        "WAITING" | "waiting" => Some("\x1b[36m"),
        "RUNNING" | "running" => Some("\x1b[34m"),
        "COMPLETE" | "complete" => Some("\x1b[32m"),
        "FAILED" | "failed" => Some("\x1b[31m"),
        _ => None,
    }
}

/// Wraps a status string in the appropriate ANSI color code.
///
/// Returns the string unchanged when `styled` is false.
pub fn colorize_status(status: &str, styled: bool) -> String {
    if !styled {
        return status.to_string();
    }
    match status_color(status) {
        Some(code) => format!("{code}{status}\x1b[0m"),
        None => status.to_string(),
    }
}

/// Prints column-aligned tabular output with headers and rows.
///
/// Headers are rendered with ANSI bold+underline styling when stdout is a terminal.
/// Each column is left-padded to the maximum width of its values (including the header),
/// with columns separated by two spaces. The last column has no trailing padding.
pub fn print_columns(headers: &[&str], rows: &[Vec<String>]) {
    let col_count = headers.len();
    let mut widths = vec![0usize; col_count];

    for (i, header) in headers.iter().enumerate() {
        widths[i] = header.chars().count();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_count {
                widths[i] = widths[i].max(visible_len(cell));
            }
        }
    }

    println!("{}", format_header_row(headers, &widths));
    for row in rows {
        print_row(row, &widths);
    }
}

fn format_header_row(headers: &[&str], widths: &[usize]) -> String {
    use is_terminal::IsTerminal;
    format_header_row_styled(headers, widths, std::io::stdout().is_terminal())
}

fn format_header_row_styled(headers: &[&str], widths: &[usize], styled: bool) -> String {
    let last = headers.len().saturating_sub(1);
    let parts: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(i, header)| {
            let padded = if i == last {
                header.to_string()
            } else {
                format!("{:<width$}", header, width = widths[i])
            };
            if styled {
                format!("\x1b[1;4m{padded}\x1b[0m")
            } else {
                padded
            }
        })
        .collect();
    parts.join("  ")
}

fn print_row(cells: &[impl AsRef<str>], widths: &[usize]) {
    let last = cells.len().saturating_sub(1);
    let parts: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let cell = cell.as_ref();
            if i == last {
                cell.to_string()
            } else {
                let visible = visible_len(cell);
                let padding = widths[i].saturating_sub(visible);
                format!("{cell}{:padding$}", "", padding = padding)
            }
        })
        .collect();
    println!("{}", parts.join("  "));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_header_row_styled() {
        let headers = &["Name", "Status", "Duration"];
        let widths = [10, 8, 8];
        let result = format_header_row_styled(headers, &widths, true);

        // Each header should be wrapped in bold+underline ANSI codes
        assert!(result.contains("\x1b[1;4mName      \x1b[0m"));
        assert!(result.contains("\x1b[1;4mStatus  \x1b[0m"));
        // Last header should have no trailing padding
        assert!(result.contains("\x1b[1;4mDuration\x1b[0m"));
        // One reset code per header
        assert_eq!(result.matches("\x1b[0m").count(), headers.len());
    }

    #[test]
    fn test_format_header_row_unstyled() {
        let headers = &["Name", "Status"];
        let widths = [10, 8];
        let result = format_header_row_styled(headers, &widths, false);

        assert!(!result.contains("\x1b["));
        assert_eq!(result, "Name        Status");
    }

    #[test]
    fn test_visible_len() {
        assert_eq!(visible_len("hello"), 5);
        assert_eq!(visible_len("\x1b[32mCOMPLETE\x1b[0m"), 8);
        assert_eq!(visible_len("\x1b[1;4mHeader\x1b[0m"), 6);
        assert_eq!(visible_len(""), 0);
    }

    #[test]
    fn test_colorize_status_styled() {
        let result = colorize_status("COMPLETE", true);
        assert_eq!(result, "\x1b[32mCOMPLETE\x1b[0m");

        let result = colorize_status("FAILED", true);
        assert_eq!(result, "\x1b[31mFAILED\x1b[0m");

        let result = colorize_status("RUNNING", true);
        assert_eq!(result, "\x1b[34mRUNNING\x1b[0m");

        let result = colorize_status("QUEUED", true);
        assert_eq!(result, "\x1b[33mQUEUED\x1b[0m");

        let result = colorize_status("WAITING", true);
        assert_eq!(result, "\x1b[36mWAITING\x1b[0m");
    }

    #[test]
    fn test_colorize_status_unstyled() {
        let result = colorize_status("COMPLETE", false);
        assert_eq!(result, "COMPLETE");
    }

    #[test]
    fn test_colorize_status_unknown() {
        let result = colorize_status("UNKNOWN", true);
        assert_eq!(result, "UNKNOWN");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(-5), "0s");
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(60), "1m 0s");
        assert_eq!(format_duration(725), "12m 5s");
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(9000), "2h 30m");
    }
}
