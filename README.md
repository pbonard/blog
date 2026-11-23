# blog

A static blog generator. Posts are markdown files in `posts/`, and the file name is the url:
`posts/good.md` becomes `/good`.

    cargo run -- new good "good"   # create posts/good.md as a draft
    cargo run -p preview                # preview on http://127.0.0.1:3000, drafts included
    cargo run -p publish                # upload what changed to garage, --dry-run only prints it
    cargo run -p publish -- --delete    # also take pages offline whose post is gone from posts/

Remove `draft = true` from a post to publish it. The site's title and author are in `hero.toml`.

A photo or video goes in `media/<slug>/` and is written as just its name: `![name](file1.webp)` in
`posts/good.md` is `media/good/file1.webp`.
