//! `gutenberg --keyword <query>`: add a keyword alert and save what Google
//! News has for it now.

use std::process::ExitCode;

use gutenberg_core::{keyword, sync};
use gutenberg_sources::feed;
use gutenberg_sources::keyword::{tidy_google_news, Keyword};
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
    let url = keyword.google_news_url();
    let bytes = runtime
        .block_on(feed::fetch(&url))
        .map_err(|e| e.to_string())?;
    let mut found = feed::parse(&bytes, &url).map_err(|e| e.to_string())?;
    tidy_google_news(&mut found);

    let save = |store: &mut Store| -> gutenberg_store::Result<usize> {
        let folder_id = match folder {
            Some(name) => Some(store.folder_named(name)?.id),
            None => None,
        };
        let source = keyword::insert(store, &keyword, &keyword.query, folder_id)?;
        store.insert_articles(&sync::articles(&found, source.id, sync::unix_now()))
    };
    let added = save(&mut store).map_err(|e| e.to_string())?;
    let noun = if added == 1 { "article" } else { "articles" };
    let place = folder.map_or(String::new(), |name| format!(" to {name}"));
    Ok(format!(
        "Added keyword alert {}{place}: {added} {noun}",
        keyword.query
    ))
}
