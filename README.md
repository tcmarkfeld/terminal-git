# terminal-git

`tg` is a fast, minimal git diff viewer for the terminal. Run it in a repo to scroll through everything you've changed since your last commit, with syntax highlighting and a few lines of surrounding context. It updates live as files change, and you can commit and push without leaving it.

## Install

macOS and Linux:

```sh
curl -LsSf https://github.com/tcmarkfeld/terminal-git/releases/latest/download/tg-installer.sh | sh
```

Or from source with Rust:

```sh
cargo install --git https://github.com/tcmarkfeld/terminal-git
```

## Usage

```sh
tg                 # everything changed since HEAD, including untracked files
tg main...HEAD     # any arguments are passed straight to `git diff`
```

| Key | Action |
| --- | --- |
| `j` `k` / scroll | Scroll |
| `d` `u` / `Space` `b` | Half page / full page |
| `g` `G` | Top / bottom |
| `n` `p` | Next / previous file |
| `m` `o` | Collapse / open the current file (or click its header) |
| `h` `l` | Scroll sideways (long lines wrap automatically in narrow windows) |
| `⌘O` or `f` | File finder |
| `⌘/` | Commit dialog |
| `r` | Reload |
| `q` | Quit |

In the commit dialog: type a message, `Tab` to pick files (`Space` toggles, `a` toggles all), then `⏎` to commit, `⌘\` to commit and push, or `⌘U` to push. If your terminal doesn't pass `⌘` through, `Ctrl` works for the finder, commit dialog, and commit and push.

## License

MIT
