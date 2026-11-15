# Make MKV

A small Windows app that copies the titles you pick from a DVD, a Blu-ray disc or a folder of video files into MKV files. It uses ffmpeg to copy the streams without re-encoding, so it is fast and loses no quality.

It is a personal tool. It does not decrypt AACS or BD+ protected discs. For a protected disc it shows a note and does nothing else.

## Requirements

- Windows
- ffmpeg and ffprobe, either on the PATH or in a folder you set in the app (`ffmpeg -version` should work in a terminal)
- Rust (edition 2024) to build

## Build and run

    cd app
    cargo run --release

The window opens at once. ffmpeg and the drive list are checked in the background.

## Media Renamer

The repo also holds a second app, [Media Renamer](media-renamer/README.md). It renames TV episode and movie files into a consistent naming scheme, offline: it shows the new names before anything changes, and the last batch can be undone.

    cd media-renamer
    cargo run --release

## Using it

1. Put a disc in a drive and press **Use this** next to it, or press **Browse** and pick a folder.
2. Press **Look for titles**. **Stop scan** ends a scan early.
   - **DVD:** each program (title) in a title set becomes one title, read from the disc's menu file (`VIDEO_TS/VTS_NN_0.IFO`). Without that file, the whole title set is one title.
   - **Blu-ray:** each playlist (`BDMV/PLAYLIST/*.mpls`) becomes one title. Duplicate playlists are dropped.
   - **Other folders:** every `mkv, m2ts, ts, vob, mp4` file is one title.
   - Titles are named after the disc: its Blu-ray title, or its volume label.
3. Tick the titles you want. Titles shorter than **Shortest title** are unticked. Changing that setting re-ticks the list.
4. Choose an **Output folder**.
5. Press **Rip selected**. **Stop** cancels the current file and deletes its partial output.

If the output drive lacks room for the selected titles plus 1 GB, the rip does not start.

### File names

The **File name** template supports two tokens:

- `%02u` is the title number, for example `01`.
- `%s` is the disc name, or the folder name when the disc has none, for example `MOVIE_DISC`.

The default is `%s_%02u.mkv`, which gives `MOVIE_DISC_01.mkv`. If a name is taken, `_tNN` is added. Existing files are never overwritten.

### Settings

The output folder, file-name template, shortest-title length, ffmpeg folder and eject option are remembered between runs.

- **ffmpeg folder:** leave it blank to use the PATH, or set the folder that holds `ffmpeg.exe` and `ffprobe.exe`.
- **Eject the disc when done:** ejects the disc after a rip finishes, when the source is a drive.

### Log

Failed rips write the full ffmpeg error to `make_mkv.log` in the output folder.

## Layout

- `app/src/main.rs`: the window, the scan and rip workers, the settings, and ffmpeg.
- `app/src/logic.rs`: naming, the length rule, the disc-type check, and the Blu-ray playlist parser. Tested.
- `app/src/ifo.rs`: the DVD program-chain reader. Tested with a synthetic menu file.
- `app/src/disc.rs`: finds the titles on a DVD or Blu-ray layout. Tested with temporary folders.
- `app/src/drives.rs`: Windows drive list, volume labels, free space and eject.

## Notes and limits

- **Not tested against real discs.** The DVD and Blu-ray title reading was written from the format layouts and tested with synthetic files. Check the first rip of each disc type.
- The pieces of a multi-part title are fed to ffmpeg through a pipe, so they play as one stream. This needs no temporary files.
- Blu-ray playlists are played as whole clips. A playlist's in and out points are not cut yet, so a clip can run past the part you wanted.
- Each DVD program is copied in full. Its angles and subtitle choices are not selected separately.
- The ffmpeg check looks for `ffmpeg` in the folder you set, or on the PATH.

## License

MIT. See [LICENSE](LICENSE).
