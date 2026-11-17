use chrono::{DateTime, Local};
use std::time::SystemTime;

pub fn format_size(size: Option<u64>) -> String {
    let Some(n) = size else {
        return "--".into();
    };
    if n == 0 {
        return "Zero bytes".into();
    }
    if n < 1000 {
        return format!("{n} bytes");
    }
    let units = ["KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = "bytes";
    for unit in units {
        v /= 1000.0;
        u = unit;
        if v < 1000.0 {
            break;
        }
    }
    let s = if v >= 100.0 {
        format!("{v:.0}")
    } else {
        let t = format!("{v:.1}");
        t.strip_suffix(".0").map(str::to_string).unwrap_or(t)
    };
    format!("{s} {u}")
}

pub fn format_modified(t: Option<SystemTime>, now: SystemTime) -> String {
    let Some(t) = t else {
        return "--".into();
    };
    let dt: DateTime<Local> = t.into();
    let now: DateTime<Local> = now.into();
    let time = dt.format("%H:%M");
    let d = dt.date_naive();
    let today = now.date_naive();
    if d == today {
        format!("Today at {time}")
    } else if today.pred_opt() == Some(d) {
        format!("Yesterday at {time}")
    } else {
        format!("{} at {time}", dt.format("%-d %b %Y"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn st(y: i32, m: u32, d: u32, h: u32, mi: u32) -> SystemTime {
        Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().into()
    }

    #[test]
    fn sizes() {
        assert_eq!(format_size(None), "--");
        assert_eq!(format_size(Some(0)), "Zero bytes");
        assert_eq!(format_size(Some(999)), "999 bytes");
        assert_eq!(format_size(Some(1000)), "1 KB");
        assert_eq!(format_size(Some(1_500_000)), "1.5 MB");
    }

    #[test]
    fn dates() {
        let now = st(2026, 11, 17, 15, 0);
        assert_eq!(format_modified(Some(st(2026, 11, 17, 14, 3)), now), "Today at 14:03");
        assert_eq!(format_modified(Some(st(2026, 11, 16, 9, 5)), now), "Yesterday at 09:05");
        assert_eq!(format_modified(Some(st(2026, 11, 1, 9, 12)), now), "1 Nov 2026 at 09:12");
        assert_eq!(format_modified(None, now), "--");
    }
}
