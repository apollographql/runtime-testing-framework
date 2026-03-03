const BASE_URL: &str = "https://apollographql.github.io/runtime-testing-framework";

pub fn open_docs(search_term: &[String]) -> anyhow::Result<()> {
    let url = if search_term.is_empty() {
        BASE_URL.to_string()
    } else {
        format!("{BASE_URL}?search={}", search_term.join("+"))
    };

    open::that(url)?;

    Ok(())
}
