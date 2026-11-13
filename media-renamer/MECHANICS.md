# Mechanics sheet (from the installed program's jar, read-only)

Source: C:\Program Files\FileBot\jar\filebot.jar (main class net.filebot.Main), read with java_method_show, java_class_show and java_classes_find.
The launcher (filebot.exe) only starts `jar\filebot.jar`; its strings contain no rename logic, so the rules are read from the jar's bytecode.
Each rule gives its function address as Class.method(descriptor). "estimated" = not found in the code, our guess.
No decompiled code was copied into the app; the rules are written again in our own code.

## Naming format (expression)
- Default episode format: `{n} - {s00e00} - {t}{subt}`. Source: net/filebot/ui/rename/FormatDialog.properties, episode.example[0] (default = first sample of the mode: Mode.getDefaultFormatExpression, Mode.getSampleExpressions).
- Default movie format: `{n.colon(' - ')} ({y}){subt}`. Source: FormatDialog.properties, movie.example[0].
- Episode bindings: n (series name), s (season), e (episode), t (title). Source: FormatDialog.properties episode.syntax.
- `{s00e00}` = MediaBindingBean.getS00E00 -> EpisodeFormat.formatS00E00(Episode): `S%02d` of the season, then `E%02d` of the episode (the special number is used when there is no season or episode). Result: S01E02.
- `{sxe}` = EpisodeFormat.formatSxE(Episode): season unpadded, then `x`, then `%02d` of the episode. Result: 1x02.
- Movie bindings: n (name), y (year). Source: FormatDialog.properties movie.syntax.
- `.colon(x)` = ExpressionFormatMethods.colon(String, String): replaces the regex `\s*[:꞉]+\s*` (util/RegularExpressions.<clinit> COLON) with x. So "Avatar: The Way" with ' - ' gives "Avatar - The Way".
- `.space(x)` = ExpressionFormatMethods.space -> Normalization.normalizeSpace(String, String): replaces `[:?._]` with a space and trims, then replaces `\s+` with x and trims.
- `.pad(n)` = ExpressionFormatMethods.pad(String, int[]) with lambda$pad$2: every run of digits in the value is zero-padded to width n. Only the one-width case was read; several widths are estimated.
- `.clean()` = ExpressionFormatMethods.clean(String) = FileUtilities.validateFileName(MediaDetection.stripReleaseInfo(s, true)). shortcut: release tags are not stripped in our build (ReleaseInfo.cleanRelease not read).
- `{subt}` = MediaBindingBean.getSubtitleTags(): a subtitle category tag plus the 3-letter language code of the matched subtitle, read from the media file. Not built: our build has no media probing, so {subt} is empty.

## Clean-up of new file names
- FileUtilities.validateFileName(String): removes the characters in ILLEGAL_CHARACTERS (`\ / : * ? " < > |`, line breaks, control characters, leading and trailing spaces, leading and trailing dots), repeats until nothing changes. Also cuts the name to 247 characters before the extension (`(?<=.{247}).+(?=[.][^.]+$)`). Source: FileUtilities.<clinit> ILLEGAL_CHARACTERS.

## Media files
- Video extensions: mkv, mk3d, mp4, m4v, 3gp, iso, mov, avi, mpg, mpeg, vob, ts, tp, m2ts, m4s, ogg, ogm, divx, asf, wmv, wtv, dvr-ms, rec, webm, flv, rm, rmvb, rmp4, tivo, nuv, 3dsbs, 3dtab, strm, strmlnk, streamlnk. Source: net/filebot/MediaTypes.<clinit> VIDEO_FILES.
- Subtitle extensions: srt, sub, ssa, ass, smi, sami, mpl, sup, vtt, ttml, vobsub, idx. Source: MediaTypes.<clinit> SUBTITLE_FILES.
- Image, audio and archive lists: MediaTypes.<clinit> (not used by this build).
- Companion files: EpisodeListMatcher.match keeps video files, folders and subtitle files, and subtitles follow their video. Source: EpisodeListMatcher.match(Collection, MatchMode, SortOrder, Locale, AutoDetectionMode, Set, Component).
- Empty selection message: "No episode files have been selected. Please <Load> episode files." Source: EpisodeListMatcher.match.

## Season and episode detection (file names)
Source: net/filebot/similarity/SeasonEpisodeMatcher.<init>(int, SeasonEpisodeFilter, boolean). Patterns are tried in this order; the first that finds numbers wins. Java's `\p{Alnum}` is written `[[:alnum:]]` in our code.
- 0: `Season N ... Episode N`. 1: `S01E02-E04`. 2: `S01E02` with ep, e, p or dash. 3: `1x02-1x03`. 4: `1x02` or `S1E02`. 5: `S01 - 02-03`. 6: `1.02`. 7: `EP 02` / `Episode 2` / `Act 2`. 8: `02-03`. 9: `SP01`. 10: union of 14, 15, 16. 11: ` - 123`. 12: `S01E02`. 13: `102`. 14: optional 2 or 4 digits, then e/ep/episode/act and a number. 15: `0102`. 16: `1 of 12`.
- Season folder name pattern: `(?i:Season|Series)[-._ ]?(\d{1,2})`. Source: SeasonEpisodeMatcher seasonPattern.
- Strict uses patterns 0 to 6 only. Source: EpisodeListMatcher.match, the bytecode flag at 0x166 to 0x177, and the SeasonEpisodeMatcher constructor.

Sanity limits (SeasonEpisodeFilter fields: seasonLimit, seasonEpisodeLimit, absoluteEpisodeLimit, seasonYearBegin, seasonYearEnd):
- Default: 30, 50, 1000, years 1920 to 2040. Source: SeasonEpisodeMatcher.<clinit> DEFAULT_SANITY. Which mode uses this set is estimated (Opportunistic).
- Strict: 10, 30, -1, -1, -1. Source: STRICT_SANITY. Which mode uses this set is estimated (Strict).
- Lenient: 300, 3000, 5000, 1920 to 2040. Source: LENIENT_SANITY (used by MediaDetection.<clinit>).

## Matching
- Two match modes. Opportunistic: "Find the best possible match". Strict: "Find the perfect match or none at all". Source: net/filebot/ui/rename/MatchMode.getDescription.
- The program's matching pipeline (EpisodeListMatcher.matchVideoFiles(Collection, SortOrder, Locale, boolean, AutoDetectionMode, Set, Component)): it groups the files by the series name found in each file name (MediaDetection.mapSeriesNamesByFiles), then matches each group against an episode list fetched from an online database (the call lambdas). Our build has no online list, so it matches against the list you type.
- Fallback to a match on the episode number alone (when the file has no season): our own choice, estimated. It is not in matchVideoFiles.
- Movie year in a file name: MediaDetection.<clinit> YEAR = `\b(?:19|20)\d{2}\b`. Name-with-year patterns: NAME_YEAR_IN_BRACKETS `^(.+?)[\[\(]((?:19|20)\d{2})[\]\)]` and NAME_YEAR `^([^\[\(]+?)((?:19|20)\d{2})`. Found years are filtered with DateMatcher.<clinit> DEFAULT_SANITY = (1930, 2040). (Our code writes the pattern as `\b(?:19|20)[0-9]{2}\b`, the same match.)
- Movie title: the quality tag cut (resolution, source, codec) is estimated. The program's own list is in ReleaseInfo.cleanRelease, not read.
- Online lookup of episode and movie data: not built (see the blocked unknown).

## Rename actions
- Move, Copy, Keep link, Symlink, Hard link, Clone, Duplicate, Test. Source: net/filebot/StandardRenameAction.<clinit>.
- Move = StandardRenameAction$1.rename(File, File): FileUtilities.move (on Linux a copy-on-write move is tried first). Our Move is a rename, with copy then delete when the drive changes.
- Conflict choices: Skip, Replace, Auto, Index, Fail. Source: net/filebot/cli/StandardConflictAction.<clinit>.
- Auto (number): StandardConflictAction$4.conflict(File, File): tries numbers 1 to 99 (IntStream.range(1, 100)), the first free name is used. The text joining the name, number and extension is not visible in the bytecode listing (the bootstrap recipe is not shown); our build writes "name (n).ext" (estimated).
- Default conflict choice: Skip. Source: net/filebot/cli/ArgumentBean.getConflictAction (`orElse(StandardConflictAction.SKIP)`, command-line default). The window's own default was not read.
- Undo: the program keeps a rename history (Settings.useRenameHistory, HistoryDialog). Our build keeps the last batch in memory.

## Scope of this version
- Built: the file list, season and episode detection, naming formats (s00e00, sxe, colon, space, pad, clean), match modes, preview, editable titles, Move, Copy, Hard link, Symlink, Test, conflict choices, undo of the last batch.
- Not built: online lookup of episode and movie databases (needs service keys and a network), {subt} tags (needs media probing), Keep link, Clone, Duplicate, music tags, subtitle matching and upload, checksum panel, post-processing scripts.
