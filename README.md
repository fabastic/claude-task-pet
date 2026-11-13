# Claude Task Pet

A tiny desktop pet that shows what Claude Code is doing. It is just the animation: no window, no background, no taskbar entry. Hover over it to see the current task.

Written in Rust (tao + wry on the system WebView2). The release exe is about 2.7 MB and uses about 28 MB of RAM.

## Use

```
claude-task-pet.exe install     # copies itself to %LOCALAPPDATA%\ClaudeTaskPet and adds Claude Code hooks
%LOCALAPPDATA%\ClaudeTaskPet\claude-task-pet.exe   # start the pet
claude-task-pet.exe uninstall   # removes the hooks
```

- Drag it anywhere; its position is remembered.
- Right-click: Character (`clawd` / `mascot`), Size, Quit.
- Hover: shows the current task, e.g. "Editing main.rs" or "Running: npm test".

Settings live in `%APPDATA%\claude-task-pet\pet.json`.

## Build

```
cargo build --release
```

On the `x86_64-pc-windows-gnu` toolchain, `parking_lot` is pinned to 0.12.3 in `Cargo.lock` so no external `dlltool`/assembler is needed.

## Artwork

The animations are not mine and are not covered by any license here:

- `web/assets/clawd`: from [rullerzhou-afk/clawd-on-desk](https://github.com/rullerzhou-afk/clawd-on-desk). All Rights Reserved; Clawd is Anthropic's character; non-commercial only.
- `web/assets/mascot`: from [Harriszed/claude-desktop-pet](https://github.com/Harriszed/claude-desktop-pet). No redistribution license for the artwork.

This is a private, personal project. Unofficial; not affiliated with Anthropic.
