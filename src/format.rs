//! Number and size text for the UI. Matches the Python `human_size` in `DiskUsageViewer\scanner.py`.

pub fn human_size(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 {
            return if unit == "B" {
                format!("{size:.0} {unit}")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1024.0;
    }
    format!("{size:.1} TB")
}

pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0.0%".to_owned();
    }
    format!("{:.1}%", 100.0 * (part as f64) / (whole as f64))
}

pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.char_indices() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_small_values_have_no_decimals() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1023), "1023 B");
    }

    #[test]
    fn human_size_kb_mb_gb_have_one_decimal() {
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MB");
        assert_eq!(human_size(724_000_000_000), "674.3 GB");
    }

    #[test]
    fn human_size_tb_and_python_edge_cases() {
        assert_eq!(human_size(1024_u64.pow(4)), "1.0 TB");
        assert_eq!(human_size(1280), "1.2 KB");
        assert_eq!(human_size(1792), "1.8 KB");
        assert_eq!(human_size(1_048_575), "1024.0 KB");
        assert_eq!(human_size(1024_u64.pow(4) - 1), "1024.0 GB");
        assert_eq!(human_size(u64::MAX), "16777216.0 TB");
    }

    #[test]
    fn percent_of_whole() {
        assert_eq!(percent(1, 8), "12.5%");
        assert_eq!(percent(0, 8), "0.0%");
    }

    #[test]
    fn percent_of_zero_whole_is_zero() {
        assert_eq!(percent(5, 0), "0.0%");
    }

    #[test]
    fn grouped_inserts_commas() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1_933_720), "1,933,720");
        assert_eq!(grouped(u64::MAX), "18,446,744,073,709,551,615");
    }
}
