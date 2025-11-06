use crate::harness::make_request;

mod harness;

#[tokio::test]
async fn test_default_config_processes_request() -> anyhow::Result<()> {
    harness::initialize(None)?;
    make_request(42, None).await?;
    Ok(())
}
