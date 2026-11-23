use std::{net::Ipv4Addr, path::PathBuf, sync::Arc};

use anyhow::Result;
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use clap::Parser;
use site::{Rendered, Site, markdown::Highlighter, path::content_type};
use tower_http::services::ServeFile;

#[derive(Parser)]
#[command(about = "Preview the site, drafts included; every request renders from disk")]
struct Cli {
    /// Directory that holds hero.toml, posts/, media/ and static/
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, default_value_t = 3000)]
    port: u16,
}

struct Preview {
    root: PathBuf,
    highlighter: Highlighter,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let preview = Arc::new(Preview {
        root: cli.root,
        highlighter: Highlighter::default(),
    });
    let app = Router::new().fallback(page).with_state(preview);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, cli.port)).await?;
    println!("http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

enum Found {
    Page(String, Vec<u8>),
    /// A photo or video a post shows, straight from media/.
    Media(PathBuf),
    Nothing,
}

async fn page(State(preview): State<Arc<Preview>>, request: Request) -> Response {
    let path = request.uri().path().to_owned();
    let found = tokio::task::spawn_blocking(move || find(&preview, &path)).await;

    let mut response = match found {
        Ok(Ok(Found::Page(key, body))) => {
            ([(header::CONTENT_TYPE, content_type(&key))], body).into_response()
        }
        // answers range requests too, safari plays no video without them
        Ok(Ok(Found::Media(file))) => match ServeFile::new(file).try_call(request).await {
            Ok(response) => response.map(Body::new),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        },
        Ok(Ok(Found::Nothing)) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Ok(Err(error)) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{error:#}")).into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn find(preview: &Preview, path: &str) -> Result<Found> {
    let Rendered { mut pages, media } =
        Site::load(&preview.root, true)?.render(&preview.highlighter)?;
    if let Some(key) = find_key(path, |key| pages.contains_key(key))
        && let Some((key, body)) = pages.remove_entry(&key)
    {
        return Ok(Found::Page(key, body));
    }
    // only what a post names, so a path can't leave media/
    match path.strip_prefix('/') {
        Some(key) if media.contains(key) => Ok(Found::Media(preview.root.join("media").join(key))),
        _ => Ok(Found::Nothing),
    }
}

/// The file of a url path: / is index.html, and /numa is numa.html.
fn find_key(path: &str, exists: impl Fn(&str) -> bool) -> Option<String> {
    let name = match path.strip_prefix('/')? {
        "" => "index.html",
        name => name,
    };
    [name.to_owned(), format!("{name}.html")]
        .into_iter()
        .find(|key| exists(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_pages_without_the_extension() {
        let exists = |key: &str| ["index.html", "numa.html", "static/style.css"].contains(&key);
        assert_eq!(find_key("/", exists).unwrap(), "index.html");
        assert_eq!(find_key("/numa", exists).unwrap(), "numa.html");
        assert_eq!(
            find_key("/static/style.css", exists).unwrap(),
            "static/style.css"
        );
        for path in ["/missing", "/static", "//numa", "/../hero.toml"] {
            assert_eq!(find_key(path, exists), None, "{path}");
        }
    }
}
