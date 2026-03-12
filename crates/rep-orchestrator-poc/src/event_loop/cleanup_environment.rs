use crate::k8s::ClusterClients;
use tracing::info;
use uuid::Uuid;

pub async fn run(execution_id: Uuid, _clients: &ClusterClients) -> anyhow::Result<()> {
    info!(%execution_id, "would delete namespace");
    // info!(%execution_id, "deleting namespace");
    // clients
    //     .delete_workload_namespace(&execution_id.to_string())
    //     .await?;

    Ok(())
}
