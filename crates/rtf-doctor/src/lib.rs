use rtf_core::{APOLLO_KEY_ENV_VAR, GITHUB_TOKEN_ENV_VAR, ReqwestClient};
use std::{collections::HashMap, env};

pub mod cli;
pub mod commands;

/// The environment variable to set to control logging within the rtf CLI
pub const LOG_LEVEL_ENV_VAR: &str = "APOLLO_RTF_LOG";

pub(crate) fn new_client() -> ReqwestClient {
    let mut client = ReqwestClient::new();

    let mut env_vars: HashMap<String, String> = env::vars_os()
        .map(|(k, v)| {
            (
                k.to_string_lossy().to_string(),
                v.to_string_lossy().to_string(),
            )
        })
        .collect();

    if let Some(api_key) = env_vars.remove(APOLLO_KEY_ENV_VAR) {
        let staging = false;
        let sudo = true;

        client.with_platform_config(api_key, staging, sudo);
    }

    if let Some(api_token) = env_vars.remove(GITHUB_TOKEN_ENV_VAR) {
        client.with_github_config(api_token);
    }

    client
}

pub(crate) fn format_bytes(n: usize) -> String {
    let suffix;
    let mut f = n as f64;

    if n >= 1024 * 1024 * 1024 {
        f /= (1024 * 1024 * 1024) as f64;
        suffix = "G";
    } else if n >= 1024 * 1024 {
        f /= (1024 * 1024) as f64;
        suffix = "M";
    } else if n >= 1024 {
        f /= 1024.0;
        suffix = "k";
    } else {
        return n.to_string();
    }

    format!("{f:.2}{suffix}")
}
