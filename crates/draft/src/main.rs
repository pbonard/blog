use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};

use site::{Site, markdown::Highlighter, post};

#[derive(Parser)]
#[command(about = "A small static blog generator")]
struct Cli {
    #[arg(
        long,
        default_value = ".",
        global = true,
        help = "Directory that holds hero.toml, posts/ and static/"
    )]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Render the site into a new folder of dist/posts/, to look at the files")]
    Build,
    #[command(
        about = "Create posts/<slug>.md as a draft, and posts/<slug>/ for its photos and videos"
    )]
    New { title: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build => build(&cli.root),
        Command::New { title } => new_post(&cli.root, &slug_of(&title)?, &title),
    }
}

fn build(root: &Path) -> Result<()> {
    let site = Site::load(root, false)?;
    let pages = site.render(&Highlighter::default())?.pages;

    let dist = site::write(root, &pages)?;
    println!(
        "{} posts, {} files -> {}",
        site.posts.len(),
        pages.len(),
        dist.display()
    );
    Ok(())
}

fn slug_of(title: &str) -> Result<String> {
    let slug = title
        .replace(['\'', '’'], "")
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    ensure!(
        !slug.is_empty(),
        "{title:?} has no a-z or 0-9 to make the file name from"
    );
    Ok(slug)
}

fn new_post(root: &Path, slug: &str, title: &str) -> Result<()> {
    post::check_slug(slug)?;
    let path = root.join("posts").join(format!("{slug}.md"));
    ensure!(!path.exists(), "{} already exists", path.display());

    let title = toml::Value::String(title.to_owned());
    let date = jiff::ZonedDateTime::now().date();
    let media = root.join("posts").join(slug);
    fs::create_dir_all(&media)?;
    fs::write(
        &path,
        format!("+++\ntitle = {title}\ndate = \"{date}\"\ndraft = true\n+++\n\n"),
    )?;
    println!("{}", path.display());
    println!("{}/", media.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_the_slug_from_the_title() {
        for (title, slug) in [
            ("Good Good", "good-good"),
            ("NUMA Cost & Performance", "numa-cost-performance"),
            ("  Don't use div(3)!  ", "dont-use-div-3"),
            ("std::lower_bound", "std-lower-bound"),
            ("Rust 시작하기 2", "rust-2"),
        ] {
            assert_eq!(slug_of(title).unwrap(), slug, "{title}");
            post::check_slug(slug).unwrap();
        }
        for title in ["", "첫 번째 글", "!!!"] {
            assert!(slug_of(title).is_err(), "{title}");
        }
    }

    #[test]
    fn makes_the_post_and_its_media_folder() {
        let root = std::env::temp_dir().join(format!("blog-draft-{}", std::process::id()));

        new_post(&root, "good-good", "Good Good").unwrap();
        let post = fs::read_to_string(root.join("posts/good-good.md")).unwrap();
        assert!(
            post.contains("Good Good") && post.contains("draft = true"),
            "{post}"
        );
        assert!(root.join("posts/good-good").is_dir());

        assert!(new_post(&root, "good-good", "Again").is_err());
        assert!(new_post(&root, "../evil", "Evil").is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
