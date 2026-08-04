use crate::{
    Result,
    state::{ServerState, UserType},
};
use axum::{extract::State, http::HeaderMap};

pub async fn handler(State(state): State<ServerState>, headers: HeaderMap) -> Result<String> {
    let user = state.identify_user(&headers).await?;

    Ok(match user {
        UserType::Admin(email) => format!("admin: {email}"),
        UserType::User(email) => format!("user: {email}"),
        UserType::Unknown => "unknown".to_owned(),
    })
}
