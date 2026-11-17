//! Formatting helpers and file-type detection.

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 { format!("{value:.0} {}", UNITS[unit]) } else { format!("{value:.1} {}", UNITS[unit]) }
}

pub fn human_time(secs: Option<i64>) -> String {
    let Some(secs) = secs else { return "—".into() };
    let Some(utc) = chrono::DateTime::from_timestamp(secs, 0) else { return "—".into() };
    let local = utc.with_timezone(&chrono::Local);
    let now = chrono::Local::now();
    if local.date_naive() == now.date_naive() {
        local.format("Today %H:%M").to_string()
    } else if local.format("%Y").to_string() == now.format("%Y").to_string() {
        local.format("%b %d, %H:%M").to_string()
    } else {
        local.format("%b %d, %Y").to_string()
    }
}

pub fn full_time(secs: Option<i64>) -> String {
    secs.and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S %Z").to_string())
        .unwrap_or_else(|| "—".into())
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// A short type label for the "Kind" column.
pub fn kind_label(name: &str) -> String {
    let ext = extension(name);
    if ext.is_empty() { "File".into() } else { format!("{} file", ext.to_uppercase()) }
}

pub fn guess_mime(name: &str) -> &'static str {
    match extension(name).as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "parquet" => "application/vnd.apache.parquet",
        _ => "application/octet-stream",
    }
}

/// Bucket names: 3–63 chars, lowercase letters, digits, dots and hyphens.
pub fn valid_bucket_name(name: &str) -> bool {
    let len = name.len();
    (3..=63).contains(&len)
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name.as_bytes()[len - 1].is_ascii_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn digit_groups() {
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(1050), "1,050");
        assert_eq!(group_digits(1234567), "1,234,567");
    }

    #[test]
    fn bucket_names() {
        assert!(valid_bucket_name("my-bucket.01"));
        assert!(!valid_bucket_name("My_Bucket"));
        assert!(!valid_bucket_name("ab"));
    }
}

pub fn extension_of(name: &str) -> String {
    extension(name)
}

pub fn is_video(name: &str) -> bool {
    matches!(extension(name).as_str(), "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v")
}

/// Label and colour for the extension badge drawn on file artwork.
pub fn badge(name: &str) -> Option<(String, u32)> {
    let ext = extension(name);
    if ext.is_empty() {
        return None;
    }
    let color = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "heic" | "heif" | "tif" | "tiff" | "avif" | "psd" => 0x8b5cf6,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" => 0xec4899,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" => 0x10b981,
        "zip" | "gz" | "tgz" | "tar" | "bz2" | "xz" | "7z" | "rar" | "zst" => 0xf59e0b,
        "pdf" => 0xef4444,
        "csv" | "xls" | "xlsx" | "parquet" | "tsv" | "ods" => 0x22c55e,
        "rs" | "js" | "ts" | "tsx" | "jsx" | "py" | "go" | "java" | "c" | "cpp" | "h" | "rb" | "sh" | "json" | "yaml" | "yml"
        | "toml" | "xml" | "html" | "css" | "sql" => 0xf97316,
        _ => 0x2599ff,
    };
    Some((ext.to_uppercase().chars().take(4).collect(), color))
}

/// 12345 -> "12,345"
pub fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Short label for a folder's size under its tile / in the list.
pub fn folder_size_label(size: Option<crate::app::FolderSize>) -> String {
    use crate::app::FolderSize;
    match size {
        Some(FolderSize::Done(s)) if s.complete => human_size(s.bytes),
        Some(FolderSize::Done(s)) => format!("{}+", human_size(s.bytes)),
        Some(FolderSize::Counting) => "…".into(),
        _ => String::new(),
    }
}
