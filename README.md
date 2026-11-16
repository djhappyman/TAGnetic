<p align="center">
  <img src="app/ui/assets/tagnetic-icon.svg" width="88" height="88" alt="Tagnetic logo">
</p>

<h1 align="center">Tagnetic</h1>

<p align="center">
  <b>A free music tag editor for Windows, macOS and Linux.</b><br>
  Edit tags, organise files, find artwork and spot duplicates.<br>
  Preview your changes, save when ready and undo an applied batch later.
</p>

<p align="center">
  <a href="https://github.com/djhappyman/TAGnetic/releases">Download</a> ·
  <a href="docs/guide/README.md">User guide</a> ·
  <a href="CHANGELOG.md">What’s new</a> ·
  <a href="https://github.com/djhappyman/TAGnetic/issues">Report an issue</a>
</p>

<p align="center">
  <img src="docs/img/tagnetic-workspace.svg" width="1000" alt="Tagnetic showing a music collection grouped by folder, online release results with cover artwork, and six staged changes ready to review.">
</p>

## What it can do

| Feature | What you can do |
| --- | --- |
| **Edit music tags** | Edit tracks in a table or update a whole selection. Copy, paste, number tracks and choose the columns you need. |
| **Search with chips** | Type a name or genre, choose a suggested filter and keep narrowing the results. Combine conditions and save presets to use again. |
| **Find artists across collaborations** | Find Hixxy on his own or in credits such as Hixxy & Sharkey and Hixxy feat. another artist. Searches ignore letter case. |
| **Find duplicates** | Check for exact file copies, matching audio or likely copies of the same recording. Compare tags and file details without changing your files. |
| **Look up releases** | Find tags and artwork on Discogs, MusicBrainz, Beatport, Bandcamp and Soundeo. You can also read tags from file names or pasted tracklists. |
| **Organise files** | Rename tracks and build folders from their tags. Preview the resulting names and locations before applying them. |
| **Clean up tags** | Fix spacing, change case, replace text and reuse your own transformation presets. |
| **Manage artwork** | Browse, import, rotate, embed and export cover images. |
| **Check BPM and keys** | Detect tempo from the audio and convert between musical and Camelot key notation. |
| **Preview and undo** | Review before-and-after values in Staged. Applied batches stay available for Undo, even after restarting the app. |
| **Export and listen** | Create playlists and reports, or check a track with the built-in player and waveform. |
| **Make it yours** | Choose light or dark themes, adjust the panels, save layouts and use keyboard shortcuts. |

Supports MP3, FLAC, M4A/MP4, AAC, WAV, AIFF, Ogg Vorbis, Opus, Speex, Musepack, Monkey’s Audio and WavPack.

## Find and compare duplicates

Choose a matching method and scan the open files. Results appear in groups as they are found. Select a group to compare every copy, including its tags, path, format, duration and artwork.

Duplicate scanning is read-only. It does not delete or merge your music.

<p align="center">
  <img src="docs/img/tagnetic-duplicates.svg" width="1000" alt="Tagnetic duplicate review with an example library: two copies grouped by track name, with shared tags, album differences and per-copy details in the sidebar.">
  <br>
  <sub>Duplicate review with an example library.</sub>
</p>

## A simple workflow

1. **Open a folder.** Browse the files or narrow them with search chips.
2. **Stage your changes.** Edit tags, import a release, change artwork or rename files.
3. **Review and Apply.** Check the before-and-after values in Staged, then save. Undo an applied batch if you need to.

## Download

Get Tagnetic from [this repository’s Releases](https://github.com/djhappyman/TAGnetic/releases).

| Platform | Downloads |
| --- | --- |
| Windows x64 and ARM64 | `.exe` installer |
| macOS Apple Silicon and Intel | Universal `.dmg` |
| Linux x64 | `.deb`, `.rpm`, AppImage |
| Linux ARM64 | `.deb`, `.rpm` |

Tagnetic is in active development. In-app updates are disabled; download new versions from Releases. See [build and installation notes](docs/BUILDING.md) for more details.

<details>
<summary><b>Build from source</b></summary>

Use Node 22.12+, Rust and the platform’s [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
npm ci
npm run desktop:dev
```

See [development notes](docs/development.md), [build instructions](docs/BUILDING.md) and the [contribution workflow](docs/WORKFLOW.md).

</details>

## Built on TagRex

Tagnetic is an independent project built on [TagRex](https://github.com/tagrex/tagrex), created by [ThaVip3r](https://github.com/ThaVip3r) and its contributors. Their tagging engine, player and recovery work made this project possible.

Original copyright notices and the [GPL-3.0-only licence](LICENSE) are retained. The [original TagRex README](docs/upstream/README.md) is preserved. Shared improvements are tracked in the [upstream contribution ledger](docs/UPSTREAM.md).
