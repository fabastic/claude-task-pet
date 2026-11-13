# Claude Task Pet

A tiny desktop pet that shows what Claude Code is doing. It is just the animation: no window, no background, no taskbar entry. Hover over it to see the current task.

Written in Rust (tao + wry on the system WebView2). The release exe is about 2.7 MB and uses about 28 MB of RAM.

## Install

Download `claude-task-pet.exe` from the latest release and double-click it. That's it. It installs for your user only (no admin):

- copies itself to `%LOCALAPPDATA%\ClaudeTaskPet`
- adds the Claude Code hooks to `~/.claude/settings.json` (a backup is saved next to it)
- adds **Claude Task Pet** to the Start menu, so Windows search finds it
- turns on start with Windows
- adds an entry in **Settings → Apps → Installed apps**, where you can uninstall it

## Use

- Drag it anywhere; its position is remembered.
- Hover: shows what Claude is doing right now, and the model and mode underneath.
- Right-click the pet: Character (`clawd` / `clawd-pet` / `mascot` / `calico` / `cloudling`), Size, Quit.
- Tray icon (pixel crab): Hide/Show pet, Start with Windows, Quit.
- Opening it from the Start menu while it's running brings a hidden pet back.

Settings live in `%APPDATA%\claude-task-pet\pet.json`.

## Build

```
cargo build --release
node tools/make-icon.js icon.ico
rcedit target/release/claude-task-pet.exe --set-icon icon.ico --set-version-string ProductName "Claude Task Pet"
```

On the `x86_64-pc-windows-gnu` toolchain, `parking_lot` is pinned to 0.12.3 in `Cargo.lock` so no external `dlltool`/assembler is needed.

## Artwork

The animations are not mine and are not covered by any license here:

- `web/assets/clawd`: from [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk). All Rights Reserved; Clawd is Anthropic's character; non-commercial only.
- `web/assets/calico`, `web/assets/cloudling`: also from clawd-on-desk (Calico © rullerzhou-afk, All Rights Reserved).
- `web/assets/mascot`: from [Harriszed/claude-desktop-pet](https://github.com/Harriszed/claude-desktop-pet). No redistribution license for the artwork.
- `web/assets/clawd-pet`: from [abderrahimghazali/clawd-pet](https://github.com/abderrahimghazali/clawd-pet), MIT licensed (Clawd itself is Anthropic's character).

This is a private, personal project. Unofficial; not affiliated with Anthropic.
