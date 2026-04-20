mod common;

use anyhow::Context;
use common::TestHelper;
use rep_orchestrator_shared::{status::Status::*, summary::TestRunSummary};
use std::{
    io::{self, Read},
    time::Duration,
};
use zip::ZipArchive;

#[tokio::test]
async fn full_test_run_happy_path_completes_successfully() {
    let t = TestHelper::new();
    let run: TestRunSummary = t
        .json_post(
            "test-run/trigger",
            t.prepare_rep_payload("resources/test-plans/valid/minimal")
                .await
                .unwrap(),
        )
        .await
        .unwrap();

    // Wait for the run to enter a successful status
    let ex_id = t
        .poll_for_execution_id(run.id, Duration::from_secs(5))
        .await;
    t.poll_for_status(ex_id, Successful, Duration::from_secs(180))
        .await;

    // Fetch and validate the log output
    let log_txt = t
        .get_text(format!("test-execution/{ex_id}/log.txt"))
        .await
        .unwrap();

    assert!(log_txt.ends_with("hello, world!\n"), "{log_txt:?}");

    // Fetch and validate the output.zip
    let bytes = t
        .get_bytes(format!("test-execution/{ex_id}/output.zip"))
        .await
        .unwrap();

    let mut zip = ZipArchive::new(io::Cursor::new(bytes))
        .context("unable to parse zip file")
        .unwrap();

    let mut file_names = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).unwrap();
        file_names.push(file.name().to_string());

        if file.name().ends_with("RTF_OUTPUT") {
            let mut s = String::new();
            file.read_to_string(&mut s)
                .context("unable to read RTF_OUTPUT file")
                .unwrap();

            assert_eq!(s, "hello, world!\n", "unexpected RTF_OUTPUT content");
            return;
        }
    }

    panic!("no RTF_OUTPUT file in output.zip. Got: {file_names:#?}");
}
