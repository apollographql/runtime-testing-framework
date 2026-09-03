//! Admin routes are only accessible to users in the Orchestrator's "admins" configmap.
//!
//! See: <https://github.com/mdg-private/kanaveral/blob/main/management-cluster/orchestrator/admins.yaml>
use crate::{Result, endpoints::AdminUser, event_loop::Snapshot, state::ServerState};
use axum::{Extension, Json, extract::State, http::StatusCode};
use std::str::FromStr;
use tracing_subscriber::{EnvFilter, Registry, reload::Handle};

pub mod known_test_plan_cluster_pin;
pub mod purge_queue;
pub mod register_known_test_plan;

const RESET: &str = "reset";
const ENV_FILTER_DOCS: &str = "See here for docs on the logging filter format: https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives";

pub async fn get_logging_filter_handler() -> String {
    format!("{}\n\n", EnvFilter::from_default_env())
}

pub async fn set_logging_filter_handler(
    _admin: AdminUser,
    Extension(reload_handle): Extension<Handle<EnvFilter, Registry>>,
    body: String,
) -> (StatusCode, String) {
    let new_filter = if body == RESET {
        EnvFilter::from_default_env()
    } else {
        match EnvFilter::from_str(&body) {
            Ok(f) => f,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("invalid logging filter string: {e}\n\n{ENV_FILTER_DOCS}\n\n"),
                );
            }
        }
    };

    let filter_str = new_filter.to_string();

    match reload_handle.reload(new_filter) {
        Ok(_) => {
            let action = if body == RESET { "reset" } else { "updated" };

            (
                StatusCode::OK,
                format!(
                    "server logging filter {action} to {filter_str:?}\n\n{ENV_FILTER_DOCS}\n\n"
                ),
            )
        }

        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unable to set logging filter: {e}\n\n{ENV_FILTER_DOCS}\n\n"),
        ),
    }
}

pub async fn event_queue_snapshot_handler(
    _admin: AdminUser,
    State(ServerState { eq_state, .. }): State<ServerState>,
) -> Result<Json<Snapshot>> {
    let snapshot = eq_state.event_queue_snapshot().await;

    Ok(Json(snapshot))
}
