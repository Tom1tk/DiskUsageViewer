import Foundation

/// Byte count as text, base 1024: "512 B", "1.5 KB", "674.3 GB", and TB beyond that.
public func humanSize(_ bytes: UInt64) -> String {
    var size = Double(bytes)
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024 {
            return unit == "B" ? String(format: "%.0f B", size) : String(format: "%.1f \(unit)", size)
        }
        size /= 1024
    }
    return String(format: "%.1f TB", size)
}

/// "12.5%" for part of whole; "0.0%" when whole is zero.
public func percent(_ part: UInt64, of whole: UInt64) -> String {
    if whole == 0 {
        return "0.0%"
    }
    return String(format: "%.1f%%", 100 * Double(part) / Double(whole))
}

/// Digits with a comma every three places: "1,933,720".
public func grouped(_ n: UInt64) -> String {
    let digits = String(n)
    var out = ""
    for (i, ch) in digits.enumerated() {
        if i > 0 && (digits.count - i) % 3 == 0 {
            out.append(",")
        }
        out.append(ch)
    }
    return out
}
