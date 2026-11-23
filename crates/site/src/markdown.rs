use anyhow::{Context, Result};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use syntect::{
    highlighting::ThemeSet,
    html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

use crate::path::{check_media_name, content_type};

const CLASSES: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

pub struct Highlighter {
    syntaxes: SyntaxSet,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
        }
    }
}

impl Highlighter {
    fn highlight(&self, lang: &str, code: &str) -> Result<Option<String>> {
        let Some(syntax) = self.syntaxes.find_syntax_by_token(lang) else {
            return Ok(None);
        };
        let mut generator =
            ClassedHTMLGenerator::new_with_class_style(syntax, &self.syntaxes, CLASSES);
        for line in LinesWithEndings::from(code) {
            generator.parse_html_for_line_which_includes_newline(line)?;
        }
        Ok(Some(generator.finalize()))
    }

    pub(crate) fn css() -> Result<String> {
        let themes = ThemeSet::load_defaults().themes;
        let light = css_for_theme_with_class_style(&themes["InspiredGitHub"], CLASSES)?;
        let dark = css_for_theme_with_class_style(&themes["base16-ocean.dark"], CLASSES)?;
        Ok(format!(
            "@media (prefers-color-scheme: light) {{\n{light}}}\n@media (prefers-color-scheme: dark) {{\n{dark}}}\n"
        ))
    }
}

pub(crate) struct Body {
    pub html: String,
    pub media: Vec<String>,
}

pub(crate) fn to_html(markdown: &str, slug: &str, highlighter: &Highlighter) -> Result<Body> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_SMART_PUNCTUATION
        | Options::ENABLE_HEADING_ATTRIBUTES;

    let mut events = Vec::new();
    let mut media = Vec::new();
    let mut block: Option<(String, String)> = None;
    let mut video: Option<(String, String)> = None;
    for event in Parser::new_ext(markdown, options) {
        if let Some((url, label)) = &mut video {
            match event {
                Event::End(TagEnd::Image) => {
                    events.push(Event::InlineHtml(
                        format!(
                            "<video controls preload=\"metadata\" src=\"{url}\" aria-label=\"{}\"></video>",
                            escape(label).replace('"', "&quot;")
                        )
                        .into(),
                    ));
                    video = None;
                }
                Event::Text(text) | Event::Code(text) => label.push_str(&text),
                _ => {}
            }
            continue;
        }
        match (event, &mut block) {
            (
                Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }),
                _,
            ) if is_file_name(&dest_url) => {
                check_media_name(&dest_url).context("a photo or video of the post")?;
                let url = format!("/{slug}/{dest_url}");
                media.push(dest_url.to_string());
                if content_type(&dest_url).starts_with("video/") {
                    video = Some((url, String::new()));
                } else {
                    events.push(Event::Start(Tag::Image {
                        link_type,
                        dest_url: url.into(),
                        title,
                        id,
                    }));
                }
            }
            (Event::Start(Tag::CodeBlock(kind)), _) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split([' ', ',']).next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                block = Some((lang, String::new()));
            }
            (Event::Text(text), Some((_, code))) => code.push_str(&text),
            (Event::End(TagEnd::CodeBlock), Some((lang, code))) => {
                let body = match highlighter.highlight(lang, code)? {
                    Some(html) => html,
                    None => escape(code),
                };
                events.push(Event::Html(
                    format!("<pre><code>{body}</code></pre>\n").into(),
                ));
                block = None;
            }
            (event, _) => events.push(event),
        }
    }

    let mut html = String::new();
    html::push_html(&mut html, events.into_iter());
    Ok(Body { html, media })
}

fn is_file_name(url: &str) -> bool {
    !url.is_empty() && !url.starts_with(['/', '#']) && !url.contains(':')
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(markdown: &str) -> Result<Body> {
        to_html(markdown, "numa", &Highlighter::default())
    }

    fn html(markdown: &str) -> String {
        body(markdown).unwrap().html
    }

    #[test]
    fn file_names_belong_to_the_post() {
        let body = body("![A chart](a.webp) ![](b.JPG)").unwrap();
        assert!(
            body.html
                .contains("<img src=\"/numa/a.webp\" alt=\"A chart\""),
            "{}",
            body.html
        );
        assert_eq!(body.media, ["a.webp", "b.JPG"]);
    }

    #[test]
    fn addresses_are_left_alone() {
        for url in [
            "https://x.org/a.png",
            "/static/a.png",
            "data:image/png;base64,AA",
        ] {
            let body = body(&format!("![]({url})")).unwrap();
            assert!(
                body.html.contains(&format!("src=\"{url}\"")),
                "{}",
                body.html
            );
            assert!(body.media.is_empty());
        }
    }

    #[test]
    fn videos_get_a_player() {
        let body = body("before ![A \"clip\"](c.mp4) after").unwrap();
        assert!(
            body.html
                .contains("<video controls preload=\"metadata\" src=\"/numa/c.mp4\" aria-label="),
            "{}",
            body.html
        );
        assert!(!body.html.contains("<img"));
        assert!(body.html.contains("after"));
        assert_eq!(body.media, ["c.mp4"]);
    }

    #[test]
    fn rejects_other_files_and_folders() {
        for url in ["img/a.png", "../a.png", "a.heic", "notes.txt"] {
            assert!(body(&format!("![]({url})")).is_err(), "{url}");
        }
    }

    #[test]
    fn highlights_known_languages() {
        let html = html("```rust\nfn main() {}\n```\n");
        assert!(html.contains("hl-"), "{html}");
        assert!(html.ends_with("</code></pre>\n"));
    }

    #[test]
    fn escapes_unknown_languages() {
        let html = html("```nope\na < b && c\n```\n");
        assert_eq!(html, "<pre><code>a &lt; b &amp;&amp; c\n</code></pre>\n");
    }

    #[test]
    fn knows_the_extra_languages() {
        for lang in ["toml", "ts", "dockerfile", "kotlin", "go", "yaml", "sh"] {
            assert!(
                Highlighter::default()
                    .highlight(lang, "x\n")
                    .unwrap()
                    .is_some(),
                "{lang}"
            );
        }
    }
}
