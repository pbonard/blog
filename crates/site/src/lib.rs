pub mod markdown;
pub mod path;
pub mod post;

use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    fs,
    hash::{DefaultHasher, Hasher},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use askama::Template;
use serde::Deserialize;

use crate::{markdown::Highlighter, post::Post};

pub type Pages = BTreeMap<String, Vec<u8>>;

pub struct Rendered {
    pub pages: Pages,
    pub media: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    title: String,
    author: String,
    intro: String,
}

pub struct Site {
    config: Config,
    pub posts: Vec<Post>,
    statics: Pages,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexPage<'a> {
    config: &'a Config,
    css: &'a str,
    posts: &'a [Post],
}

#[derive(Template)]
#[template(path = "post.html")]
struct PostPage<'a> {
    config: &'a Config,
    css: &'a str,
    post: &'a Post,
    body: &'a str,
}

#[derive(Template)]
#[template(path = "404.html")]
struct NotFoundPage<'a> {
    config: &'a Config,
    css: &'a str,
}

impl Site {
    pub fn load(root: &Path, drafts: bool) -> Result<Self> {
        let path = root.join("hero.toml");
        let config =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let config =
            toml::from_str(&config).with_context(|| format!("parsing {}", path.display()))?;

        let mut posts = Vec::new();
        for entry in fs::read_dir(root.join("posts")).context("reading posts/")? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "md") {
                let post = Post::load(&path).with_context(|| path.display().to_string())?;
                if drafts || !post.draft {
                    posts.push(post);
                }
            }
        }
        posts.sort_by(|a, b| (Reverse(a.date), &a.slug).cmp(&(Reverse(b.date), &b.slug)));

        let mut statics = Pages::new();
        collect(&root.join("static"), "static/", &mut statics)?;
        let mut css = Highlighter::css()?.into_bytes();
        css.extend(
            statics
                .remove("static/style.css")
                .context("static/style.css is missing")?,
        );
        statics.insert("static/style.css".to_owned(), css);

        Ok(Self {
            config,
            posts,
            statics,
        })
    }

    pub fn render(&self, highlighter: &Highlighter) -> Result<Rendered> {
        let mut pages = self.statics.clone();
        let css = format!(
            "/static/style.css?v={}",
            hash(&self.statics["static/style.css"])
        );

        let mut media = BTreeSet::new();
        for post in &self.posts {
            let body = markdown::to_html(&post.markdown, &post.slug, highlighter)
                .with_context(|| post.slug.clone())?;
            media.extend(
                body.media
                    .iter()
                    .map(|name| format!("{}/{name}", post.slug)),
            );
            let page = PostPage {
                config: &self.config,
                css: &css,
                post,
                body: &body.html,
            };
            pages.insert(format!("{}.html", post.slug), page.render()?.into_bytes());
        }

        let index = IndexPage {
            config: &self.config,
            css: &css,
            posts: &self.posts,
        };
        pages.insert("index.html".to_owned(), index.render()?.into_bytes());

        let not_found = NotFoundPage {
            config: &self.config,
            css: &css,
        };
        pages.insert("404.html".to_owned(), not_found.render()?.into_bytes());
        Ok(Rendered { pages, media })
    }
}

pub fn write(root: &Path, pages: &Pages) -> Result<PathBuf> {
    let time = jiff::ZonedDateTime::now()
        .strftime("%Y-%m-%d_%H-%M-%S")
        .to_string();
    let dist = root.join("dist/posts");
    fs::create_dir_all(&dist)?;
    let dist = dist.join(time);
    fs::create_dir(&dist).with_context(|| format!("creating {}", dist.display()))?;
    for (path, body) in pages {
        let target = dist.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, body).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(dist)
}

fn collect(dir: &Path, prefix: &str, out: &mut Pages) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .ok()
            .context("file name is not utf-8")?;
        if name.starts_with('.') {
            continue;
        }
        let key = format!("{prefix}{name}");
        if entry.file_type()?.is_dir() {
            collect(&entry.path(), &format!("{key}/"), out)?;
        } else {
            out.insert(key, fs::read(entry.path())?);
        }
    }
    Ok(())
}

fn hash(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    format!("{:016x}", hasher.finish())
}
