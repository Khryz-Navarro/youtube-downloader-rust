# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

- **The public:** people on Windows 10/11 who want to save YouTube videos to their PC. The repository is
  private today; a public release is the intent, with no date set.
- **People evaluating Khryz's work:** the app is also a portfolio piece, so its code, README and release
  flow are read by employers or clients, not only used.

## Product Purpose

YT Downloader turns a YouTube link into a video file on disk. Paste a link, see what it is, pick a
quality, download; yt-dlp does the downloading and ffmpeg merges the video and audio streams. Success is
the video landing in the chosen folder, in the expected quality, as one playable file, without the user
touching yt-dlp, ffmpeg or a command line.

## Positioning

A small, quiet app that feels at home on Windows 11, rather than a web page in a window or a front-end
that exposes yt-dlp's options. It follows Windows conventions: its own title bar with Windows-style
window buttons, a theme that can follow the Windows setting, finished videos deleted to the Recycle Bin,
and "Show in folder" opening File Explorer. Its installer is about 23 MB, most of it the bundled ffmpeg.

## Operating Context

- A Windows desktop app built with Rust and Tauri 2 (`Khryz-Navarro/youtube-downloader-rust`). The UI is
  HTML/CSS/JS in `ui/`, rendered by WebView2; the Rust backend in `src/` runs yt-dlp and ffmpeg. It began
  as an Electron app (`Khryz-Navarro/youtube-downloader`), discontinued on 2026-11-07.
- Two tabs: **New download** (link box, preview, save folder and Download on the left; Quality on the
  right) and **Downloads** (the full-width list). Several downloads run at once.
- People use it on large and high-DPI monitors; that is why "Text and page size" exists.
- Installed from GitHub Releases (`YT-Downloader-Setup-<version>.exe`); each merged pull request publishes a
  new version.

## Capabilities and Constraints

- **Quality:** five presets (4K Best MKV; 4K, 1080p and 720p MP4 preferring H.264; Default MP4 via `-t mp4`)
  and a Custom row (max resolution 8K–144p × MP4/MKV/WebM). Every option includes the best audio. The
  exact yt-dlp arguments are in `src/ytdlp.rs` and the README.
- **Audio only:** a Video / Audio switch in Quality. Audio saves the best audio stream as MP3 (best VBR),
  AAC (.m4a, YouTube's own AAC kept as it is), WAV, FLAC or WMA, converted by the bundled ffmpeg. Only
  formats that ffmpeg can encode are offered.
- **Downloads list:** progress, size, speed and time left; Pause/Resume, Cancel, Retry, Show in folder,
  Delete, Clear finished. Delete removes only files yt-dlp reported for that download: leftovers
  permanently, a finished file to the Recycle Bin after a confirm.
- **Managed tools:** yt-dlp downloads on first launch and runs `yt-dlp -U` every launch; the status bar
  shows its version and update state. ffmpeg is bundled.
- **Settings:** Theme (Light, Dark, Desktop = follow Windows), Text and page size (100/125/150/175%),
  File names (Classic, Basic, Pretty, Nerdy; the finished file is renamed, never over another file) and
  Speed limit (a slider: 500 KB/s to 50 MB/s per download, or Unlimited; yt-dlp's `--limit-rate`).
  About shows the app, yt-dlp, ffmpeg and runtime versions. Last quality and folder are remembered.
- **Terminology in the UI:** New download, Downloads, Quality, Video, Audio, Custom, Paste, Change, Download,
  Show in folder, Retry, Resume, Clear finished, Settings, Theme, Desktop, Text and page size, File names,
  Speed limit.
- **Constraints:** Windows only (tested on Windows 11). One video per link (playlists are ignored). No
  cookies or login, so age-restricted and members-only videos fail. Only http(s) links reach yt-dlp. The
  installer is not code-signed. The app doesn't update itself (yt-dlp does).
- **Undecided:** no license has been chosen; no date for making the repository public.

## Brand Commitments

- Name: **YT Downloader**. Credit: **"Developed by Khryz"**, centered in the status bar and in About.
- The approved look is **Contact Sheet** (chosen 2026-11-07, recorded in DESIGN.md), which replaced the v1 look
  from the Figma file https://www.figma.com/design/aj74kwrRhnwNIhxQSibcxE. The redesign kept the two-tab
  layout and dark by default.
- Icons are Lucide (ISC). The app icon (`icons/icon.svg`) is the title-bar logo tile.
- The README keeps the legal note: only download videos you have the right to save.

## Evidence on Hand

- No screenshots of the current look are committed yet.
- README tables of each preset's yt-dlp options and what each picked for a 4K/60fps test video.
- Installer size: about 22.7 MiB (`YT-Downloader-Setup-0.1.2.exe` on Releases).
- There are no users, download counts, reviews, testimonials or press yet. Don't invent any.

## Product Principles

1. **Belong on Windows 11.** Prefer the platform's conventions (window buttons, theme, Recycle Bin, File
   Explorer) over web-app habits.
2. **Stay small and quiet.** Weigh every addition against size and calm; the footprint is part of the pitch.
3. **Public-grade, portfolio-grade.** A stranger installs and uses it without help, and the code, README
   and release flow hold up when someone reviews them.
4. **Never surprise people with their files.** Delete only what this app created; anything finished goes
   to the Recycle Bin, and only after asking.

## Accessibility & Inclusion

- "Text and page size" (up to 175%) exists for large and high-DPI screens; every screen must stay usable
  at each size.
- Keyboard: every control is reachable with Tab, Enter in the link box starts the download, arrow keys
  switch tabs.
- Text colors are chosen for at least 4.5:1 contrast in both themes.
