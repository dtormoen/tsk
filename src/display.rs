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

/// Prints column-aligned tabular output with headers and rows.
///
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
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
    }

    print_row(headers, &widths);
    for row in rows {
        print_row(row, &widths);
    }
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
                format!("{:<width$}", cell, width = widths[i])
            }
        })
        .collect();
    println!("{}", parts.join("  "));
}

#[cfg(test)]
mod tests {
    use super::*;

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
