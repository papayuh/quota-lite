// Coverage must consider retained transcripts even when old files are excluded
// from usage accounting by their modification time.
#[path = "../src/config.rs"]
#[allow(dead_code)]
mod config;
#[path = "../src/date.rs"]
#[allow(dead_code)]
mod date;
#[path = "../src/logs.rs"]
#[allow(dead_code)]
mod logs;
#[path = "../src/types.rs"]
mod types;
#[test]
fn oldest_timestamp_includes_inactive_and_nonbillable_records() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("coverage-{}", std::process::id()));
    std::fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p/old.jsonl");
    std::fs::write(
        &path,
        "{\"timestamp\":\"2024-01-01T00:00:00Z\",\"type\":\"user\"}\n",
    )
    .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH)
        .unwrap();
    let scan = logs::read(
        &root,
        &config::Config::default(),
        date::parse("2024-02-01T00:00:00Z").unwrap(),
    );
    assert_eq!(scan.oldest_timestamp, date::parse("2024-01-01T00:00:00Z"));
    assert_eq!(scan.files, 0);
    assert!(scan.events.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
