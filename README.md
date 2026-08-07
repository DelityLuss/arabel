<div align="center">

<img src="docs/logo.png" width="88" alt="">

# Arabel

**A desktop terminal with vertical tabs, built for working with AI coding agents.**

[![Licence: MIT](https://img.shields.io/badge/licence-MIT-blue.svg)](#licence)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey)
![Version](https://img.shields.io/badge/version-0.4.0-green)

![Arabel](docs/cover.png)

</div>

## Why

I was tired of Termius and friends. I wanted vertical tabs, and I wanted a
terminal that actually knows when an agent is working, waiting on me, or done.

There are probably other tools that do some of this — honestly, I didn't go
looking. I built the one that answered my own needs. It stays small on purpose:
a terminal, a sidebar, and the handful of things you actually reach for.

## Features

| | |
|---|---|
| **Remotes** | Add, edit and reuse SSH identities across machines. Import from `~/.ssh/config` (with `Include` resolution) or from VS Code's Remote-SSH config. Per-remote working directory and per-remote flags: tmux, mosh, system-ssh, auto-launch the agent on connect. |
| **Four ways to connect** | Built-in SSH (russh: key, password, ssh-agent, known_hosts TOFU) · **system ssh** when you need real OpenSSH compatibility · **mosh** for flaky links · local shells and **WSL distros**. |
| **Vertical tabs & projects** | Group terminals into named, emoji-tagged projects. Each keeps its tabs and splits, restores on launch, re-attaches surviving tmux sessions, and can replay startup commands in every new pane. |
| **Agent-aware panes** | Working / waiting / done per pane, from Claude Code hooks *and* from reading the TUI directly (so it works with no hooks, even locally). Live activity ("Read src/x.ts", "thinking…"), notifications with sound and Dock badge, and you can answer the agent — ✓/✗ or a numbered choice — straight from the notification card. |
| **Agent tooling** | One-click hook sync, agent teams, plugin provisioning, paste an image into a remote agent (Ctrl+V, via SFTP), and `arabel preview <port>` — the agent asks, a browser pane opens and the tunnel is created. |
| **tmux** | Persistent sessions with reconnection, plus native control mode (`tmux -CC`) mirroring tmux panes into the app grid. |
| **Git panel** | Over the remote, through the SSH channel: status, stage, diff viewer, commit, branch list and switch, log, fetch with ahead/behind, `pull --ff-only`, push. |
| **SFTP** | Browse, download, drag-and-drop upload. |
| **Port forwards & browser panes** | Preview your dev server inside the grid. |
| **Dictation** | Speak into any pane (⌘⇧M): the mic is read natively (cpal), and transcription runs either through an OpenAI-compatible API (Groq, OpenAI, or a server of your own) or fully offline via a local Whisper model you download from the app. The text is inserted, not sent — you read it first. |
| **Getting around** | Command palette (⌘P) over panes, projects and remotes · find in terminal (⌘F) · zoom a pane · drag panes between tabs and projects. |
| **Yours to tweak** | 18 rebindable actions, a theme editor (16-colour ANSI grid, 5 presets incl. OLED), font import from VS Code, live CPU/RAM/disk meters, and config sharing between machines (secrets excluded). |

## How it fits together

```
┌──────────────────────────────────────────────────┐
│  SvelteKit + xterm.js  (UI, panes, vertical tabs)│
├──────────────────────────────────────────────────┤
│  Tauri IPC                                       │
├──────────────────────────────────────────────────┤
│  Rust backend                                    │
│    ├─ SSH / SFTP / port forwards   (russh)       │
│    ├─ local PTY · system ssh · mosh · WSL        │
│    ├─ tmux control-mode                          │
│    └─ encrypted secrets vault                    │
└──────────────────────────────────────────────────┘
```

Secrets (key passphrases, passwords) live in a local ChaCha20-Poly1305 vault
rather than the OS keychain — a deliberate choice for an unsigned build, see
`store.rs`.

## Install

Grab a build from [Releases](https://github.com/DelityLuss/arabel/releases), or
build it yourself:

```sh
npm install
npm run tauri build
```

The default build embeds a local Whisper (whisper.cpp), so it needs **cmake and a
C++ toolchain** — and Metal is switched on for macOS. If you only want dictation
through an API, drop it:

```sh
npm run tauri build -- --no-default-features
```

## Development

```sh
npm run tauri dev     # the desktop app
npm run dev           # browser preview, fully mocked backend, no Tauri
npm run check         # typecheck
```

Recommended setup: [VS Code](https://code.visualstudio.com/) with the
[Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode),
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
and [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
extensions.

## Contributing

This terminal is not perfect and I know it — it scratches my itch first. I'd be
genuinely happy to have people build it with me. Features, bug fixes, ideas: all
welcome. Open an issue to talk it over, or just send a PR. Run `npm run check`
before you do, and keep it simple — that's the whole point of the project.

Some known gaps, if you're looking for somewhere to start:

- **No ProxyJump / jump hosts.** Only reachable today by flipping a remote to the
  system-ssh transport. The `~/.ssh/config` importer ignores `ProxyJump`, so such
  a host imports silently broken.
- **No agent forwarding.**
- **SSH keepalive is hardcoded** (15s × 3), not configurable.
- The system-ssh and mosh transports **trade away SFTP, port forwards, metrics
  and tmux control mode** — reconciling that would be a nice win.
- The UI is one 5000-line `+page.svelte`. It works, but it's ripe for splitting.

## Licence

MIT.

Project emojis in `static/emoji/` come from the **NewsEmoji** pack on Telegram
([t.me/addemoji/NewsEmoji](https://t.me/addemoji/NewsEmoji)) and remain the
property of their authors. They are bundled here for personal use and are not
covered by this project's licence.
