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

struct Pagination {
    total: i64,
    limit: i64,
    offset: i64,
    n_rows: usize,
}

impl Pagination {
    fn has_prev(&self) -> bool {
        self.offset > 0
    }

    fn has_next(&self) -> bool {
        self.offset + (self.n_rows as i64) < self.total
    }

    fn prev_offset(&self) -> i64 {
        if self.n_rows == 0 && self.total > 0 {
            ((self.total - 1) / self.limit) * self.limit
        } else {
            (self.offset - self.limit).max(0)
        }
    }

    fn next_offset(&self) -> i64 {
        self.offset + self.limit
    }

    fn showing_range(&self) -> Option<String> {
        if self.n_rows == 0 {
            return None;
        }
        let last = self.offset + self.n_rows as i64;
        Some(format!("{}-{} of {}", self.offset + 1, last, self.total))
    }
}

#[cfg(test)]
mod tests {
    use super::Pagination;
    use simple_test_case::test_case;

    #[test_case(20, 137, 20, 0, true, false; "more rows remain and first page")]
    #[test_case(20, 20, 20, 0, false, false; "last full page")]
    #[test_case(0, 15, 20, 40, false, true; "offset landed past the end")]
    #[test]
    fn has_next_and_has_prev(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected_has_next: bool,
        expected_has_prev: bool,
    ) {
        let pagination = Pagination {
            total,
            limit,
            offset,
            n_rows,
        };
        assert_eq!(pagination.has_next(), expected_has_next);
        assert_eq!(pagination.has_prev(), expected_has_prev);
    }

    #[test_case(20, 137, 20, 40, 20; "steps back by limit normally")]
    // total=137, limit=20 -> last real page starts at offset 120 (rows 121-137).
    #[test_case(0, 137, 20, 500, 120; "jumps to the last real page when offset overshot")]
    #[test]
    fn prev_offset_cases(n_rows: usize, total: i64, limit: i64, offset: i64, expected: i64) {
        let pagination = Pagination {
            total,
            limit,
            offset,
            n_rows,
        };
        assert_eq!(pagination.prev_offset(), expected);
    }

    #[test]
    fn next_offset_steps_forward_by_limit() {
        let pagination = Pagination {
            total: 137,
            limit: 20,
            offset: 40,
            n_rows: 20,
        };
        assert_eq!(pagination.next_offset(), 60);
    }

    #[test_case(0, 0, 20, 0, None; "no matches at all")]
    #[test_case(0, 137, 20, 500, None; "offset landed past the end")]
    #[test_case(17, 137, 20, 120, Some("121-137 of 137"); "formats the current page")]
    #[test]
    fn showing_range_cases(
        n_rows: usize,
        total: i64,
        limit: i64,
        offset: i64,
        expected: Option<&str>,
    ) {
        let pagination = Pagination {
            total,
            limit,
            offset,
            n_rows,
        };
        assert_eq!(pagination.showing_range(), expected.map(str::to_owned));
    }
}
