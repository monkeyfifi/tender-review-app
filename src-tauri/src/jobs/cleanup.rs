use crate::domain::job::{JobManifest, JobState};
use chrono::{DateTime, Duration, Utc};

pub(crate) fn is_expired(manifest: &JobManifest, now: DateTime<Utc>) -> bool {
    matches!(manifest.state, JobState::Failed | JobState::Cancelled)
        && now.signed_duration_since(manifest.updated_at) >= Duration::hours(24)
}
