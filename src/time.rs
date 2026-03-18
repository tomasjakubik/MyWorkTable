use chrono::{DateTime, Utc};

pub fn relative_time(timestamp: &str) -> String {
    let dt = if let Ok(dt) = DateTime::parse_from_rfc3339(timestamp) {
        dt.with_timezone(&Utc)
    } else if let Ok(naive) =
        chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
    {
        naive.and_utc()
    } else {
        return timestamp.to_string();
    };

    let secs = Utc::now().signed_duration_since(dt).num_seconds();
    if secs < 10 {
        "now".to_string()
    } else if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}
