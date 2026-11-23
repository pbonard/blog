# blog

A static blog generator.

```sh  
 # create posts/good.md as a draft
cargo run -- new good-good "Good Good"
  
# preview on http://127.0.0.1:3000, drafts included
cargo run -p preview
            
# make the pages online the same as posts/, -- --dry-run only show the plan
cargo run -p publish
```

- Posts are markdown files in `posts/`, and the file name is the url: `posts/good.md` becomes `/good`.
- Remove `draft = true` from a post to publish it.
- The site's title and author are in `hero.toml`.
