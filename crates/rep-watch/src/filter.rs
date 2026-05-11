use ad_client::{
    EventData, EventOutcome,
    tokio::{AsyncEventFilter, Client as AdClient},
};
use reqwest::Client;
use uuid::Uuid;

#[derive(Default)]
pub struct Filter {
    http_client: Client,
}

impl Filter {
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
            .http_client
            .get(format!("http://localhost:8035/{kind}/{id}/status"))
            .send()
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
            .http_client
            .get(format!("http://localhost:8035/test-execution/{id}/log.txt"))
            .send()
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
