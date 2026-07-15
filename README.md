<div align="center">

# arabel

**A desktop terminal with vertical tabs, built for working with AI coding agents.**

[![Licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](#licence)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)
![Version](https://img.shields.io/badge/version-0.4.0-green)

![arabel](docs/cover.png)

</div>

## Why

I was tired of Termius and friends. I wanted vertical tabs, and I wanted a
terminal that actually knows when an AI agent is working, waiting on me, or
done. So I built one. It stays small on purpose: a terminal, a sidebar, and the
handful of things you actually reach for.

## Features

| | |
|---|---|
| **SSH & local shells** | Key, password or ssh-agent auth. Imports hosts from `~/.ssh/config`. |
| **Vertical tabs & projects** | Group terminals into named, emoji-tagged projects. Each project keeps its views (tabs/splits) and restores them on launch. |
| **AI agent aware** | Per-pane working / waiting / done indicators, system notifications on agent events, one-click hook sync for Claude Code. |
| **tmux** | Persistent server-side sessions with reconnection, plus native control-mode (`tmux -CC`) mirroring tmux panes into the app grid. |
| **Git panel** | Status, stage, diff, commit, fetch/pull — over the remote, through the SSH channel. |
| **SFTP** | Browse, download, drag-and-drop upload. |
| **Port forwards & browser panes** | Preview your dev server inside the grid. |
| **Yours to tweak** | Configurable keybindings, terminal themes and fonts, config sharing between machines. |

## How it fits together

```
┌──────────────────────────────────────────────────┐
│  SvelteKit + xterm.js  (UI, panes, vertical tabs)│
├──────────────────────────────────────────────────┤
│  Tauri IPC                                       │
├──────────────────────────────────────────────────┤
│  Rust backend                                    │
│    ├─ SSH / SFTP / port forwards                 │
│    ├─ local PTY                                  │
│    ├─ tmux control-mode                          │
│    └─ encrypted secrets vault                    │
└──────────────────────────────────────────────────┘
```

Secrets (key passphrases, passwords) live in a local encrypted vault rather than
the OS keychain — a deliberate choice for an unsigned build, see `store.rs`.

## Install

Grab a build from [Releases](https://github.com/DelityLuss/arabel/releases), or
build it yourself:

```sh
npm install
npm run tauri build
```

## Development

```sh
npm run tauri dev     # the desktop app
npm run dev           # browser preview (mocked backend, no Tauri)
npm run check         # typecheck
```

Recommended setup: [VS Code](https://code.visualstudio.com/) with the
[Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode),
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
and [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
extensions.

## Contributing

All contributions welcome — features, bug fixes, ideas. Open an issue to talk it
over, or just send a PR. Please run `npm run check` before you do. Keep it
simple: that's the whole point of this project.

## Licence

MIT.

Project emojis in `static/emoji/` come from the **NewsEmoji** pack on Telegram
([t.me/addemoji/NewsEmoji](https://t.me/addemoji/NewsEmoji)) and remain the
property of their authors. They are bundled here for personal use and are not
covered by this project's licence.
