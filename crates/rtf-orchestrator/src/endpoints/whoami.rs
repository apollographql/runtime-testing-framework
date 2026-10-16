use crate::{
    Result,
    state::{ServerState, UserType},
};
use axum::{Json, extract::State, http::HeaderMap};
use rtf_orchestrator_shared::whoami::WhoAmIResponse;

pub async fn handler(
    State(state): State<ServerState>,
    headers: HeaderMap,
) -> Result<Json<WhoAmIResponse>> {
    let user = state.identify_user(&headers).await?;

    Ok(Json(match user {
        UserType::Admin(email) => WhoAmIResponse::Admin { email },
        UserType::User(email) => WhoAmIResponse::User { email },
        UserType::Automation { email, org, repo } => {
            WhoAmIResponse::Automation { email, org, repo }
        }
        UserType::Unknown => WhoAmIResponse::Unknown,
    }))
}
