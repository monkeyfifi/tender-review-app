mod fixtures;

use app_lib::{domain::job::JobState, error::ErrorCode, jobs::store::JobStore};
use chrono::{Duration, Utc};

#[test]
fn saves_and_recovers_manifest_without_secrets() {
    let temp = tempfile::tempdir().unwrap();
    let store = JobStore::new(temp.path().to_path_buf());
    let manifest = fixtures::manifest("job-1", JobState::Draft, Utc::now());

    store.save_atomic(&manifest).unwrap();

    let loaded = store.load("job-1").unwrap();
    assert_eq!(loaded.id, "job-1");
    let raw = std::fs::read_to_string(temp.path().join("job-1/manifest.json")).unwrap();
    assert!(!raw.contains("apiKey"));
    assert!(!temp.path().join("job-1/manifest.json.tmp").exists());
}

#[test]
fn preserves_a_corrupt_manifest_and_returns_a_stable_error() {
    let temp = tempfile::tempdir().unwrap();
    let store = JobStore::new(temp.path().to_path_buf());
    let job_dir = temp.path().join("corrupt-job");
    std::fs::create_dir_all(&job_dir).unwrap();
    std::fs::write(job_dir.join("manifest.json"), "not valid json").unwrap();

    let error = store.load("corrupt-job").unwrap_err();

    assert_eq!(error.code, ErrorCode::CorruptJobManifest);
    assert!(job_dir.exists());
    assert!(job_dir.join("manifest.json").exists());
}

#[test]
fn removes_only_failed_or_cancelled_jobs_expired_for_at_least_24_hours() {
    let temp = tempfile::tempdir().unwrap();
    let store = JobStore::new(temp.path().to_path_buf());
    let now = Utc::now();

    store
        .save_atomic(&fixtures::manifest(
            "expired-cancelled",
            JobState::Cancelled,
            now - Duration::hours(25),
        ))
        .unwrap();
    store
        .save_atomic(&fixtures::manifest(
            "recent-cancelled",
            JobState::Cancelled,
            now - Duration::hours(23),
        ))
        .unwrap();
    store
        .save_atomic(&fixtures::manifest(
            "old-preparing",
            JobState::Preparing,
            now - Duration::hours(25),
        ))
        .unwrap();

    store.remove_expired(now).unwrap();

    assert!(!temp.path().join("expired-cancelled").exists());
    assert!(temp.path().join("recent-cancelled").exists());
    assert!(temp.path().join("old-preparing").exists());
}

#[test]
fn rejects_traversal_ids_without_touching_adjacent_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("jobs");
    let sentinel = temp.path().join("sentinel");
    std::fs::create_dir_all(&sentinel).unwrap();
    std::fs::write(sentinel.join("keep.txt"), "keep").unwrap();
    let store = JobStore::new(root);

    let error = store
        .save_atomic(&fixtures::manifest(
            "../sentinel",
            JobState::Draft,
            Utc::now(),
        ))
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::InvalidJobId);
    assert!(!sentinel.join("manifest.json").exists());
    assert_eq!(
        std::fs::read_to_string(sentinel.join("keep.txt")).unwrap(),
        "keep"
    );

    let error = store.load("../sentinel").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidJobId);

    let error = store.remove("../sentinel").unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidJobId);
    assert!(sentinel.exists());
    assert_eq!(
        std::fs::read_to_string(sentinel.join("keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn rejects_empty_and_non_single_segment_job_ids() {
    let temp = tempfile::tempdir().unwrap();
    let store = JobStore::new(temp.path().join("jobs"));

    for job_id in ["", ".", "..", "nested/job", r"nested\job"] {
        let error = store
            .save_atomic(&fixtures::manifest(job_id, JobState::Draft, Utc::now()))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidJobId, "{job_id:?}");

        let error = store.load(job_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidJobId, "{job_id:?}");

        let error = store.remove(job_id).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidJobId, "{job_id:?}");
    }
}

#[test]
fn cleanup_preserves_a_tampered_manifest_id_without_blocking_startup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("jobs");
    let sentinel = temp.path().join("sentinel");
    let job_directory = root.join("job-1");
    std::fs::create_dir_all(&sentinel).unwrap();
    std::fs::write(sentinel.join("keep.txt"), "keep").unwrap();
    std::fs::create_dir_all(&job_directory).unwrap();
    let now = Utc::now();
    let tampered = fixtures::manifest(
        "../sentinel",
        JobState::Cancelled,
        now - Duration::hours(25),
    );
    std::fs::write(
        job_directory.join("manifest.json"),
        serde_json::to_vec(&tampered).unwrap(),
    )
    .unwrap();
    let store = JobStore::new(root);

    store.remove_expired(now).unwrap();

    assert!(job_directory.exists());
    assert!(sentinel.exists());
    assert_eq!(
        std::fs::read_to_string(sentinel.join("keep.txt")).unwrap(),
        "keep"
    );
}
