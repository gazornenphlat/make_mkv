# Disc Ripper

A small Windows app that copies the titles you pick from a DVD, a Blu-ray disc or a folder of video files into MKV files. It uses ffmpeg to copy the streams without re-encoding, so it is fast and loses no quality.

It is a personal tool. It does not decrypt AACS or BD+ protected discs. For a protected disc it shows a note and does nothing else.

## Requirements

- Windows
- ffmpeg and ffprobe on the PATH (`ffmpeg -version` should work in a terminal)
- Rust (edition 2024) to build

## Build and run

    cd app
    cargo run --release

The window opens at once. ffmpeg and the drive list are checked in the background.

## Using it

1. Put a disc in a drive and press **Use this** next to it, or press **Browse** and pick a folder.
2. Press **Look for titles**. **Stop scan** ends a scan early.
   - Titles are named after the disc: its Blu-ray title, or its volume label. The playlist or title-set number follows.
   - DVD: each title set (`VIDEO_TS/VTS_NN_*.VOB`) becomes one title. Its parts are joined in order.
   - Blu-ray: each playlist (`BDMV/PLAYLIST/*.mpls`) becomes one title, with its clips joined in order. Duplicate playlists are dropped.
   - Other folders: every `mkv, m2ts, ts, vob, mp4` file is one title.
3. Tick the titles you want. Titles shorter than **Shortest title** are unticked. Changing that setting re-ticks the list.
4. Choose an **Output folder**.
5. Press **Rip selected**. **Stop** cancels the current file and deletes its partial output.

### File names

The **File name** template supports two tokens:

- `%02u` is the title number, for example `01`.
- `%s` is the name of the disc or folder, for example `MOVIE_DISC`.

The default is `title%02u.mkv`. A template such as `%s_%02u.mkv` gives `MOVIE_DISC_01.mkv`. If a name is taken, `_tNN` is added. Existing files are never overwritten.

## Layout

- `app/src/main.rs`: the window, the scan and rip workers, and ffmpeg.
- `app/src/logic.rs`: naming, the length rule, the disc-type check and the Blu-ray and DVD parsers. Tested.
- `app/src/disc.rs`: finds the titles on a DVD or Blu-ray layout. Tested with temporary folders.
- `app/src/drives.rs`: Windows drive list.

## Notes and limits

- **Not tested against real discs.** The Blu-ray playlist and DVD part joining were written from the format layouts and tested with synthetic files. Check the first rip of each disc type.
- DVD title sets can hold a main feature and extras together. Each title set is one title, so the feature may need splitting by chapter later.
- Joined parts use ffmpeg's `concat:` input. Paths containing `|` will not work.
- Blu-ray and DVD titles are the playlists and title sets on the disc. The disc does not say which one is the film, so check the lengths and sizes.
- A failed rip removes its partial file. A file that already existed is never removed.
