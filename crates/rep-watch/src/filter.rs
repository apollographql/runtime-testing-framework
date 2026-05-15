use std::str::FromStr;

use ad_client::{
    EventData, EventOutcome,
    tokio::{AsyncEventFilter, Client as AdClient},
};
use reqwest::{Method, Request, Url};
use rtf_integrations::orchestrator::{DEFAULT_ORCHESTRATOR_URL, OrchestratorClient};
use uuid::Uuid;

pub struct Filter {
    orchestrator_client: OrchestratorClient,
    base: Url,
}

impl Filter {
    pub async fn try_new() -> anyhow::Result<Self> {
        let orchestrator_client = OrchestratorClient::new().await?;

        Ok(Self {
            orchestrator_client,
            base: Url::from_str(DEFAULT_ORCHESTRATOR_URL).unwrap(),
        })
    }

    async fn try_show_summary(
        &self,
        kind: &str,
        s: &str,
        ad_client: &AdClient,
    ) -> anyhow::Result<EventOutcome> {
        let id = match Uuid::try_parse(s) {
            Ok(id) => id,
            Err(_) => return Ok(EventOutcome::Passthrough),
        };

        let data: serde_json::Value = self
            .orchestrator_client
            .send(Request::new(
                Method::GET,
                self.base.join(&format!("{kind}/{id}/status"))?,
            ))
            .await?
            .json()
            .await?;

        let s = serde_json::to_string_pretty(&data)?;
        ad_client
            .open_virtual_in_new_window(format!("+{kind}-summary/{id}.json"), s)
            .await?;

        Ok(EventOutcome::Handled)
    }

    async fn try_show_log(&self, s: &str, ad_client: &AdClient) -> anyhow::Result<EventOutcome> {
        let id = match Uuid::try_parse(s) {
            Ok(id) => id,
            Err(_) => return Ok(EventOutcome::Passthrough),
        };

        let txt = self
            .orchestrator_client
            .send(Request::new(
                Method::GET,
                self.base.join(&format!("test-execution/{id}/log.txt"))?,
            ))
            .await?
            .text()
            .await?;

        ad_client
            .open_virtual_in_new_window(format!("+test-execution-log/{id}"), txt)
            .await?;

        Ok(EventOutcome::Handled)
    }
}

impl AsyncEventFilter for Filter {
    async fn on_load(
        &mut self,
        data: EventData<'_>,
        client: &AdClient,
    ) -> ad_client::Result<EventOutcome> {
        if let Some(s) = data.txt.strip_prefix("TR-") {
            return self
                .try_show_summary("test-run", s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                });
        } else if let Some(s) = data.txt.strip_prefix("EX-") {
            return self
                .try_show_summary("test-execution", s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                });
        }

        Ok(EventOutcome::Passthrough)
    }

    async fn on_execute(
        &mut self,
        data: EventData<'_>,
        _chorded_arg: Option<EventData<'_>>,
        client: &AdClient,
    ) -> ad_client::Result<EventOutcome> {
        match data.txt.strip_prefix("EX-") {
            Some(s) => self
                .try_show_log(s, client)
                .await
                .map_err(|e| ad_client::Error::Rerror {
                    ename: e.to_string(),
                }),
            None => Ok(EventOutcome::Passthrough),
        }
    }
}
