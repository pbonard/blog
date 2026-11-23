use anyhow::{Result, ensure};

pub fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "pdf" => "application/pdf",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// A photo or video of a post, with no character that would be encoded in a url.
pub(crate) fn check_media_name(name: &str) -> Result<()> {
    ensure!(
        !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "{name:?} may only contain a-z, A-Z, 0-9, . _ and -"
    );
    let kind = content_type(name);
    ensure!(
        kind.starts_with("image/") || kind.starts_with("video/"),
        "{name:?} is not a photo or video that browsers show"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_names_need_no_encoding() {
        for name in ["a.png", "IMG_1.JPG", "clip-2.mp4"] {
            assert!(check_media_name(name).is_ok(), "{name}");
        }
        for name in [
            "",
            "a b.png",
            "사진.png",
            "../a.png",
            "x/a.png",
            ".a.png",
            "a%20.png",
            "a.heic",
            "a.html",
            "a",
        ] {
            assert!(check_media_name(name).is_err(), "{name}");
        }
    }
}
