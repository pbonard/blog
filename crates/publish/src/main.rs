use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use aws_sdk_s3::{
    Client,
    config::{BehaviorVersion, Credentials, Region},
    error::{ProvideErrorMetadata, SdkError},
    primitives::ByteStream,
    types::{Delete, ObjectIdentifier},
};
use clap::Parser;
use md5::{Digest, Md5};
use serde::Deserialize;
use site::{Pages, Site, markdown::Highlighter, path::content_type};

/// Render the posts and upload what changed to garage, where the blog is served from
///
/// Drafts and their photos stay on this computer. The key is read from upload.toml next to this crate's Cargo.toml,
/// which git ignores. It's written once from the cluster's secret garage-blog, in the namespace garage.
// the bucket, its key and its route are in my-infra k8s/bootstrap/garage.yaml
#[derive(Parser)]
struct Cli {
    /// Directory that holds hero.toml, posts/, media/ and static/
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Only print what would be done
    #[arg(long)]
    dry_run: bool,
    /// Take pages offline whose post is gone
    #[arg(long)]
    delete: bool,
    /// Garage's s3 api
    #[arg(long, default_value = "https://s3.pbonard.com")]
    endpoint: String,
    #[arg(long, default_value = "garage")]
    region: String,
    #[arg(long, default_value = "blog")]
    bucket: String,
}

const KEY_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/upload.toml");

const YEAR: &str = "public, max-age=31536000, immutable";
const HOUR: &str = "public, max-age=3600";
/// Asked for again each time, and answered with 304 when nothing changed.
const CHECK: &str = "no-cache";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    access_key_id: String,
    secret_access_key: String,
}

struct Object {
    key: String,
    body: Vec<u8>,
    content_type: &'static str,
    cache_control: &'static str,
}

/// The etag of every object online, the md5 of its content.
type Online = HashMap<String, String>;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let site = Site::load(&cli.root, false)?;
    // an empty posts/ is a mistake more often than a wish
    ensure!(!site.posts.is_empty(), "there's no published post");
    let rendered = site.render(&Highlighter::default())?;

    let client = client(&cli, read_credentials(Path::new(KEY_FILE))?);
    let online = list(&client, &cli.bucket).await?;

    // everything is checked before anything is uploaded
    let media = new_media(&cli.root.join("media"), &rendered.media, &online)?;
    let objects = objects(rendered.pages);
    let wanted: BTreeSet<_> = objects.iter().map(|object| object.key.clone()).collect();
    let stale = stale(&online, &wanted);

    // media first and the home page last, so no page ever names something that isn't online
    let changed = media.into_iter().chain(
        objects
            .into_iter()
            .filter(|object| online.get(&object.key) != Some(&etag(&object.body))),
    );
    let mut count = 0;
    for object in changed {
        println!("upload {}", object.key);
        count += 1;
        if !cli.dry_run {
            let key = object.key.clone();
            upload(&client, &cli.bucket, object)
                .await
                .with_context(|| format!("uploading {key}"))?;
        }
    }
    if !cli.delete {
        for key in &stale {
            println!("still online without a post: {key}, take it offline with --delete");
        }
    } else if !stale.is_empty() {
        for key in &stale {
            println!("delete {key}");
        }
        if !cli.dry_run {
            delete(&client, &cli.bucket, &stale).await?;
        }
    }
    println!("{count} changed, {} posts", site.posts.len());
    Ok(())
}

/// The pages as they are named online: numa.html is numa, the address of a post.
fn objects(pages: Pages) -> Vec<Object> {
    let mut objects: Vec<_> = pages
        .into_iter()
        .map(|(path, body)| {
            let key = match path.strip_suffix(".html") {
                Some(slug) if !["index", "404"].contains(&slug) && !path.starts_with("static/") => {
                    slug.to_owned()
                }
                _ => path.clone(),
            };
            let cache_control = match key.as_str() {
                // linked with a hash of its content
                "static/style.css" => YEAR,
                key if key.starts_with("static/") => HOUR,
                _ => CHECK,
            };
            Object {
                content_type: content_type(&path),
                cache_control,
                key,
                body,
            }
        })
        .collect();
    objects.sort_by_key(|object| match object.key.as_str() {
        key if key.starts_with("static/") => 0,
        "index.html" => 2,
        _ => 1,
    });
    objects
}

/// The photos and videos that aren't online yet.
fn new_media(dir: &Path, media: &BTreeSet<String>, online: &Online) -> Result<Vec<Object>> {
    let mut new = Vec::new();
    let mut problems = Vec::new();
    for key in media {
        let file = dir.join(key);
        if !file.is_file() {
            if !online.contains_key(key) {
                problems.push(format!("media/{key} is missing"));
            }
            continue;
        }
        let body = fs::read(&file).with_context(|| file.display().to_string())?;
        match online.get(key) {
            None => new.push(Object {
                content_type: content_type(key),
                cache_control: YEAR,
                key: key.clone(),
                body,
            }),
            // browsers keep a file for a year, so its content must never change under the same url
            Some(online) if *online != etag(&body) => {
                problems.push(format!("media/{key} changed, use another name"));
            }
            Some(_) => {}
        }
    }
    ensure!(problems.is_empty(), "{}", problems.join("\n"));
    Ok(new)
}

/// The pages and static files online that the site no longer has. Photos and videos, <slug>/<file>, are kept.
fn stale(online: &Online, wanted: &BTreeSet<String>) -> BTreeSet<String> {
    online
        .keys()
        .filter(|key| !key.contains('/') || key.starts_with("static/"))
        .filter(|key| !wanted.contains(*key))
        .cloned()
        .collect()
}

fn etag(body: &[u8]) -> String {
    Md5::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn list(client: &Client, bucket: &str) -> Result<Online> {
    let mut online = Online::new();
    let mut objects = client
        .list_objects_v2()
        .bucket(bucket)
        .into_paginator()
        .send();
    while let Some(page) = objects.next().await {
        let page = page.map_err(|error| anyhow!("listing {bucket}: {}", describe(&error)))?;
        for object in page.contents() {
            if let (Some(key), Some(etag)) = (object.key(), object.e_tag()) {
                online.insert(key.to_owned(), etag.trim_matches('"').to_owned());
            }
        }
    }
    Ok(online)
}

async fn upload(client: &Client, bucket: &str, object: Object) -> Result<()> {
    client
        .put_object()
        .bucket(bucket)
        .key(object.key)
        .content_type(object.content_type)
        .cache_control(object.cache_control)
        // whole, so the signature covers the content. garage rejects the signature of the sdk's streamed files
        .body(ByteStream::from(object.body))
        .send()
        .await
        .map_err(|error| anyhow!("{}", describe(&error)))?;
    Ok(())
}

/// With one POST: s3.pbonard.com takes no DELETE.
async fn delete(client: &Client, bucket: &str, keys: &BTreeSet<String>) -> Result<()> {
    let objects = keys
        .iter()
        .map(|key| ObjectIdentifier::builder().key(key).build())
        .collect::<Result<Vec<_>, _>>()?;
    let deleted = client
        .delete_objects()
        .bucket(bucket)
        .delete(Delete::builder().set_objects(Some(objects)).build()?)
        .send()
        .await
        .map_err(|error| anyhow!("deleting: {}", describe(&error)))?;
    if let Some(error) = deleted.errors().first() {
        bail!(
            "deleting {}: {}",
            error.key().unwrap_or("?"),
            error.message().unwrap_or("no message")
        );
    }
    Ok(())
}

/// What garage answered, or why it couldn't be reached.
fn describe<E>(error: &SdkError<E>) -> String
where
    E: ProvideErrorMetadata + std::error::Error + 'static,
{
    match error {
        SdkError::ServiceError(service) => format!(
            "garage answered {}: {}",
            service.raw().status(),
            match service.err().message() {
                Some(message) => message,
                None if service.raw().status().as_u16() == 403 => "is the key valid?",
                None => "no message",
            }
        ),
        error => {
            let error: &dyn std::error::Error = error;
            std::iter::successors(Some(error), |error| error.source())
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join(": ")
        }
    }
}

fn client(cli: &Cli, credentials: Credentials) -> Client {
    // nothing is read from ~/.aws or the AWS_ environment, so the keys of an aws profile are never used
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .endpoint_url(&cli.endpoint)
        .region(Region::new(cli.region.clone()))
        .credentials_provider(credentials)
        .force_path_style(true)
        .build();
    Client::from_conf(config)
}

// not the AWS_ environment or ~/.aws: those keys belong to aws and must not reach garage
fn read_credentials(path: &Path) -> Result<Credentials> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("reading the key in {}", path.display()))?;
    // only the message: the whole error quotes the line, which can be the secret
    let key: Key = toml::from_str(&text)
        .map_err(|error| anyhow!("{}: {}", path.display(), error.message()))?;
    ensure!(
        !key.access_key_id.is_empty() && !key.secret_access_key.is_empty(),
        "{}: the key is empty",
        path.display()
    );
    Ok(Credentials::new(
        key.access_key_id,
        key.secret_access_key,
        None,
        None,
        "garage",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    fn online(objects: &[(&str, &str)]) -> Online {
        objects
            .iter()
            .map(|(key, body)| (key.to_string(), etag(body.as_bytes())))
            .collect()
    }

    #[test]
    fn pages_are_named_by_their_address_and_the_home_page_is_last() {
        let pages = [
            "404.html",
            "index.html",
            "numa.html",
            "static/style.css",
            "static/a.html",
            "static/favicon.ico",
        ];
        let objects = objects(
            pages
                .iter()
                .map(|page| (page.to_string(), Vec::new()))
                .collect(),
        );
        let keys: Vec<_> = objects.iter().map(|object| object.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "static/a.html",
                "static/favicon.ico",
                "static/style.css",
                "404.html",
                "numa",
                "index.html"
            ]
        );
        assert!(
            objects
                .iter()
                .filter(|o| ["numa", "index.html"].contains(&o.key.as_str()))
                .all(|o| o.content_type.starts_with("text/html") && o.cache_control == CHECK)
        );
        assert!(
            objects
                .iter()
                .any(|o| o.key == "static/style.css" && o.cache_control == YEAR)
        );
    }

    #[test]
    fn only_new_media_is_uploaded() {
        let dir = std::env::temp_dir().join(format!("blog-publish-{}", std::process::id()));
        fs::create_dir_all(dir.join("numa")).unwrap();
        for name in ["new.png", "same.png", "unused.png"] {
            fs::write(dir.join("numa").join(name), "x").unwrap();
        }

        let online = online(&[("numa/same.png", "x"), ("numa/only-online.png", "y")]);
        let wanted = set(&["numa/new.png", "numa/same.png", "numa/only-online.png"]);
        let new = new_media(&dir, &wanted, &online).unwrap();
        assert_eq!(
            new.iter()
                .map(|object| object.key.as_str())
                .collect::<Vec<_>>(),
            ["numa/new.png"]
        );

        fs::write(dir.join("numa/same.png"), "edited").unwrap();
        let wanted = set(&["numa/same.png", "numa/nowhere.png"]);
        let error = new_media(&dir, &wanted, &online).err().unwrap().to_string();
        assert!(
            error.contains("same.png changed") && error.contains("nowhere.png is missing"),
            "{error}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_pages_go_stale() {
        let online = online(&[
            ("numa", ""),
            ("old", ""),
            ("static/old.ico", ""),
            ("old/photo.png", ""),
            ("index.html", ""),
        ]);
        assert_eq!(
            stale(&online, &set(&["numa", "index.html"])),
            set(&["old", "static/old.ico"])
        );
    }

    #[test]
    fn reads_the_key_without_printing_it() {
        let path =
            std::env::temp_dir().join(format!("blog-upload-key-{}.toml", std::process::id()));
        let read = |text: &str| {
            fs::write(&path, text).unwrap();
            read_credentials(&path)
        };

        let key = read("access_key_id = \"GK1\"\nsecret_access_key = \"topsecret\"\n").unwrap();
        assert_eq!(key.access_key_id(), "GK1");
        assert_eq!(key.secret_access_key(), "topsecret");
        for text in [
            "",
            "access_key_id = \"\"\nsecret_access_key = \"\"\n",
            "access_key_id = \"GK1\"\nsecret_access_key = topsecret\n",
            "access_key_id = \"GK1\"\nsecret_key = \"topsecret\"\n",
        ] {
            let error = format!("{:#}", read(text).unwrap_err());
            assert!(!error.contains("topsecret"), "{error}");
        }
        fs::remove_file(&path).unwrap();
        assert!(read_credentials(&path).is_err());
    }
}
