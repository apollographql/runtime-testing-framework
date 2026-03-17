mod common;

use common::TestHelper;

#[tokio::test]
async fn health_check_works() {
    let t = TestHelper::new();
    let res = t.health().await;

    assert!(res.is_ok(), "{res:?}");
}
