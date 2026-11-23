use std::{fs, path::Path};

use anyhow::{Context, Result, anyhow, ensure};
use jiff::civil::Date;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    title: String,
    date: Date,
    #[serde(default)]
    draft: bool,
}

#[derive(Debug)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub date: Date,
    pub draft: bool,
    pub markdown: String,
}

impl Post {
    /// The file name is the slug, and the slug is the url: posts/numa.md is served at /numa.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("file name is not utf-8")?;
        check_slug(slug)?;
        let source = fs::read_to_string(path)?;
        Self::parse(slug, &source)
    }

    fn parse(slug: &str, source: &str) -> Result<Self> {
        let source = source.replace("\r\n", "\n");
        let (front, markdown) = source
            .strip_prefix("+++\n")
            .and_then(|rest| rest.split_once("\n+++\n"))
            .ok_or_else(|| anyhow!("missing front matter between +++ lines"))?;
        let front: FrontMatter = toml::from_str(front).context("invalid front matter")?;
        Ok(Self {
            slug: slug.to_owned(),
            title: front.title,
            date: front.date,
            draft: front.draft,
            markdown: markdown.to_owned(),
        })
    }

    pub(crate) fn long_date(&self) -> String {
        self.date.strftime("%B %-d, %Y").to_string()
    }
}

pub fn check_slug(slug: &str) -> Result<()> {
    ensure!(
        !slug.is_empty()
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "slug {slug:?} may only contain a-z, 0-9 and -"
    );
    ensure!(
        !["index", "static", "404"].contains(&slug),
        "slug {slug:?} is reserved"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_front_matter_and_body() {
        let post = Post::parse(
            "a",
            "+++\ntitle = \"A & B\"\ndate = \"2026-11-03\"\n+++\n\nbody\n",
        )
        .unwrap();
        assert_eq!(post.title, "A & B");
        assert_eq!(post.long_date(), "November 3, 2026");
        assert!(!post.draft);
        assert_eq!(post.markdown.trim(), "body");
    }

    #[test]
    fn rejects_bad_front_matter() {
        assert!(Post::parse("a", "no front matter").is_err());
        assert!(Post::parse("a", "+++\ntitle = \"x\"\ndate = \"2026-13-01\"\n+++\n").is_err());
        assert!(
            Post::parse(
                "a",
                "+++\ntitle = \"x\"\ndate = \"2026-11-03\"\ndarft = true\n+++\n"
            )
            .is_err()
        );
    }

    #[test]
    fn slugs_stay_inside_the_site() {
        assert!(check_slug("numa-2").is_ok());
        for bad in ["", "../x", "a/b", "A", "index", "static"] {
            assert!(check_slug(bad).is_err(), "{bad}");
        }
    }
}
