//! `gutenberg --keyword <query>`: add a keyword alert and save what the
//! news searches have for it now.

use std::process::ExitCode;

use gutenberg_core::{keyword, sync};
use gutenberg_sources::keyword::Keyword;
use gutenberg_store::Store;

pub fn run(query: &str, folder: Option<&str>) -> ExitCode {
    match add(query, folder) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("gutenberg: {query}: {message}");
            ExitCode::FAILURE
        }
    }
}

fn add(query: &str, folder: Option<&str>) -> Result<String, String> {
    let keyword = Keyword::new(query).ok_or("the query is empty")?;
    let path = Store::default_path().map_err(|e| e.to_string())?;
    let mut store = Store::open(&path).map_err(|e| e.to_string())?;
    if keyword::queries(&store)
        .map_err(|e| e.to_string())?
        .contains(&keyword.query)
    {
        return Err("already added".to_owned());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let folder_id = match folder {
        Some(name) => Some(store.folder_named(name).map_err(|e| e.to_string())?.id),
        None => None,
    };
    let source =
        keyword::insert(&store, &keyword, &keyword.query, folder_id).map_err(|e| e.to_string())?;
    let summary = runtime
        .block_on(sync::refresh_source(
            &mut store,
            source.id,
            sync::unix_now(),
            &sync::Limits::default(),
        ))
        .map_err(|e| e.to_string())?;
    // A failed search leaves the alert to try again on the next refresh.
    if let Some((_, error)) = summary.failed.first() {
        return Ok(format!(
            "Added keyword alert {}; no results yet ({error})",
            keyword.query
        ));
    }
    let added = summary.new_articles;
    let noun = if added == 1 { "article" } else { "articles" };
    let place = folder.map_or(String::new(), |name| format!(" to {name}"));
    Ok(format!(
        "Added keyword alert {}{place}: {added} {noun}",
        keyword.query
    ))
}
