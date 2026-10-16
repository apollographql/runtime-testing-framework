use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WhoAmIResponse {
    Admin {
        email: String,
    },
    User {
        email: String,
    },
    Automation {
        email: String,
        org: String,
        repo: String,
    },
    Unknown,
}
