use reqwest::Client;
use serde_json::Value;

const SERVER_URL: &str = "http://localhost:8035";

pub struct TestHelper {
    client: Client,
}

impl TestHelper {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn health(&self) -> anyhow::Result<Value> {
        Ok(self
            .client
            .get(format!("{SERVER_URL}/health"))
            .send()
            .await?
            .json()
            .await?)
    }
}
