use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand};

use site::{Site, markdown::Highlighter, post};

#[derive(Parser)]
#[command(about = "A small static blog generator")]
struct Cli {
    /// Directory that holds hero.toml, posts/ and static/
    #[arg(long, default_value = ".", global = true)]
    root: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render the site into dist/, to look at the files
    Build,
    /// Create posts/<slug>.md as a draft
    New { slug: String, title: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Build => build(&cli.root),
        Command::New { slug, title } => new_post(&cli.root, &slug, &title),
    }
}

fn build(root: &Path) -> Result<()> {
    // Everything is rendered before dist/ is touched, so a broken post leaves the last good build in place.
    let site = Site::load(root, false)?;
    let pages = site.render(&Highlighter::default())?.pages;

    let dist = root.join("dist");
    if dist.exists() {
        fs::remove_dir_all(&dist).with_context(|| format!("clearing {}", dist.display()))?;
    }
    for (path, body) in &pages {
        let target = dist.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, body).with_context(|| format!("writing {}", target.display()))?;
    }
    println!(
        "{} posts, {} files -> {}",
        site.posts.len(),
        pages.len(),
        dist.display()
    );
    Ok(())
}

fn new_post(root: &Path, slug: &str, title: &str) -> Result<()> {
    post::check_slug(slug)?;
    let path = root.join("posts").join(format!("{slug}.md"));
    ensure!(!path.exists(), "{} already exists", path.display());

    let title = toml::Value::String(title.to_owned());
    let date = jiff::ZonedDateTime::now().date();
    fs::create_dir_all(root.join("posts"))?;
    fs::write(
        &path,
        format!("+++\ntitle = {title}\ndate = \"{date}\"\ndraft = true\n+++\n\n"),
    )?;
    println!("{}", path.display());
    Ok(())
}
