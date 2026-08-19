mod execution_detail;
mod known_test_plan_list;
mod run;
mod run_list;
mod test_plan_details;

pub use execution_detail::ExecutionDetailView;
pub use known_test_plan_list::{KnownTestPlanListView, KnownTestPlanRowView};
pub use run::RunView;
pub use run_list::RunListView;
pub use test_plan_details::TestPlanDetailsView;

use chrono::{DateTime, Utc};
use humantime::format_rfc3339_seconds;
use std::time::{Duration, SystemTime};

/// Renders a timestamp as RFC 3339 at second precision (e.g. `2026-07-27T14:23:01Z`), the format
/// every timestamp on the UI is shown in.
fn format_rfc3339(dt: DateTime<Utc>) -> String {
    let seconds_since_epoch = dt.timestamp().max(0) as u64;
    let system_time = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds_since_epoch);

    format_rfc3339_seconds(system_time).to_string()
}
