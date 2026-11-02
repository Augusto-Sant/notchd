use super::*;
use notchd_proto::Validate;

#[test]
fn defaults_are_the_docs_numbers() {
    let o = Options::for_dir(Path::new("/x/modules"));
    assert_eq!(o.backoff_base, Duration::from_secs(1));
    assert_eq!(o.backoff_max, Duration::from_secs(30));
    assert_eq!(o.reset_after, Duration::from_secs(60));
    assert_eq!(o.first_view_timeout, Duration::from_secs(5));
    assert_eq!(o.term_after, Duration::from_secs(1));
    assert_eq!(o.kill_after, Duration::from_secs(3));
    assert_eq!(o.max_views_per_second, 20);
    assert_eq!(o.log_max_bytes, 1 << 20);
    assert_eq!(o.log_keep, 5);
    assert_eq!(o.bad_lines_limit, 10);
}

#[test]
fn backoff_doubles_and_caps() {
    let o = Options::for_dir(Path::new("/x/modules"));
    let secs: Vec<u64> = (1..=8).map(|n| backoff_delay(&o, n).as_secs()).collect();
    assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(backoff_delay(&o, u32::MAX).as_secs(), 30);
}

#[test]
fn the_error_row_is_valid_and_carries_text_and_path() {
    let row = error_row("weather exited (1)", Path::new("/logs/weather.log"));
    assert!(row.validate().is_ok());
    let json = serde_json::to_string(&row).unwrap();
    assert!(
        json.contains("weather exited (1)  /logs/weather.log"),
        "{json}"
    );
    assert!(json.contains("\"action\":\"restart\""), "{json}");
    assert!(json.contains("\"tone\":\"danger\""), "{json}");
}
