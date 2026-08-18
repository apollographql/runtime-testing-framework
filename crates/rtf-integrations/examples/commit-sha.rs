//! To run this example you will need to create a "classic" personal access token.
//! See https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#personal-access-tokens-classic
//! for details on how this works.
use rtf_integrations::github::{Client, GithubClient};
use std::env;

#[tokio::main]
async fn main() {
    let token = env::var("GITHUB_TOKEN").unwrap();
    let client = GithubClient::new(token);
    let git_ref: Option<&str> = None;

    let org = "apollographql";
    let repo = "runtime-testing-framework";

    let sha = client
        .commit_sha(org, repo, git_ref)
        .await
        .expect("to be able to pull HEAD sha");

    println!("HEAD sha:\n{sha}");
}
