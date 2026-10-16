use crate::orchestrator::{self, Client, OrchestratorClient};
use axum::{
    Form,
    extract::Path,
    response::{IntoResponse, Redirect, Response},
};
use rtf_orchestrator_shared::{
    known_test_plan::{KnownTestPlanSummary, Role, UpdateKnownTestPlanRequest},
    workload_config::WorkloadConfigPatch,
};
use std::collections::BTreeMap;
use tracing::error;
use url::form_urlencoded;
use uuid::Uuid;

#[derive(Debug, Default, serde::Deserialize)]
pub struct KnownTestPlanEditForm {
    name: String,
    description: String,
    org: String,
    repo: String,
    path: String,
    restrict_triggers: bool,
    restrict_automated_triggers: bool,
    allow_k8s_write: String,
    node_label_weights: String,
    trigger_users: String,
    #[serde(default)]
    owners: Option<String>,
    #[serde(default)]
    pinned_workload_pool: Option<String>,
    #[serde(default)]
    requires_dedicated_cluster: Option<bool>,
}

impl KnownTestPlanEditForm {
    /// When submitting the request to the Orchestrator we need to only submit fields that are
    /// actually changing, particularly for owners who will get a 403 if they attempt to set
    /// admin-only fields.
    fn diff(&self, plan: &KnownTestPlanSummary) -> Result<UpdateKnownTestPlanRequest, String> {
        let changed = |new: &str, old: &str| (new.trim() != old).then(|| new.trim().to_owned());

        let description = non_empty(&self.description);
        let patch = WorkloadConfigPatch {
            allow_k8s_write: match self.allow_k8s_write.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            node_label_weights: parse_node_label_weights(&self.node_label_weights)?,
        };

        Ok(UpdateKnownTestPlanRequest {
            name: changed(&self.name, &plan.name),
            description: (description != plan.description).then_some(description),
            org: changed(&self.org, &plan.org),
            repo: changed(&self.repo, &plan.repo),
            path: changed(&self.path, &plan.path),
            pinned_workload_pool: self
                .pinned_workload_pool
                .as_deref()
                .map(non_empty)
                .filter(|pool| *pool != plan.pinned_workload_pool),
            requires_dedicated_cluster: self
                .requires_dedicated_cluster
                .filter(|r| *r != plan.requires_dedicated_cluster),
            restrict_triggers: Some(self.restrict_triggers)
                .filter(|r| *r != plan.restrict_triggers),
            restrict_automated_triggers: Some(self.restrict_automated_triggers)
                .filter(|r| *r != plan.restrict_automated_triggers),
            associated_users: self.associated_users_diff(plan)?,
            workload_config_patch: Some(patch).filter(|p| *p != plan.workload_config_patch),
        })
    }

    fn associated_users_diff(
        &self,
        plan: &KnownTestPlanSummary,
    ) -> Result<Option<BTreeMap<String, Option<Role>>>, String> {
        let mut desired: BTreeMap<String, Role> = match &self.owners {
            Some(owners) => emails(owners).map(|e| (e, Role::Owner)).collect(),
            None => plan
                .associated_users
                .iter()
                .filter(|(_, role)| **role == Role::Owner)
                .map(|(email, role)| (email.clone(), *role))
                .collect(),
        };

        for email in emails(&self.trigger_users) {
            if desired.insert(email.clone(), Role::CanTrigger) == Some(Role::Owner) {
                return Err(format!(
                    "{email} is listed as both an owner and a trigger user"
                ));
            }
        }

        let mut diff: BTreeMap<String, Option<Role>> = plan
            .associated_users
            .keys()
            .filter(|email| !desired.contains_key(*email))
            .map(|email| (email.clone(), None))
            .collect();

        diff.extend(
            desired
                .into_iter()
                .filter(|(email, role)| plan.associated_users.get(email) != Some(role))
                .map(|(email, role)| (email, Some(role))),
        );

        Ok((!diff.is_empty()).then_some(diff))
    }
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

fn emails(s: &str) -> impl Iterator<Item = String> {
    s.split(['\n', ',']).filter_map(non_empty)
}

fn parse_node_label_weights(s: &str) -> Result<Option<Vec<(String, u32)>>, String> {
    let weights = s
        .lines()
        .filter_map(non_empty)
        .map(|line| {
            let (label, weight) = line.split_once('=').ok_or_else(|| {
                format!("Node label weights must be `label=weight`, got `{line}`")
            })?;
            let weight = weight.trim().parse().map_err(|_| {
                format!(
                    "Invalid weight for node label `{}`: `{weight}`",
                    label.trim()
                )
            })?;

            Ok((label.trim().to_owned(), weight))
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok((!weights.is_empty()).then_some(weights))
}

pub async fn post_edit<C: Client>(
    OrchestratorClient(client): OrchestratorClient<C>,
    Path(uuid): Path<Uuid>,
    Form(form): Form<KnownTestPlanEditForm>,
) -> Response {
    let detail_url = format!("/ui/test-plan/{uuid}");
    let plan = match client.known_test_plan_summary(uuid).await {
        Ok(Some(plan)) => plan,
        Ok(None) => return Redirect::to(&detail_url).into_response(),
        Err(error) => {
            error!(%error, %uuid, "failed to fetch known test plan from orchestrator");
            return redirect_with_edit_error(
                uuid,
                &format!("Unable to load the current test plan: {error}"),
            );
        }
    };

    let req = match form.diff(&plan) {
        Ok(req) => req,
        Err(message) => return redirect_with_edit_error(uuid, &message),
    };

    if req == UpdateKnownTestPlanRequest::default() {
        return Redirect::to(&detail_url).into_response();
    }

    match client.update_known_test_plan(uuid, &req).await {
        Ok(_) => Redirect::to(&detail_url).into_response(),
        Err(error) => {
            error!(%error, %uuid, "failed to update known test plan from the orchestrator");
            let message = match &error {
                orchestrator::Error::UpdateKnownTestPlan { message, .. } => message.clone(),
                e => format!("Unable to update test plan: {e}"),
            };
            redirect_with_edit_error(uuid, &message)
        }
    }
}

fn redirect_with_edit_error(uuid: Uuid, message: &str) -> Response {
    let mut qs = form_urlencoded::Serializer::new(String::new());
    qs.append_pair("edit_error", message);

    Redirect::to(&format!("/ui/test-plan/{uuid}?{}", qs.finish())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orchestrator::mocks::{MockClient, sample_known_test_plan},
        preview::{OWNER_EMAIL, TRIGGER_USER_EMAIL},
    };
    use axum::http::header::LOCATION;
    use reqwest::StatusCode;
    use rtf_orchestrator_shared::status::Status;

    fn unchanged_form(plan: &KnownTestPlanSummary) -> KnownTestPlanEditForm {
        KnownTestPlanEditForm {
            name: plan.name.clone(),
            description: plan.description.clone().unwrap_or_default(),
            org: plan.org.clone(),
            repo: plan.repo.clone(),
            path: plan.path.clone(),
            restrict_triggers: plan.restrict_triggers,
            restrict_automated_triggers: plan.restrict_automated_triggers,
            allow_k8s_write: "true".to_owned(),
            node_label_weights: "rtf.io/node-pool-a=3\nrtf.io/node-pool-b=1".to_owned(),
            trigger_users: TRIGGER_USER_EMAIL.to_owned(),
            owners: Some(OWNER_EMAIL.to_owned()),
            pinned_workload_pool: plan.pinned_workload_pool.clone(),
            requires_dedicated_cluster: Some(plan.requires_dedicated_cluster),
        }
    }

    #[test]
    fn diff_of_an_unchanged_form_is_empty() {
        let plan = sample_known_test_plan(Uuid::from_u128(1));

        assert_eq!(
            unchanged_form(&plan).diff(&plan),
            Ok(UpdateKnownTestPlanRequest::default())
        );
    }

    #[test]
    fn diff_only_sets_the_changed_fields() {
        let plan = sample_known_test_plan(Uuid::from_u128(1));
        let form = KnownTestPlanEditForm {
            description: String::new(),
            pinned_workload_pool: Some(String::new()),
            node_label_weights: "rtf.io/node-pool-a=3".to_owned(),
            trigger_users: "new@apollographql.com".to_owned(),
            owners: None,
            ..unchanged_form(&plan)
        };

        assert_eq!(
            form.diff(&plan),
            Ok(UpdateKnownTestPlanRequest {
                description: Some(None),
                pinned_workload_pool: Some(None),
                associated_users: Some(BTreeMap::from([
                    ("new@apollographql.com".to_owned(), Some(Role::CanTrigger)),
                    (TRIGGER_USER_EMAIL.to_owned(), None),
                ])),
                workload_config_patch: Some(WorkloadConfigPatch {
                    allow_k8s_write: Some(true),
                    node_label_weights: Some(vec![("rtf.io/node-pool-a".to_owned(), 3)]),
                }),
                ..Default::default()
            })
        );
    }

    #[test]
    fn diff_rejects_a_user_listed_as_both_an_owner_and_a_trigger_user() {
        let plan = sample_known_test_plan(Uuid::from_u128(1));
        let form = KnownTestPlanEditForm {
            trigger_users: OWNER_EMAIL.to_owned(),
            ..unchanged_form(&plan)
        };

        assert!(form.diff(&plan).is_err());
    }

    #[tokio::test]
    async fn post_edit_redirects_with_an_error_without_updating_when_the_form_is_invalid() {
        let uuid = Uuid::from_u128(1);
        let resp = post_edit(
            OrchestratorClient(MockClient::with_test_run(
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                Status::Running,
            )),
            Path(uuid),
            Form(KnownTestPlanEditForm {
                node_label_weights: "not-a-weight".to_owned(),
                ..unchanged_form(&sample_known_test_plan(uuid))
            }),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let location = resp.headers().get(LOCATION).unwrap().to_str().unwrap();
        assert!(location.starts_with(&format!("/ui/test-plan/{uuid}?edit_error=")));
    }
}
