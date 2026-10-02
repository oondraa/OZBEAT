<p align="center"><img src="assety/OZBEAT_dark_big.png" alt="OZBEAT" width="480"></p>

**OZBEAT** is a music visualizer for Windows. It shows what is playing on your computer (Spotify, browsers, any app that reports to Windows media controls) and on BluOS network speakers, with artwork, lyrics and visuals that move smoothly with the music.

## Download

Get `OZBEAT.exe` from the [latest release](https://github.com/oondraa/OZBEAT/releases/latest) and run it. Nothing needs to be installed. The first run walks you through picking your sources and speakers.

> Windows SmartScreen may warn about an unknown publisher, because the exe is not code-signed yet. Click **More info → Run anyway**.

## Keys

| Key | Action |
|---|---|
| `Ctrl+S` | Settings |
| `Ctrl+D` | Mirror the visuals on the next screen |
| `F11` / double-click | Toggle fullscreen |
| `Esc` | Leave fullscreen / close settings |

## Options

```
--bluos <HOST>   Poll a BluOS player at HOST (repeatable)
--no-discover    Do not look for BluOS players via mDNS
--no-smtc        Do not read Windows media sessions
--youtube        Fetch background clips via yt-dlp (off by default)
--audio <MODE>   auto | loopback | mic | off
```

Optional environment variables: `FANART_API_KEY`, `THEAUDIODB_API_KEY` (better artwork), `MV_YTDLP` (path to your own `yt-dlp`).

## Building from source

Requires a recent stable Rust toolchain:

```
cargo build --release -p musicvisual
```

## License

OZBEAT is **source-available, not open source**. It is free for **personal, non-commercial use only**. Redistribution, commercial use and reuse of the code are not allowed without written permission. See [LICENSE](LICENSE) for the full terms and [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md) for the open-source components it uses.

Copyright © 2026 Ondra (oondraa). All rights reserved.
