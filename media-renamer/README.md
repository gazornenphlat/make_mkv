# Media Renamer

A small desktop app that renames TV episode and movie files into a consistent naming scheme. It reads the season, episode, title and year from the file names, shows the new names before anything changes, and can undo the last batch.

It works offline. Episode titles come from a list you type in. It does not look anything up online.

## Requirements

- Rust (edition 2024) to build
- Windows for the full feature set. It builds on other systems, but symbolic links only work on Windows.

## Build and run

    cd media-renamer
    cargo run --release

Run the tests with `cargo test`.

## Using it

1. Drop video files or folders on the window, or press **Add files…**.
2. Choose **TV episodes** or **Movies**.
3. For episodes, type the **Series name** and paste an **Episode list**, one episode per line, such as `S01E02 Title` or `1x02 Title`.
4. Check the preview. Each file shows what was found, the title, the new name and a status. In movie mode the title can be typed by hand.
5. Choose **What to do** and what happens if the new name already exists.
6. Press the button at the bottom. The log shows one line per file.
7. **Undo last rename** reverts the last batch.

Subtitle files that sit next to a video are renamed with it and keep their language part, for example `.en`.

### Match mode

- **Best possible match** tries every season and episode pattern, such as `S01E02`, `1x02`, `Season 1 Episode 2`, `102` and `1 of 12`.
- **Perfect match or none** uses only the stricter patterns and needs both the season and the episode.

### Name format

The default for episodes is `{n} - {s00e00} - {t}`, which gives `Series - S01E02 - Title.mkv`.
The default for movies is `{n.colon(' - ')} ({y})`, which gives `Avatar - The Way of Water (2022).mkv`.

| Token | Meaning |
| --- | --- |
| `{n}` | Series name or movie title |
| `{s}`, `{e}` | Season and episode number |
| `{t}` | Episode title |
| `{y}` | Movie year |
| `{s00e00}` | `S01E02` |
| `{sxe}` | `1x02` |

| Function | What it does |
| --- | --- |
| `.colon(' - ')` | Replaces a colon and the spaces around it |
| `.space('.')` | Replaces spaces, and `: ? . _`, with the given text |
| `.pad(2)` | Zero-pads every number to the given width |
| `.clean()` | Removes characters that are not allowed in file names |

Characters that Windows does not allow (`\ / : * ? " < > |`) are always removed, and names are cut to 247 characters before the extension.

### Actions and conflicts

Actions: **Move**, **Copy**, **Hard link**, **Symbolic link** and **Test**, which only lists what would happen.

If the new name already exists: **Skip** (the default), **Replace**, or **Auto-number**, which uses `name (1).ext` up to `name (99).ext`.

## Layout

- `src/main.rs`: starts the window.
- `src/app.rs`: the window, the file list, the preview and the log.
- `src/engine.rs`: detection, the name format, planning, renaming and undo. Tested.
- `MECHANICS.md` and `mechanics.json`: the rules and values the app follows, with where each came from. One is marked as an estimate.

## Notes and limits

- **No online lookup.** Titles come only from the file names and the typed episode list.
- Undo covers the last batch only and is lost when the app closes. A file that was replaced cannot be brought back.
- Undo of a move to another drive does not work yet.
- A file that already has the right name is reported as "the name already exists".
- With Auto-number, a subtitle can get a different number from its video.
- Release tags such as `1080p` or `x264` are not stripped from names.
- The package is named `media_renamer_run`. The window title is Media Renamer.
