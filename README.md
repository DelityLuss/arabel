# arabel

A desktop terminal for SSH and local shells, built with Tauri, SvelteKit and
xterm.js. Panes, tabs and splits are organised into **projects** so a whole
working set (several servers, several layouts) reopens exactly as you left it.

## Features

- **SSH & local terminals** — key, password or ssh-agent auth; import hosts from `~/.ssh/config`.
- **Projects** — group terminals into named, emoji-tagged projects; each project keeps several views (tabs/splits) and restores them on launch.
- **tmux** — persistent server-side sessions with reconnection, plus a native control-mode (`tmux -CC`) that mirrors tmux panes into the app grid.
- **Source control** — a git panel over the remote (status, stage, diff, commit, fetch/pull) driven through the SSH channel.
- **SFTP** — browse, download, and drag-and-drop upload files on the remote.
- **Port forwards** and integrated **browser panes** for previewing dev servers inside the grid.
- **Claude Code integration** — per-pane working/waiting/done indicators, system notifications on agent events, and one-click hook sync.
- **Configurable keybindings**, terminal themes and fonts, and config sharing between machines.

Secrets (key passphrases, passwords) are stored in a local encrypted vault, not
the OS keychain — a deliberate choice for an unsigned build (see `store.rs`).

## Development

```sh
npm install
npm run tauri dev     # run the desktop app
npm run dev           # browser preview (mocked backend, no Tauri)
npm run check         # typecheck
```

Recommended IDE setup: [VS Code](https://code.visualstudio.com/) with the
[Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode),
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode)
and [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer) extensions.

## Licence

MIT.

Project emojis in `static/emoji/` come from the **NewsEmoji** pack on Telegram
([t.me/addemoji/NewsEmoji](https://t.me/addemoji/NewsEmoji)), and remain the property of their
authors. They are bundled here for personal use and are not covered by this project's licence.
