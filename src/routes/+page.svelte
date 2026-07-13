<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
  import { Terminal, type ITheme } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { SearchAddon } from "@xterm/addon-search";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { WebglAddon } from "@xterm/addon-webgl";
  import "@xterm/xterm/css/xterm.css";
  import { onDestroy } from "svelte";
  import { TmuxControl, parseLayout, layToTree, layPanes, toHexKeys, demo as tmuxccDemo, type Lay } from "$lib/tmuxcc";

  // ─── mode démo (aperçu navigateur sans Tauri) ─────────────────────────────
  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  if (!inTauri) try { tmuxccDemo(); } catch (e) { console.error("tmuxcc:", e); } // auto-test parseur en dev

  const DEMO_STORE = {
    identities: [{ id: "i1", name: "id_ed25519", keyPath: "~/.ssh/id_ed25519", hasPassphrase: true }],
    remotes: [
      { id: "r1", name: "vps-prod", host: "vps.exemple.dev", port: 22, user: "deploy", identityId: "i1", claude: true },
      { id: "r2", name: "staging", host: "staging.exemple.dev", port: 22, user: "root", identityId: "i1", claude: true },
      { id: "r3", name: "raspberry", host: "192.168.1.42", port: 22, user: "pi", identityId: "i1" },
    ],
    projects: [
      { id: "p1", name: "api-backend", root: { dir: "h" as const, ratio: 0.6, a: { remoteId: "r1", cmd: "" }, b: { remoteId: "r1", cmd: "npm run dev" } } },
      { id: "p2", name: "site-vitrine", root: { remoteId: "r2", cmd: "" } },
    ],
    settings: {},
  };
  const DEMO_OUTPUT =
    "\x1b[90mLast login: Sat Jul 12 21:14:03 from 82.64.12.7\x1b[0m\r\n" +
    "\x1b[32m❯\x1b[0m export ARABEL_PANE=…\r\n\x1b[32m❯\x1b[0m claude\r\n\r\n" +
    "\x1b[38;5;208m ▐▛███▜▌\x1b[0m   \x1b[1mClaude Code\x1b[0m v2.1\r\n" +
    "\x1b[38;5;208m▝▜█████▛▘\x1b[0m  \x1b[90m~/apps/api-backend\x1b[0m\r\n\r\n" +
    "\x1b[1m>\x1b[0m corrige le bug de pagination puis lance les tests\r\n\r\n" +
    "\x1b[90m✻ Réflexion…\x1b[0m\r\n\r\n" +
    "\x1b[32m●\x1b[0m \x1b[1mRead\x1b[0m src/routes/users.ts\r\n" +
    "\x1b[32m●\x1b[0m \x1b[1mEdit\x1b[0m src/routes/users.ts \x1b[90m(+4 -2)\x1b[0m\r\n" +
    "\x1b[33m●\x1b[0m \x1b[1mBash\x1b[0m npm test\r\n" +
    "  \x1b[90m⎿ En attente de permission…\x1b[0m\r\n";

  async function rpc<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    if (inTauri) return invoke<T>(cmd, args);
    // démo : réponses factices
    await new Promise((r) => setTimeout(r, cmd === "ssh_connect" ? 700 : 30));
    if (cmd === "store_load") return JSON.stringify(DEMO_STORE) as T;
    if (cmd === "claude_sync") return "claude présent · 5 élément(s) de config poussés · hooks installés" as T;
    if (cmd === "sftp_home") return "/home/deploy" as T;
    if (cmd === "sftp_list")
      return [
        { name: "apps", isDir: true, size: 0 },
        { name: ".claude", isDir: true, size: 0 },
        { name: "docker-compose.yml", isDir: false, size: 1893 },
        { name: "deploy.log", isDir: false, size: 482133 },
      ] as T;
    if (cmd === "sftp_download") return "/Users/luss/Downloads/fichier" as T;
    return undefined as T;
  }
  const listen: typeof tauriListen = inTauri
    ? tauriListen
    : (async () => () => {}) as unknown as typeof tauriListen;

  // ─── types ────────────────────────────────────────────────────────────────
  type Identity = { id: string; name: string; keyPath: string; hasPassphrase: boolean };
  type AuthKind = "key" | "password" | "agent";
  type Remote = { id: string; name: string; host: string; port: number; user: string; identityId: string; auth?: AuthKind; claude?: boolean; tmux?: boolean };
  type ImportHost = { host: string; hostName: string; user: string; port: number; identityFile: string };
  type PaneNode = { leaf: string } | { dir: "h" | "v"; ratio: number; a: PaneNode; b: PaneNode };
  type Tab = { id: string; root: PaneNode | null; active: string | null; projectId: string | null; cc?: string };
  type ProjLeaf = { remoteId: string; cmd: string; id?: string };
  type ProjNode = ProjLeaf | { dir: "h" | "v"; ratio?: number; a: ProjNode; b: ProjNode };
  type Project = { id: string; name: string; root: ProjNode };
  type SessStatus = { status: "connecting" | "open" | "closed" | "error"; error: string };
  type Modal =
    | { type: "remote"; data: Remote; password: string; back?: boolean }
    | { type: "sshImport"; hosts: ImportHost[]; back?: boolean }
    | { type: "identity"; data: Identity; passphrase: string; back?: boolean }
    | { type: "project"; data: Project }
    | { type: "saveProject"; tabId: string; name: string }
    | { type: "picker"; tabId: string | null; sid: string | null; dir: "h" | "v" | null; projectId: string | null; filter: string; cc?: boolean }
    | { type: "connections" }
    | { type: "settings" }
    | { type: "palette"; filter: string }
    | null;

  // pseudo-remote pour le terminal local
  const LOCAL: Remote = { id: "local", name: "Ce Mac", host: "", port: 0, user: "", identityId: "" };

  // ─── thèmes terminal ──────────────────────────────────────────────────────
  const THEMES: Record<string, ITheme> = {
    "Arabel Dark": {
      background: "#1e1e1e", foreground: "#d8d8dc", cursor: "#0a84ff", cursorAccent: "#1e1e1e",
      selectionBackground: "#33467c",
      black: "#15161e", red: "#f7768e", green: "#9ece6a", yellow: "#e0af68",
      blue: "#7aa2f7", magenta: "#bb9af7", cyan: "#7dcfff", white: "#a9b1d6",
      brightBlack: "#414868", brightRed: "#ff899d", brightGreen: "#9fe044", brightYellow: "#faba4a",
      brightBlue: "#8db0ff", brightMagenta: "#c7a9ff", brightCyan: "#a4daff", brightWhite: "#c0caf5",
    },
    "One Dark": {
      background: "#282c34", foreground: "#abb2bf", cursor: "#528bff", cursorAccent: "#282c34",
      selectionBackground: "#3e4451",
      black: "#3f4451", red: "#e06c75", green: "#98c379", yellow: "#e5c07b",
      blue: "#61afef", magenta: "#c678dd", cyan: "#56b6c2", white: "#abb2bf",
      brightBlack: "#5c6370", brightRed: "#ef596f", brightGreen: "#a9dc76", brightYellow: "#f0c674",
      brightBlue: "#74b2f8", brightMagenta: "#d55fde", brightCyan: "#2bbac5", brightWhite: "#ffffff",
    },
    Dracula: {
      background: "#282a36", foreground: "#f8f8f2", cursor: "#f8f8f2", cursorAccent: "#282a36",
      selectionBackground: "#44475a",
      black: "#21222c", red: "#ff5555", green: "#50fa7b", yellow: "#f1fa8c",
      blue: "#bd93f9", magenta: "#ff79c6", cyan: "#8be9fd", white: "#f8f8f2",
      brightBlack: "#6272a4", brightRed: "#ff6e6e", brightGreen: "#69ff94", brightYellow: "#ffffa5",
      brightBlue: "#d6acff", brightMagenta: "#ff92df", brightCyan: "#a4ffff", brightWhite: "#ffffff",
    },
    "Solarized Light": {
      background: "#fdf6e3", foreground: "#657b83", cursor: "#657b83", cursorAccent: "#fdf6e3",
      selectionBackground: "#eee8d5", selectionForeground: "#586e75",
      black: "#073642", red: "#dc322f", green: "#859900", yellow: "#b58900",
      blue: "#268bd2", magenta: "#d33682", cyan: "#2aa198", white: "#eee8d5",
      brightBlack: "#002b36", brightRed: "#cb4b16", brightGreen: "#586e75", brightYellow: "#657b83",
      brightBlue: "#839496", brightMagenta: "#6c71c4", brightCyan: "#93a1a1", brightWhite: "#fdf6e3",
    },
  };

  // ─── état global ──────────────────────────────────────────────────────────
  let identities = $state<Identity[]>([]);
  let remotes = $state<Remote[]>([]);
  let projects = $state<Project[]>([]);
  const ANSI_KEYS = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"] as const;
  let settings = $state({
    fontSize: 13,
    fontFamily: 'ui-monospace, "SF Mono", Menlo, monospace',
    theme: "Arabel Dark",
    copyOnSelect: true,
    sidebar: true,
    customTheme: { ...THEMES["Arabel Dark"] } as ITheme,
  });
  function activeTheme(): ITheme {
    return settings.theme === "Personnalisé" ? settings.customTheme : (THEMES[settings.theme] ?? THEMES["Arabel Dark"]);
  }
  function setCustom(key: keyof ITheme, val: string) {
    settings.customTheme = { ...settings.customTheme, [key]: val };
    applySettings();
  }

  // fontes système (chargées à l'ouverture des réglages)
  let fontList = $state<string[]>([]);
  let fontOpen = $state(false);
  async function loadFonts() {
    if (fontList.length) return;
    fontList = (await rpc<string[]>("list_fonts")) ?? ["Menlo", "Monaco", "SF Mono", "Courier New", "Fira Code", "JetBrains Mono", "Hack", "Cascadia Code"];
  }
  let loaded = $state(false);
  let modal = $state<Modal>(null);
  let confirmDeleteId = $state<string | null>(null);

  rpc<string>("store_load").then((json) => {
    const data = JSON.parse(json || "{}");
    identities = data.identities ?? [];
    remotes = data.remotes ?? [];
    projects = data.projects ?? [];
    settings = { ...settings, ...data.settings };
    restoreWorkspace(data.workspace);
    loaded = true;
  });

  async function save() {
    await rpc("store_save", {
      data: JSON.stringify({
        identities, remotes, projects,
        settings: $state.snapshot(settings),
        workspace: snapshotWorkspace(),
      }),
    });
  }

  // ré-ouvre les onglets tels qu'ils étaient à la fermeture (les sessions tmux
  // ayant survécu côté serveur, buildNode se réattache via la key persistée)
  function snapshotWorkspace() {
    const open = tabs.filter((t) => t.root);
    return {
      tabs: open
        .map((t) => ({ projectId: t.projectId, root: serializeTree(t.root!) }))
        .filter((t) => t.root),
      active: Math.max(0, open.findIndex((t) => t.id === activeTabId)),
    };
  }
  function restoreWorkspace(ws?: { tabs?: { projectId: string | null; root: ProjNode }[]; active?: number }) {
    if (!ws?.tabs?.length) return; // rien à restaurer : on garde l'onglet vide
    const restored: Tab[] = [];
    for (const t of ws.tabs) {
      const root = buildNode(t.root);
      if (root) restored.push({ id: crypto.randomUUID(), root, active: firstLeaf(root), projectId: t.projectId ?? null });
    }
    if (!restored.length) return;
    tabs = restored;
    activeTabId = restored[Math.min(ws.active ?? 0, restored.length - 1)].id;
  }

  // ─── toasts ───────────────────────────────────────────────────────────────
  type Toast = { id: string; msg: string; kind: "info" | "success" | "error" };
  let toasts = $state<Toast[]>([]);
  function toast(msg: string, kind: Toast["kind"] = "info", ms = 4000) {
    const t = { id: crypto.randomUUID(), msg, kind };
    toasts = [...toasts.slice(-2), t];
    setTimeout(() => (toasts = toasts.filter((x) => x.id !== t.id)), kind === "error" ? 6500 : ms);
  }

  // ─── onglets ──────────────────────────────────────────────────────────────
  const firstTabId: string = crypto.randomUUID();
  let tabs = $state<Tab[]>([{ id: firstTabId, root: null, active: null, projectId: null }]);
  let activeTabId = $state(firstTabId);
  const activeTab = $derived(tabs.find((t) => t.id === activeTabId));

  function firstLeaf(node: PaneNode | null): string | null {
    if (!node) return null;
    return "leaf" in node ? node.leaf : firstLeaf(node.a);
  }
  function leaves(node: PaneNode | null): string[] {
    if (!node) return [];
    return "leaf" in node ? [node.leaf] : [...leaves(node.a), ...leaves(node.b)];
  }
  function tabTitle(t: Tab): string {
    const proj = t.projectId && projects.find((p) => p.id === t.projectId);
    if (proj) return proj.name;
    const sid = firstLeaf(t.root);
    return (sid && sessions.get(sid)?.remote.name) || "nouvel onglet";
  }
  function newTab() {
    const t: Tab = { id: crypto.randomUUID(), root: null, active: null, projectId: null };
    tabs.push(t);
    activeTabId = t.id;
  }
  function closeTab(t: Tab) {
    if (t.cc) { const cc = ccSessions.get(t.cc); if (cc) return closeCc(cc); }
    leaves(t.root).forEach((sid) => closePane(sid));
    tabs = tabs.filter((x) => x.id !== t.id);
    if (!tabs.length) tabs.push({ id: crypto.randomUUID(), root: null, active: null, projectId: null });
    if (activeTabId === t.id) activeTabId = tabs[tabs.length - 1].id;
  }

  // re-fit les terminaux quand on change d'onglet (ils étaient en display:none)
  $effect(() => {
    const t = tabs.find((x) => x.id === activeTabId);
    if (!t) return;
    if (t.cc) { const cc = ccSessions.get(t.cc); if (cc) requestAnimationFrame(() => ccResize(cc)); }
    requestAnimationFrame(() => leaves(t.root).forEach((sid) => sessions.get(sid)?.fit.fit()));
  });
  function onWindowResize() {
    const t = tabs.find((x) => x.id === activeTabId);
    const cc = t?.cc ? ccSessions.get(t.cc) : null;
    if (cc) ccResize(cc);
  }

  // persiste l'espace de travail dès que la structure des onglets change
  $effect(() => {
    if (!loaded) return;
    const _sig = tabs.map((t) => `${t.projectId ?? ""}#${leaves(t.root).join(",")}`).join("|") + `@${activeTabId}`;
    void _sig; // force le suivi réactif de la structure
    save();
  });

  // charge la liste des fontes système à l'ouverture des réglages
  $effect(() => { if (modal?.type === "settings") loadFonts(); });

  // ─── arbre de splits ──────────────────────────────────────────────────────
  function withSplit(node: PaneNode, target: string, dir: "h" | "v", newSid: string): PaneNode {
    if ("leaf" in node) {
      return node.leaf === target ? { dir, ratio: 0.5, a: node, b: { leaf: newSid } } : node;
    }
    return { ...node, a: withSplit(node.a, target, dir, newSid), b: withSplit(node.b, target, dir, newSid) };
  }
  function withoutLeaf(node: PaneNode, target: string): PaneNode | null {
    if ("leaf" in node) return node.leaf === target ? null : node;
    const a = withoutLeaf(node.a, target);
    const b = withoutLeaf(node.b, target);
    if (!a) return b;
    if (!b) return a;
    return { ...node, a, b };
  }

  // redimensionnement des splits à la souris
  function dragDivider(e: PointerEvent, node: { dir: "h" | "v"; ratio: number; a: PaneNode; b: PaneNode }) {
    const parent = (e.currentTarget as HTMLElement).parentElement!;
    const rect = parent.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      const pos = node.dir === "h" ? (ev.clientX - rect.left) / rect.width : (ev.clientY - rect.top) / rect.height;
      node.ratio = Math.min(0.85, Math.max(0.15, pos));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    e.preventDefault();
  }

  // ─── sessions (hors $state : contient les objets xterm) ───────────────────
  type Sess = {
    term: Terminal;
    fit: FitAddon;
    search: SearchAddon;
    remote: Remote;
    cmd: string;
    key: string; // identité stable du panneau (nom de session tmux), survit aux relances
    tmux?: boolean;
    unlisteners: UnlistenFn[];
    webgl: boolean;
    cc?: { ctrlSid: string; paneId: string }; // panneau piloté en mode contrôle tmux
  };
  const sessions = new Map<string, Sess>();
  let sessStatus = $state<Record<string, SessStatus>>({});

  function termOptions() {
    return {
      fontFamily: settings.fontFamily,
      fontSize: settings.fontSize,
      cursorBlink: true,
      cursorStyle: "bar" as const,
      macOptionIsMeta: true,
      scrollback: 10000,
      lineHeight: 1.25,
      theme: activeTheme(),
    };
  }

  // ─── tmux : sessions persistantes côté serveur ────────────────────────────
  function shq(s: string): string {
    return `'${s.replace(/'/g, `'\\''`)}'`;
  }
  /** Crée ou réattache la session tmux ; l'init n'est tapé qu'à la création. */
  function tmuxCmd(name: string, init: string, paneSid: string | null): string {
    const n = shq(name);
    // ponytail: sleep 1 laisse le shell s'initialiser avant la frappe ; suffisant
    // pour les shells de VPS, un zsh très lourd peut nécessiter plus
    const send = init ? `sleep 1; tmux send-keys -t ${n} ${shq(init)} C-m; ` : "";
    const setenv = paneSid ? `tmux set-environment -t ${n} ARABEL_PANE ${shq(paneSid)} 2>/dev/null; ` : "";
    // réglages recommandés par Claude Code dans tmux : passthrough (notifs,
    // barre de progression) + touches étendues (Shift+Entrée & co.)
    const tset =
      `tmux set -g allow-passthrough on 2>/dev/null; ` +
      `tmux set -s extended-keys on 2>/dev/null; ` +
      `tmux set -as terminal-features 'xterm*:extkeys' 2>/dev/null; `;
    return (
      `export PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"; ` + // exec SSH non-interactif = PATH minimal
      `if command -v tmux >/dev/null 2>&1; then ` +
      `if tmux has-session -t ${n} 2>/dev/null; then ${setenv}exec tmux -u attach-session -t ${n}; ` +
      `else tmux -u new-session -d -s ${n} && ${tset}${send}${setenv}exec tmux -u attach-session -t ${n}; fi; ` +
      `else exec "$SHELL" -l; fi`
    );
  }

  /** Crée un xterm configuré (addons, presse-papier, raccourcis). `send` route la saisie. */
  function setupTerm(sid: string, send: (data: string) => void) {
    const term = new Terminal(termOptions());
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.loadAddon(new WebLinksAddon());
    term.onSelectionChange(() => {
      const sel = term.getSelection();
      if (sel && settings.copyOnSelect && inTauri) writeText(sel).catch(() => {});
    });
    term.attachCustomKeyEventHandler((e) => {
      // Shift+Entrée → nouvelle ligne sans envoyer (attendu par Claude Code) :
      // on émet ESC+CR, reconnu comme saut de ligne même à travers tmux
      if (e.type === "keydown" && e.key === "Enter" && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
        send("\x1b\r");
        return false;
      }
      if (e.type !== "keydown" || !e.metaKey) return true;
      if (e.key === "Enter" && e.shiftKey) {
        toggleZoom(sid); // ⌘⇧Entrée : plein écran du panneau
        return false;
      }
      if (e.key === "v") {
        if (inTauri) readText().then((t) => t && term.paste(t)).catch(() => {});
        return false;
      }
      if (e.key === "f") {
        openSearch(sid);
        return false;
      }
      // laisse les accélérateurs du menu natif / la palette (⌘P) agir sans que xterm n'envoie de séquence
      if (["t", "w", "d", "k", "b", ",", "p"].includes(e.key)) return false;
      return true;
    });
    term.onData(send);
    return { term, fit, search };
  }

  function newSession(remote: Remote, cmd = "", key?: string): string {
    const sid = crypto.randomUUID();
    const { term, fit, search } = setupTerm(sid, (data) => rpc("ssh_write", { sessionId: sid, data }));
    term.onResize(({ cols, rows }) => rpc("ssh_resize", { sessionId: sid, cols, rows }));
    sessions.set(sid, { term, fit, search, remote, cmd, key: key ?? sid, unlisteners: [], webgl: false });
    sessStatus[sid] = { status: "connecting", error: "" };
    connectSession(sid);
    return sid;
  }

  async function connectSession(sid: string) {
    const s = sessions.get(sid);
    if (!s) return;
    sessStatus[sid] = { status: "connecting", error: "" };
    if (s.remote.id === "local") {
      try {
        await rpc("local_connect", { sessionId: sid, cols: s.term.cols, rows: s.term.rows });
      } catch (e) {
        sessStatus[sid] = { status: "error", error: String(e) };
        return;
      }
    } else {
      const authKind: AuthKind = s.remote.auth ?? "key";
      let keyPath = "";
      let identityId = s.remote.id; // sert de clé Keychain pour l'auth par mot de passe
      if (authKind === "key") {
        const identity = identities.find((i) => i.id === s.remote.identityId);
        if (!identity) {
          sessStatus[sid] = { status: "error", error: "Ce remote n'a pas d'identité valide." };
          return;
        }
        keyPath = identity.keyPath;
        identityId = identity.id;
      }
      const useTmux = s.remote.tmux !== false; // activé par défaut : survie aux déconnexions
      let execCmd: string | null = null;
      if (useTmux) {
        // la sync config doit précéder le premier lancement de claude (hooks)
        if (s.remote.claude && !syncedRemotes.has(s.remote.id)) {
          try {
            const msg = await rpc<string>("claude_sync", remoteParams(s.remote));
            syncedRemotes.add(s.remote.id);
            s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
          } catch (e) {
            s.term.write(`\x1b[31m[arabel] sync échouée : ${e}\x1b[0m\r\n`);
          }
        }
        const init: string[] = [];
        if (s.remote.claude) init.push(`export ARABEL_PANE=${sid}`);
        if (s.cmd) init.push(s.cmd);
        else if (s.remote.claude) init.push("claude");
        execCmd = tmuxCmd(`arabel-${s.key.slice(0, 8)}`, init.join("; "), s.remote.claude ? sid : null);
      }
      try {
        await rpc("ssh_connect", {
          sessionId: sid,
          host: s.remote.host,
          port: Number(s.remote.port),
          user: s.remote.user,
          keyPath,
          passphrase: null,
          identityId,
          auth: authKind,
          cols: s.term.cols,
          rows: s.term.rows,
          execCmd,
        });
      } catch (e) {
        sessStatus[sid] = { status: "error", error: String(e) };
        return;
      }
      s.tmux = useTmux;
    }
    sessStatus[sid] = { status: "open", error: "" };
    s.unlisteners.push(
      await listen<string>(`ssh-output-${sid}`, (ev) => s.term.write(b64ToBytes(ev.payload))),
      await listen(`ssh-closed-${sid}`, () => {
        if (!sessStatus[sid]) return; // fermeture volontaire déjà nettoyée
        sessStatus[sid] = { status: "closed", error: "" };
        s.unlisteners.forEach((u) => u());
        s.unlisteners = [];
        if (s.remote.id !== "local") scheduleReconnect(sid); // auto-reconnexion
      }),
    );
    // taille réelle du pane (la connexion est partie en 80x24 par défaut)
    s.fit.fit();
    rpc("ssh_resize", { sessionId: sid, cols: s.term.cols, rows: s.term.rows });
    if (s.remote.id !== "local") {
      rpc("metrics_watch", { remoteId: s.remote.id, ...remoteParams(s.remote) }).catch(() => {});
      if (s.remote.claude) rpc("events_watch", { remoteId: s.remote.id, ...remoteParams(s.remote) }).catch(() => {});
      if (!s.tmux) {
        // sans tmux : init envoyé dans le shell après connexion (comportement historique)
        if (s.remote.claude) await rpc("ssh_write", { sessionId: sid, data: `export ARABEL_PANE=${sid}\n` });
        if (s.cmd) await rpc("ssh_write", { sessionId: sid, data: s.cmd + "\n" });
        else if (s.remote.claude) claudeSetup(sid, s.remote);
      }
    } else {
      if (!inTauri) {
        metrics["local"] = { load: 3.2, cpus: 10, memTotal: 34359738368, memUsed: 21474836480, diskTotal: 994662584320, diskUsed: 703687441776 };
      }
      if (s.cmd) await rpc("ssh_write", { sessionId: sid, data: s.cmd + "\n" });
    }
    if (!inTauri) {
      s.term.write(DEMO_OUTPUT);
      if (s.remote.id !== "local")
        metrics[s.remote.id] = { load: 1.4, cpus: 4, memTotal: 8321499136, memUsed: 5100273664, diskTotal: 84825923584, diskUsed: 39728447488 };
      // démo : simule une demande de permission Claude (état « attend » du dashboard)
      if (s.remote.claude)
        setTimeout(() => { attentions = [...attentions, { id: crypto.randomUUID(), sid, remoteId: s.remote.id, kind: "notif" as const, message: "Autoriser npm test ?" }].slice(-20); }, 1200);
    }
  }

  /** Reconnexion automatique avec backoff après une coupure non voulue. */
  function scheduleReconnect(sid: string, attempt = 1) {
    if (attempt > 5) return;
    setTimeout(async () => {
      const st = sessStatus[sid];
      if (!st || (st.status !== "closed" && st.status !== "error")) return; // fermé ou déjà reconnecté
      await connectSession(sid);
      if (sessStatus[sid] && sessStatus[sid].status !== "open") scheduleReconnect(sid, attempt + 1);
    }, Math.min(15000, 1500 * 2 ** (attempt - 1)));
  }

  function removeSession(sid: string) {
    const s = sessions.get(sid);
    if (!s) return;
    sessions.delete(sid);
    if (zoomedSid === sid) zoomedSid = null;
    delete sessStatus[sid];
    s.unlisteners.forEach((u) => u());
    s.term.dispose();
    attentions = attentions.filter((a) => a.sid !== sid);
    if (![...sessions.values()].some((x) => x.remote.id === s.remote.id)) {
      rpc("events_unwatch", { remoteId: s.remote.id }).catch(() => {});
      rpc("metrics_unwatch", { remoteId: s.remote.id }).catch(() => {});
      delete metrics[s.remote.id];
    }
    for (const t of tabs) {
      if (t.root && leaves(t.root).includes(sid)) {
        t.root = withoutLeaf(t.root, sid);
        if (t.active === sid) t.active = firstLeaf(t.root);
        if (!t.root && tabs.length > 1) closeTab(t);
      }
    }
  }

  async function closePane(sid: string) {
    if (sessions.get(sid)?.cc) return ccKill(sid); // panneau tmux : kill-pane
    const st = sessStatus[sid]?.status;
    if (st === "open") {
      try {
        await rpc("ssh_disconnect", { sessionId: sid });
        if (inTauri) {
          removeSession(sid); // l'event ssh-closed nettoie aussi, removeSession est idempotent
          return;
        }
      } catch {
        /* déjà morte */
      }
    }
    removeSession(sid);
  }

  function openInTab(tab: Tab, remote: Remote) {
    const sid = newSession(remote);
    tab.root = { leaf: sid };
    tab.active = sid;
  }
  function openRemote(remote: Remote) {
    const tab = activeTab && !activeTab.root ? activeTab : null;
    if (tab) openInTab(tab, remote);
    else {
      newTab();
      openInTab(tabs[tabs.length - 1], remote);
    }
  }

  // ─── mode contrôle tmux (tmux -CC) : panneaux natifs miroir de tmux ───────
  type CcSession = {
    ctrlSid: string; remote: Remote; tabId: string; ctrl: TmuxControl;
    unlisteners: UnlistenFn[]; pending: ((lines: string[], error: boolean) => void)[];
    windows: Map<string, Lay>; activeWindow: string | null; winName: Record<string, string>;
  };
  const ccSessions = new Map<string, CcSession>();
  const ccSid = (ctrlSid: string, paneId: string) => `cc:${ctrlSid}:${paneId}`;
  const ccOf = (sid: string): CcSession | undefined => {
    const c = sessions.get(sid)?.cc;
    return c ? ccSessions.get(c.ctrlSid) : undefined;
  };
  const ccPaneId = (sid: string) => sessions.get(sid)?.cc?.paneId ?? "";
  function ccExec(cc: CcSession, cmd: string, handler?: (lines: string[], error: boolean) => void) {
    cc.pending.push(handler ?? (() => {}));
    rpc("ssh_write", { sessionId: cc.ctrlSid, data: cmd + "\n" });
  }
  function ccSplit(sid: string, dir: "h" | "v") {
    const cc = ccOf(sid);
    if (cc) ccExec(cc, `split-window ${dir === "h" ? "-h" : "-v"} -t %${ccPaneId(sid)}`);
  }
  function ccKill(sid: string) {
    const cc = ccOf(sid);
    if (cc) ccExec(cc, `kill-pane -t %${ccPaneId(sid)}`);
  }
  function ccEnsurePane(cc: CcSession, paneId: string): string {
    const sid = ccSid(cc.ctrlSid, paneId);
    if (sessions.has(sid)) return sid;
    const { term, fit, search } = setupTerm(sid, (data) =>
      rpc("ssh_write", { sessionId: cc.ctrlSid, data: `send-keys -t %${paneId} -H ${toHexKeys(data)}\n` }),
    );
    sessions.set(sid, { term, fit, search, remote: cc.remote, cmd: "", key: sid, unlisteners: [], webgl: false, cc: { ctrlSid: cc.ctrlSid, paneId } });
    sessStatus[sid] = { status: "open", error: "" };
    return sid;
  }
  function ccApplyLayout(cc: CcSession, winId: string, lay: Lay) {
    cc.windows.set(winId, lay);
    if (cc.activeWindow == null) cc.activeWindow = winId;
    // supprime les xterms des panneaux disparus (kill-pane côté tmux)
    const live = new Set([...cc.windows.values()].flatMap(layPanes));
    for (const [sid, s] of [...sessions]) {
      if (s.cc?.ctrlSid === cc.ctrlSid && !live.has(s.cc.paneId)) {
        s.term.dispose();
        sessions.delete(sid);
        delete sessStatus[sid];
      }
    }
    if (winId !== cc.activeWindow) return;
    layPanes(lay).forEach((p) => ccEnsurePane(cc, p));
    const root = layToTree(lay, (p) => ({ leaf: ccSid(cc.ctrlSid, p) })) as PaneNode;
    const tab = tabs.find((t) => t.id === cc.tabId);
    if (tab) {
      tab.root = root;
      if (!tab.active || !leaves(root).includes(tab.active)) tab.active = firstLeaf(root);
    }
    requestAnimationFrame(() => leaves(root).forEach((s) => sessions.get(s)?.fit.fit()));
  }
  function ccResize(cc: CcSession) {
    const el = document.querySelector(`[data-tab="${cc.tabId}"]`) as HTMLElement | null;
    if (!el || !el.clientWidth) return;
    // taille client approximative en cellules → tmux relaie un %layout-change
    const cols = Math.max(20, Math.floor(el.clientWidth / 8));
    const rows = Math.max(5, Math.floor(el.clientHeight / 18));
    rpc("ssh_write", { sessionId: cc.ctrlSid, data: `refresh-client -C ${cols}x${rows}\n` });
  }
  async function openTmuxNative(remote: Remote) {
    if (remote.id === "local") return toast("Le mode tmux natif est pour les remotes SSH.", "error");
    const authKind: AuthKind = remote.auth ?? "key";
    let keyPath = "";
    let identityId = remote.id;
    if (authKind === "key") {
      const identity = identities.find((i) => i.id === remote.identityId);
      if (!identity) return toast("Ce remote n'a pas d'identité valide.", "error");
      keyPath = identity.keyPath;
      identityId = identity.id;
    }
    const ctrlSid = crypto.randomUUID();
    const tabId = crypto.randomUUID();
    const cc: CcSession = { ctrlSid, remote, tabId, ctrl: null as unknown as TmuxControl, unlisteners: [], pending: [], windows: new Map(), activeWindow: null, winName: {} };
    cc.ctrl = new TmuxControl({
      output: (pane, bytes) => sessions.get(ccSid(ctrlSid, pane))?.term.write(bytes),
      layout: (win, tree) => ccApplyLayout(cc, win, tree),
      windowClose: (win) => {
        cc.windows.delete(win);
        if (cc.activeWindow === win) {
          cc.activeWindow = cc.windows.keys().next().value ?? null;
          const l = cc.activeWindow ? cc.windows.get(cc.activeWindow) : null;
          if (cc.activeWindow && l) ccApplyLayout(cc, cc.activeWindow, l);
          else closeCc(cc);
        }
      },
      windowRenamed: (win, name) => (cc.winName[win] = name),
      paneActive: (win, pane) => {
        const tab = tabs.find((t) => t.id === tabId);
        if (tab && win === cc.activeWindow) tab.active = ccSid(ctrlSid, pane);
      },
      reply: (lines, error) => cc.pending.shift()?.(lines, error),
      exit: () => closeCc(cc),
    });
    ccSessions.set(ctrlSid, cc);
    tabs.push({ id: tabId, root: null, active: null, projectId: null, cc: ctrlSid });
    activeTabId = tabId;

    const name = `arabel-cc-${remote.id.slice(0, 8)}`;
    const execCmd = `export PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"; exec tmux -CC new-session -A -s ${name}`;
    try {
      await rpc("ssh_connect", {
        sessionId: ctrlSid, host: remote.host, port: Number(remote.port), user: remote.user,
        keyPath, passphrase: null, identityId, auth: authKind, cols: 200, rows: 50, execCmd,
      });
    } catch (e) {
      toast(`tmux natif : ${e}`, "error");
      return closeCc(cc);
    }
    cc.unlisteners.push(
      await listen<string>(`ssh-output-${ctrlSid}`, (ev) => cc.ctrl.feed(atob(ev.payload))),
      await listen(`ssh-closed-${ctrlSid}`, () => closeCc(cc)),
    );
    ccResize(cc);
    // amorçage : récupère les fenêtres et leur layout courant
    ccExec(cc, `list-windows -F "#{window_id}|#{window_active}|#{window_layout}"`, (lines) => {
      for (const l of lines) {
        const [win, active, layout] = l.split("|");
        if (!win || !layout) continue;
        const w = win.replace(/^@/, "");
        if (active === "1") cc.activeWindow = w;
        try { ccApplyLayout(cc, w, parseLayout(layout)); } catch { /* layout illisible */ }
      }
    });
  }
  function closeCc(cc: CcSession) {
    if (!ccSessions.has(cc.ctrlSid)) return;
    ccSessions.delete(cc.ctrlSid);
    cc.unlisteners.forEach((u) => u());
    rpc("ssh_disconnect", { sessionId: cc.ctrlSid }).catch(() => {});
    for (const [sid, s] of [...sessions]) {
      if (s.cc?.ctrlSid === cc.ctrlSid) {
        s.term.dispose();
        sessions.delete(sid);
        delete sessStatus[sid];
      }
    }
    tabs = tabs.filter((t) => t.id !== cc.tabId);
    ensureOneTab();
  }
  function openPicker(opts: { tabId?: string | null; sid?: string | null; dir?: "h" | "v" | null; projectId?: string | null } = {}) {
    modal = { type: "picker", tabId: opts.tabId ?? null, sid: opts.sid ?? null, dir: opts.dir ?? null, projectId: opts.projectId ?? null, filter: "" };
  }
  function doPick(remote: Remote) {
    if (modal?.type !== "picker") return;
    const m = modal;
    modal = null;
    if (m.cc && !m.dir && !m.projectId && remote.id !== "local") {
      openTmuxNative(remote);
      return;
    }
    if (m.projectId) {
      const p = projects.find((x) => x.id === m.projectId);
      if (!p) return;
      let tab = openTabFor(p.id);
      if (!tab) {
        openProject(p);
        tab = openTabFor(p.id);
      }
      if (!tab) return;
      const sid = newSession(remote);
      addPaneToTab(tab, sid);
      activeTabId = tab.id;
      persistProject(p);
    } else if (m.dir && m.sid && m.tabId) {
      const tab = tabs.find((t) => t.id === m.tabId);
      if (!tab?.root) return;
      const newSid = newSession(remote);
      tab.root = withSplit(tab.root, m.sid, m.dir, newSid);
      tab.active = newSid;
      if (tab.projectId) persistProject(projects.find((p) => p.id === tab.projectId)!);
    } else if (m.tabId) {
      const tab = tabs.find((t) => t.id === m.tabId);
      if (tab && !tab.root) openInTab(tab, remote);
      else openRemote(remote);
    } else {
      openRemote(remote);
    }
  }

  // ─── déplacement de panneaux (drag & drop, + sur projet) ─────────────────
  let dragSid = $state<string | null>(null);
  let dropTarget = $state<string | null>(null); // id de projet ou "standalone"
  let dropRow = $state<{ sid: string; after: boolean } | null>(null); // réordonnancement

  function ensureOneTab() {
    if (!tabs.length) tabs.push({ id: crypto.randomUUID(), root: null, active: null, projectId: null });
    if (!tabs.some((t) => t.id === activeTabId)) activeTabId = tabs[tabs.length - 1].id;
  }
  /** Retire le panneau de son onglet SANS fermer la session. */
  function extractPane(sid: string) {
    const tab = tabs.find((t) => leaves(t.root).includes(sid));
    if (!tab) return;
    tab.root = tab.root ? withoutLeaf(tab.root, sid) : null;
    if (tab.active === sid) tab.active = firstLeaf(tab.root);
    if (!tab.root) tabs = tabs.filter((x) => x.id !== tab.id);
    ensureOneTab();
  }
  function addPaneToTab(tab: Tab, sid: string) {
    tab.root = tab.root ? { dir: "h", ratio: 0.5, a: tab.root, b: { leaf: sid } } : { leaf: sid };
    tab.active = sid;
  }
  /** Ré-enregistre le layout ouvert d'un projet dans sa définition. */
  function persistProject(p: Project) {
    const tab = openTabFor(p.id);
    if (tab?.root) {
      const root = serializeTree(tab.root);
      if (root) p.root = root;
    }
    save();
  }
  function movePaneToProject(sid: string, p: Project) {
    let tab = openTabFor(p.id);
    if (!tab) {
      openProject(p);
      tab = openTabFor(p.id);
    }
    if (!tab || leaves(tab.root).includes(sid)) return;
    const from = tabs.find((t) => leaves(t.root).includes(sid));
    const fromProject = from?.projectId ? projects.find((x) => x.id === from.projectId) : null;
    extractPane(sid);
    addPaneToTab(tab, sid);
    activeTabId = tab.id;
    if (fromProject) persistProject(fromProject);
    persistProject(p);
  }
  function movePaneToStandalone(sid: string) {
    const from = tabs.find((t) => leaves(t.root).includes(sid));
    if (!from?.projectId) return; // déjà hors projet
    const proj = projects.find((x) => x.id === from.projectId);
    extractPane(sid);
    const tab: Tab = { id: crypto.randomUUID(), root: { leaf: sid }, active: sid, projectId: null };
    tabs.push(tab);
    activeTabId = tab.id;
    if (proj) persistProject(proj);
  }
  function handleDrop(target: string) {
    const sid = dragSid;
    dragSid = null;
    dropTarget = null;
    if (!sid || !sessions.has(sid)) return;
    if (target === "standalone") movePaneToStandalone(sid);
    else {
      const p = projects.find((x) => x.id === target);
      if (p) movePaneToProject(sid, p);
    }
  }
  function dropzone(target: string) {
    return {
      ondragover: (e: DragEvent) => {
        if (!dragSid) return;
        e.preventDefault();
        e.dataTransfer!.dropEffect = "move";
        dropTarget = target;
      },
      ondragleave: () => {
        if (dropTarget === target) dropTarget = null;
      },
      ondrop: (e: DragEvent) => {
        e.preventDefault();
        handleDrop(target);
      },
    };
  }

  // réordonnancement d'un terminal autonome : l'ordre de la liste suit l'ordre
  // des onglets, donc déplacer l'onglet suffit
  function isStandalone(sid: string): boolean {
    const t = tabs.find((x) => leaves(x.root).includes(sid));
    return !!t && !t.projectId;
  }
  function reorderTerminal(fromSid: string, toSid: string, after: boolean) {
    if (fromSid === toSid) return;
    const fromTab = tabs.find((t) => leaves(t.root).includes(fromSid));
    const toTab = tabs.find((t) => leaves(t.root).includes(toSid));
    if (!fromTab || !toTab || fromTab === toTab) return; // même onglet (split) : rien à faire
    tabs.splice(tabs.indexOf(fromTab), 1);
    tabs.splice(tabs.indexOf(toTab) + (after ? 1 : 0), 0, fromTab);
  }
  /** Handlers de dépôt sur une ligne de terminal pour la réordonner (section autonome). */
  function reorderzone(sid: string, sub: boolean) {
    if (sub) return {}; // les panneaux d'un projet ne se réordonnent pas ainsi
    return {
      ondragover: (e: DragEvent) => {
        // seulement entre terminaux autonomes ; sinon on laisse la section gérer
        // (déplacer-vers-standalone) ou la ligne cible n'est pas concernée
        if (!dragSid || dragSid === sid || !isStandalone(dragSid)) return;
        e.preventDefault();
        e.stopPropagation();
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        dropRow = { sid, after: e.clientY > r.top + r.height / 2 };
        dropTarget = null;
      },
      ondragleave: () => {
        if (dropRow?.sid === sid) dropRow = null;
      },
      ondrop: (e: DragEvent) => {
        if (!dragSid || !isStandalone(dragSid)) return;
        e.preventDefault();
        e.stopPropagation();
        reorderTerminal(dragSid, sid, dropRow?.after ?? false);
        dragSid = null;
        dropRow = null;
        dropTarget = null;
      },
    };
  }

  // ─── projets ──────────────────────────────────────────────────────────────
  function serializeTree(node: PaneNode): ProjNode | null {
    if ("leaf" in node) {
      const s = sessions.get(node.leaf);
      // les panneaux tmux natifs se réattachent via tmux -CC, pas via le store
      return s && !s.cc ? { remoteId: s.remote.id, cmd: s.cmd, id: s.key } : null;
    }
    const a = serializeTree(node.a);
    const b = serializeTree(node.b);
    if (!a) return b;
    if (!b) return a;
    return { dir: node.dir, ratio: node.ratio, a, b };
  }
  function projLeaves(n: ProjNode): ProjLeaf[] {
    return "remoteId" in n ? [n] : [...projLeaves(n.a), ...projLeaves(n.b)];
  }
  function projRemote(remoteId: string): Remote | undefined {
    return remoteId === "local" ? LOCAL : remotes.find((r) => r.id === remoteId);
  }
  function buildNode(n: ProjNode): PaneNode | null {
    if ("remoteId" in n) {
      const remote = projRemote(n.remoteId);
      if (!remote) return null;
      return { leaf: newSession(remote, n.cmd, n.id) };
    }
    const a = buildNode(n.a);
    const b = buildNode(n.b);
    if (!a) return b;
    if (!b) return a;
    return { dir: n.dir, ratio: n.ratio ?? 0.5, a, b };
  }
  function openProject(p: Project) {
    const root = buildNode(p.root);
    if (!root) {
      toast("Aucun remote de ce projet n'existe encore.", "error");
      return;
    }
    const tab: Tab = { id: crypto.randomUUID(), root, active: firstLeaf(root), projectId: p.id };
    tabs.push(tab);
    activeTabId = tab.id;
  }
  async function confirmSaveProject() {
    if (modal?.type !== "saveProject") return;
    const { tabId, name } = modal;
    modal = null;
    const tab = tabs.find((t) => t.id === tabId);
    if (!tab?.root) return;
    const root = serializeTree(tab.root);
    if (!root) return;
    const existing = tab.projectId && projects.find((p) => p.id === tab.projectId);
    if (existing) {
      existing.name = name;
      existing.root = root;
    } else {
      const p: Project = { id: crypto.randomUUID(), name, root };
      projects = [...projects, p];
      tab.projectId = p.id;
    }
    toast(`Projet « ${name} » enregistré`, "success");
    await save();
  }

  // ─── CRUD remotes / identités / projets ───────────────────────────────────
  function editRemote(r?: Remote, back = false) {
    modal = {
      type: "remote",
      back,
      password: "",
      data: r
        ? { auth: "key", ...r }
        : { id: crypto.randomUUID(), name: "", host: "", port: 22, user: "root", identityId: identities[0]?.id ?? "", auth: "key", claude: false, tmux: true },
    };
  }
  async function saveRemote() {
    if (modal?.type !== "remote") return;
    const r = modal.data;
    if (!r.name) r.name = `${r.user}@${r.host}`;
    // mot de passe → Keychain sous l'id du remote, jamais dans le store
    if (r.auth === "password" && modal.password) {
      await rpc("passphrase_set", { identityId: r.id, passphrase: modal.password });
    }
    remotes = [...remotes.filter((x) => x.id !== r.id), r];
    modal = modal.back ? { type: "connections" } : null;
    await save();
  }
  async function deleteRemote(r: Remote) {
    if (r.auth === "password") await rpc("passphrase_delete", { identityId: r.id }).catch(() => {});
    remotes = remotes.filter((x) => x.id !== r.id);
    projects = projects.filter((p) => projLeaves(p.root).some((l) => remotes.some((x) => x.id === l.remoteId)));
    confirmDeleteId = null;
    await save();
  }
  async function openSshImport() {
    try {
      const hosts = (await rpc<ImportHost[]>("ssh_config_parse")) ?? [];
      const fresh = hosts.filter((h) => !remotes.some((r) => r.host === h.hostName && r.user === h.user));
      if (!fresh.length) return toast("Aucun nouvel hôte dans ~/.ssh/config", "info");
      modal = { type: "sshImport", hosts: fresh, back: true };
    } catch (e) {
      toast(String(e), "error");
    }
  }
  function importHost(h: ImportHost) {
    // une IdentityFile → auth par clé (identité réutilisée ou créée), sinon ssh-agent
    let auth: AuthKind = "agent";
    let identityId = "";
    if (h.identityFile) {
      const existing = identities.find((i) => i.keyPath === h.identityFile);
      if (existing) identityId = existing.id;
      else {
        const id: Identity = { id: crypto.randomUUID(), name: h.identityFile.split("/").pop() ?? "clé", keyPath: h.identityFile, hasPassphrase: false };
        identities = [...identities, id];
        identityId = id.id;
      }
      auth = "key";
    }
    remotes = [...remotes, { id: crypto.randomUUID(), name: h.host, host: h.hostName, port: h.port, user: h.user || "root", identityId, auth, tmux: true }];
    if (modal?.type === "sshImport") modal = { ...modal, hosts: modal.hosts.filter((x) => x !== h) };
    save();
  }
  function editIdentity(i?: Identity, back = false) {
    modal = {
      type: "identity",
      back,
      data: i ? { ...i } : { id: crypto.randomUUID(), name: "", keyPath: "~/.ssh/id_ed25519", hasPassphrase: false },
      passphrase: "",
    };
  }
  async function saveIdentity() {
    if (modal?.type !== "identity") return;
    const i = modal.data;
    if (!i.name) i.name = i.keyPath.split("/").pop() ?? "clé";
    if (modal.passphrase) {
      await rpc("passphrase_set", { identityId: i.id, passphrase: modal.passphrase });
      i.hasPassphrase = true;
    }
    identities = [...identities.filter((x) => x.id !== i.id), i];
    modal = modal.back ? { type: "connections" } : null;
    await save();
  }
  async function deleteIdentity(i: Identity) {
    if (remotes.some((r) => r.identityId === i.id)) {
      toast(`« ${i.name} » est utilisée par un remote.`, "error");
      confirmDeleteId = null;
      return;
    }
    await rpc("passphrase_delete", { identityId: i.id });
    identities = identities.filter((x) => x.id !== i.id);
    confirmDeleteId = null;
    await save();
  }
  async function saveProjectEdit() {
    if (modal?.type !== "project") return;
    const p = modal.data;
    projects = [...projects.filter((x) => x.id !== p.id), p];
    modal = null;
    await save();
  }
  async function deleteProject(p: Project) {
    projects = projects.filter((x) => x.id !== p.id);
    for (const t of tabs) if (t.projectId === p.id) t.projectId = null;
    confirmDeleteId = null;
    await save();
  }

  function applySettings() {
    for (const s of sessions.values()) {
      s.term.options.fontSize = settings.fontSize;
      s.term.options.fontFamily = settings.fontFamily;
      s.term.options.theme = activeTheme();
      s.fit.fit();
    }
    save();
  }

  // ─── sync config Claude Code ──────────────────────────────────────────────
  const syncedRemotes = new Set<string>();
  function remoteParams(remote: Remote) {
    const auth: AuthKind = remote.auth ?? "key";
    if (auth !== "key") {
      // password : identityId = id du remote (clé Keychain) ; agent : ignoré
      return { host: remote.host, port: Number(remote.port), user: remote.user, keyPath: "", identityId: remote.id, auth };
    }
    const identity = identities.find((i) => i.id === remote.identityId)!;
    return { host: remote.host, port: Number(remote.port), user: remote.user, keyPath: identity.keyPath, identityId: identity.id, auth };
  }
  async function claudeSetup(sid: string, remote: Remote) {
    const s = sessions.get(sid);
    if (!s) return;
    try {
      if (!syncedRemotes.has(remote.id)) {
        s.term.write("\x1b[90m[arabel] sync config Claude Code…\x1b[0m\r\n");
        const msg = await rpc<string>("claude_sync", remoteParams(remote));
        syncedRemotes.add(remote.id);
        s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
      }
      await rpc("ssh_write", { sessionId: sid, data: "claude\n" });
    } catch (e) {
      s.term.write(`\x1b[31m[arabel] sync échouée : ${e}\x1b[0m\r\n`);
      toast(`Sync Claude échouée : ${e}`, "error");
    }
  }
  async function syncNow(sid: string) {
    const s = sessions.get(sid);
    if (!s || s.remote.id === "local") return;
    s.term.write("\r\n\x1b[90m[arabel] injection de la config…\x1b[0m\r\n");
    try {
      const msg = await rpc<string>("claude_sync", remoteParams(s.remote));
      syncedRemotes.add(s.remote.id);
      s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
      toast("Config Claude synchronisée", "success");
    } catch (e) {
      s.term.write(`\x1b[31m[arabel] ${e}\x1b[0m\r\n`);
      toast(String(e), "error");
    }
  }

  // ─── attentions (hooks Claude Code) ──────────────────────────────────────
  type Attention = { id: string; sid: string; remoteId: string; kind: "stop" | "notif"; message: string };
  let attentions = $state<Attention[]>([]);
  let notifOk = false;
  if (inTauri) {
    (async () => {
      notifOk = (await isPermissionGranted()) || (await requestPermission()) === "granted";
    })();
  }

  listen<{ remoteId: string; line: string }>("arabel-hook", (ev) => {
    let parsed: any = {};
    try {
      parsed = JSON.parse(ev.payload.line);
    } catch {
      return;
    }
    const hook = parsed.event ?? {};
    const kind: "stop" | "notif" = hook.hook_event_name === "Stop" ? "stop" : "notif";
    const sid: string = parsed.pane ?? "";
    const remoteName = remotes.find((r) => r.id === ev.payload.remoteId)?.name ?? "remote";
    const message = hook.message ?? (kind === "stop" ? "Claude a terminé" : "Claude attend une réponse");
    if (kind === "stop") attentions = attentions.filter((a) => a.sid !== sid || a.kind !== "notif");
    attentions = [...attentions, { id: crypto.randomUUID(), sid, remoteId: ev.payload.remoteId, kind, message }].slice(-20);
    if (notifOk) sendNotification({ title: `Arabel — ${remoteName}`, body: message });
  });

  // badge sur l'icône du Dock
  $effect(() => {
    const n = attentions.filter((a) => a.kind === "notif").length;
    if (inTauri) getCurrentWindow().setBadgeCount(n || undefined).catch(() => {});
  });

  function attentionTarget(a: Attention): string | null {
    if (a.sid && sessions.has(a.sid)) return a.sid;
    for (const [sid, s] of sessions) if (s.remote.id === a.remoteId) return sid;
    return null;
  }
  function gotoAttention(a: Attention) {
    const sid = attentionTarget(a);
    if (!sid) return dismissAttention(a);
    const tab = tabs.find((t) => leaves(t.root).includes(sid));
    if (tab) {
      activeTabId = tab.id;
      tab.active = sid;
      sessions.get(sid)?.term.focus();
    }
  }
  function answerAttention(a: Attention, keys: string) {
    const sid = attentionTarget(a);
    if (sid) rpc("ssh_write", { sessionId: sid, data: keys });
    dismissAttention(a);
  }
  function dismissAttention(a: Attention) {
    attentions = attentions.filter((x) => x.id !== a.id);
  }
  function remoteAttention(rid: string): boolean {
    return attentions.some((a) => a.remoteId === rid);
  }

  // ─── dashboard agents : sessions Claude et leur état (dérivé des hooks) ────
  type Agent = { sid: string; tab: Tab; name: string; status: "waiting" | "working" | "done"; message: string; notif?: Attention };
  const agents = $derived.by(() => {
    const list: Agent[] = [];
    for (const t of tabs) {
      for (const sid of leaves(t.root)) {
        const s = sessions.get(sid);
        if (!s || !s.remote.claude || s.cc) continue; // panneaux claude non-tmux-natif
        const mine = attentions.filter((a) => attentionTarget(a) === sid);
        const notif = mine.find((a) => a.kind === "notif");
        const stop = mine.find((a) => a.kind === "stop");
        const status = notif ? "waiting" : stop ? "done" : "working";
        list.push({ sid, tab: t, name: s.remote.name, status, message: (notif ?? stop)?.message ?? (s.cmd || "session claude"), notif });
      }
    }
    return list;
  });
  const waitingCount = $derived(agents.filter((a) => a.status === "waiting").length);

  // ─── métriques de l'hôte (façon MobaXterm) ───────────────────────────────
  type Metrics = { load: number; cpus: number; memTotal: number; memUsed: number; diskTotal: number; diskUsed: number };
  let metrics = $state<Record<string, Metrics>>({});
  const MB = 1048576;

  listen<{ remoteId: string; line: string }>("arabel-metrics", (ev) => {
    const p = ev.payload.line.trim().split(/\s+/);
    if (p[0] !== "M" || p.length < 7) return;
    metrics[ev.payload.remoteId] = {
      load: +p[1], cpus: +p[2],
      memTotal: +p[3] * MB, memUsed: +p[4] * MB,
      diskTotal: +p[5] * MB, diskUsed: +p[6] * MB,
    };
  });

  // machine locale : sondée tant qu'une session locale existe
  setInterval(async () => {
    if (![...sessions.values()].some((s) => s.remote.id === "local")) return;
    try {
      metrics["local"] = (await rpc<Metrics>("local_metrics")) ?? metrics["local"];
    } catch {}
  }, 3000);

  const activeMetrics = $derived.by(() => {
    const sid = activeTab?.active;
    const rid = sid ? sessions.get(sid)?.remote.id : null;
    return rid ? metrics[rid] : undefined;
  });
  function gb(bytes: number): string {
    return (bytes / 1073741824).toFixed(1);
  }

  // ─── panneau fichiers SFTP ────────────────────────────────────────────────
  type FEntry = { name: string; isDir: boolean; size: number };
  let files = $state<{ open: boolean; remote: Remote | null; path: string; entries: FEntry[]; busy: boolean; over: boolean }>({
    open: false, remote: null, path: "", entries: [], busy: false, over: false,
  });

  function activeSshRemote(): Remote | null {
    const sid = activeTab?.active;
    const r = sid ? sessions.get(sid)?.remote : null;
    return r && r.id !== "local" ? r : null;
  }
  function joinPath(p: string, n: string): string {
    return p === "/" ? `/${n}` : `${p}/${n}`;
  }
  function parentPath(p: string): string {
    return p.split("/").slice(0, -1).join("/") || "/";
  }
  function humanSize(n: number): string {
    if (n < 1024) return `${n} o`;
    if (n < 1048576) return `${(n / 1024).toFixed(0)} Ko`;
    if (n < 1073741824) return `${(n / 1048576).toFixed(1)} Mo`;
    return `${(n / 1073741824).toFixed(2)} Go`;
  }
  async function filesLoad(path?: string) {
    const r = files.remote;
    if (!r) return;
    files.busy = true;
    try {
      const p = path ?? (await rpc<string>("sftp_home", { remoteId: r.id, ...remoteParams(r) }));
      const entries = await rpc<FEntry[]>("sftp_list", { remoteId: r.id, ...remoteParams(r), path: p });
      files.path = p;
      files.entries = entries.sort((a, b) => (a.isDir !== b.isDir ? (a.isDir ? -1 : 1) : a.name.localeCompare(b.name)));
    } catch (e) {
      toast(String(e), "error");
    }
    files.busy = false;
  }
  function toggleFiles() {
    if (files.open) {
      files.open = false;
      return;
    }
    const r = activeSshRemote();
    if (!r) return;
    files.open = true;
    files.remote = r;
    filesLoad();
  }
  // suit le remote du panneau actif
  $effect(() => {
    const r = activeSshRemote();
    if (files.open && r && r.id !== files.remote?.id) {
      files.remote = r;
      filesLoad();
    }
  });
  async function fileDownload(entry: FEntry) {
    const r = files.remote;
    if (!r || entry.isDir) return;
    files.busy = true;
    try {
      const local = await rpc<string>("sftp_download", {
        remoteId: r.id, ...remoteParams(r),
        path: joinPath(files.path, entry.name), name: entry.name,
      });
      toast(`Téléchargé : ${local}`, "success");
    } catch (e) {
      toast(String(e), "error");
    }
    files.busy = false;
  }
  function fileToB64(f: File): Promise<string> {
    return new Promise((res, rej) => {
      const fr = new FileReader();
      fr.onload = () => res((fr.result as string).split(",")[1] ?? "");
      fr.onerror = () => rej(fr.error);
      fr.readAsDataURL(f);
    });
  }
  async function filesDrop(e: DragEvent) {
    e.preventDefault();
    files.over = false;
    const r = files.remote;
    const dropped = e.dataTransfer?.files;
    if (!r || !dropped?.length) return;
    files.busy = true;
    for (const f of dropped) {
      try {
        const b64 = await fileToB64(f);
        await rpc("sftp_upload", { remoteId: r.id, ...remoteParams(r), path: joinPath(files.path, f.name), dataB64: b64 });
        toast(`${f.name} envoyé sur ${r.name}`, "success");
      } catch (err) {
        toast(`${f.name} : ${err}`, "error");
      }
    }
    filesLoad(files.path);
  }

  // ─── redirections de ports + aperçu navigateur ───────────────────────────
  type Forward = { id: string; remoteId: string; remoteName: string; localPort: number; remoteHost: string; remotePort: number };
  let forwards = $state<Forward[]>([]);
  let forwardsOpen = $state(false);
  let preview = $state<{ url: string; forwardId: string } | null>(null);
  let previewFrame = $state<HTMLIFrameElement | null>(null);
  let newFwd = $state({ remoteHost: "localhost", remotePort: "", localPort: "" });

  async function addForward() {
    const r = activeSshRemote();
    if (!r || !newFwd.remotePort) return;
    const id = crypto.randomUUID();
    try {
      const actual =
        (await rpc<number>("port_forward_start", {
          id,
          localPort: Number(newFwd.localPort) || 0,
          remoteHost: newFwd.remoteHost || "localhost",
          remotePort: Number(newFwd.remotePort),
          ...remoteParams(r),
        })) ?? (Number(newFwd.localPort) || Number(newFwd.remotePort)); // démo
      forwards = [...forwards, { id, remoteId: r.id, remoteName: r.name, localPort: actual, remoteHost: newFwd.remoteHost || "localhost", remotePort: Number(newFwd.remotePort) }];
      newFwd = { remoteHost: "localhost", remotePort: "", localPort: "" };
      toast(`Tunnel localhost:${actual} → ${r.name}`, "success");
    } catch (e) {
      toast(String(e), "error");
    }
  }
  async function stopForward(f: Forward) {
    await rpc("port_forward_stop", { id: f.id }).catch(() => {});
    forwards = forwards.filter((x) => x.id !== f.id);
    if (preview?.forwardId === f.id) preview = null;
  }
  function openPreview(f: Forward) {
    preview = { url: `http://localhost:${f.localPort}`, forwardId: f.id };
  }
  function reloadPreview() {
    if (previewFrame) previewFrame.src = previewFrame.src; // recharge (cross-origin safe)
  }
  function openInBrowser(url: string) {
    if (inTauri) openUrl(url).catch((e) => toast(String(e), "error"));
    else window.open(url, "_blank");
  }

  // ─── recherche dans le buffer ─────────────────────────────────────────────
  let searchState = $state<{ sid: string; query: string } | null>(null);
  let searchInput = $state<HTMLInputElement | null>(null);

  function openSearch(sid: string) {
    searchState = { sid, query: searchState?.sid === sid ? searchState.query : "" };
    requestAnimationFrame(() => searchInput?.focus());
  }
  function closeSearch() {
    const sid = searchState?.sid;
    searchState = null;
    if (sid) {
      const s = sessions.get(sid);
      s?.search.clearDecorations();
      s?.term.focus();
    }
  }
  function searchKeydown(e: KeyboardEvent) {
    const s = searchState && sessions.get(searchState.sid);
    if (!s || !searchState) return;
    if (e.key === "Escape") closeSearch();
    else if (e.key === "Enter" && e.shiftKey) s.search.findPrevious(searchState.query);
    else if (e.key === "Enter") s.search.findNext(searchState.query);
  }

  async function pasteInto(sid: string, e: MouseEvent) {
    e.preventDefault();
    if (!inTauri) return;
    const t = await readText().catch(() => "");
    if (t) {
      sessions.get(sid)?.term.paste(t);
      sessions.get(sid)?.term.focus();
    }
  }

  // ─── menu natif + raccourcis ─────────────────────────────────────────────
  listen<string>("menu", (ev) => {
    const sid = activeTab?.active;
    switch (ev.payload) {
      case "close-pane":
        if (sid) closePane(sid);
        else if (activeTab) closeTab(activeTab);
        break;
      case "new-connection": openPicker(); break;
      case "split-h":
      case "split-v":
        if (sid && activeTab) openPicker({ tabId: activeTab.id, sid, dir: ev.payload === "split-h" ? "h" : "v" });
        break;
      case "clear": if (sid) sessions.get(sid)?.term.clear(); break;
      case "toggle-sidebar": settings.sidebar = !settings.sidebar; save(); break;
      case "settings": modal = { type: "settings" }; break;
      case "sync-config": if (sid) syncNow(sid); break;
    }
  });

  function globalKeydown(e: KeyboardEvent) {
    if (!e.metaKey) {
      if (e.key === "Escape" && modal) modal = null;
      return;
    }
    if (e.key >= "1" && e.key <= "9") {
      const idx = Number(e.key) - 1;
      if (tabs[idx]) {
        activeTabId = tabs[idx].id;
        e.preventDefault();
      }
    } else if (e.key === "f" && activeTab?.active) {
      e.preventDefault();
      openSearch(activeTab.active);
    } else if (e.key === "p") {
      e.preventDefault();
      modal = modal?.type === "palette" ? null : { type: "palette", filter: "" };
    } else if (!inTauri) {
      // en mode démo (pas de menu natif), on émule les accélérateurs
      if (e.key === "b") { e.preventDefault(); settings.sidebar = !settings.sidebar; }
      if (e.key === "n") { e.preventDefault(); openPicker(); }
    }
  }

  // ─── sidebar : terminaux + projets ────────────────────────────────────────
  let expanded = $state<Record<string, boolean>>({});
  function isExpanded(pid: string): boolean {
    return expanded[pid] ?? true;
  }
  function openTabFor(pid: string): Tab | undefined {
    return tabs.find((t) => t.projectId === pid);
  }
  function focusPane(tab: Tab, sid: string) {
    activeTabId = tab.id;
    tab.active = sid;
    sessions.get(sid)?.term.focus();
  }

  // zoom : le panneau actif occupe tout l'onglet (⌘⇧Entrée)
  let zoomedSid = $state<string | null>(null);
  function toggleZoom(sid: string) {
    zoomedSid = zoomedSid === sid ? null : sid;
    requestAnimationFrame(() => sessions.get(sid)?.fit.fit());
  }

  // palette ⌘P : panneaux ouverts, projets, remotes
  type PaletteItem = { icon: "remote" | "project" | "pane" | "local"; label: string; sub: string; run: () => void };
  function paletteItems(): PaletteItem[] {
    const items: PaletteItem[] = [];
    for (const t of tabs) {
      for (const sid of leaves(t.root)) {
        const s = sessions.get(sid);
        if (!s) continue;
        items.push({
          icon: s.remote.id === "local" ? "local" : "pane",
          label: s.remote.name + (s.cmd ? ` — ${s.cmd}` : ""),
          sub: t.projectId ? projects.find((p) => p.id === t.projectId)?.name ?? "projet" : "terminal ouvert",
          run: () => focusPane(t, sid),
        });
      }
    }
    for (const p of projects)
      items.push({ icon: "project", label: p.name, sub: "projet", run: () => { const o = openTabFor(p.id); o ? (activeTabId = o.id) : openProject(p); } });
    items.push({ icon: "local", label: LOCAL.name, sub: "nouveau terminal", run: () => openRemote(LOCAL) });
    for (const r of remotes)
      items.push({ icon: "remote", label: r.name, sub: `${r.user}@${r.host}`, run: () => openRemote(r) });
    return items;
  }
  function paneAttention(sid: string): boolean {
    return attentions.some((a) => attentionTarget(a) === sid);
  }

  // ─── montage xterm ────────────────────────────────────────────────────────
  function b64ToBytes(b64: string): Uint8Array {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  }

  function attach(node: HTMLElement, sid: string) {
    const s = sessions.get(sid);
    if (!s) return;
    if (s.term.element) node.appendChild(s.term.element);
    else {
      s.term.open(node);
      if (!s.webgl) {
        try {
          const webgl = new WebglAddon();
          webgl.onContextLoss(() => webgl.dispose());
          s.term.loadAddon(webgl);
          s.webgl = true;
        } catch {
          /* renderer DOM en secours */
        }
      }
    }
    s.fit.fit();
    s.term.focus();
  }
  function mountTerm(node: HTMLElement, sid: string) {
    attach(node, sid);
    let current = sid;
    const ro = new ResizeObserver(() => sessions.get(current)?.fit.fit());
    ro.observe(node);
    return {
      update(newSid: string) {
        current = newSid;
        attach(node, newSid);
      },
      destroy: () => ro.disconnect(),
    };
  }

  function autofocus(node: HTMLElement) {
    requestAnimationFrame(() => node.focus());
  }

  onDestroy(() => {
    for (const cc of [...ccSessions.values()]) rpc("ssh_disconnect", { sessionId: cc.ctrlSid }).catch(() => {});
    for (const sid of [...sessions.keys()]) {
      rpc("ssh_disconnect", { sessionId: sid }).catch(() => {});
      removeSession(sid);
    }
    forwards.forEach((f) => rpc("port_forward_stop", { id: f.id }).catch(() => {}));
  });

</script>

<!-- empêche le webview de « naviguer » vers un fichier déposé hors zone -->
<svelte:window onkeydown={globalKeydown} onresize={onWindowResize} ondragover={(e) => e.preventDefault()} ondrop={(e) => e.preventDefault()} />

<!-- ─── icônes ─────────────────────────────────────────────────────────── -->
{#snippet iTerminal()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 5.5 6 8l-3 2.5M8 11h5"/></svg>{/snippet}
{#snippet iGrid()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="1"/><rect x="9" y="9" width="4.5" height="4.5" rx="1"/></svg>{/snippet}
{#snippet iKey()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="5" cy="8" r="2.5"/><path d="M7.5 8h6M11 8v2.5M13.5 8v1.5"/></svg>{/snippet}
{#snippet iGear()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="2"/><path d="M8 2.5v2M8 11.5v2M2.5 8h2M11.5 8h2M4.1 4.1l1.4 1.4M10.5 10.5l1.4 1.4M11.9 4.1l-1.4 1.4M5.5 10.5l-1.4 1.4"/></svg>{/snippet}
{#snippet iPlus()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 3v10M3 8h10"/></svg>{/snippet}
{#snippet iClose()}<svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>{/snippet}
{#snippet iPencil()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5 2.5l2 2L5 13H3v-2z"/></svg>{/snippet}
{#snippet iTrash()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M3 4.5h10M6.5 4V3h3v1M4.5 4.5l.5 8.5h6l.5-8.5M6.5 7v4M9.5 7v4"/></svg>{/snippet}
{#snippet iSidebar()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="12" height="10" rx="2"/><path d="M6 3v10"/></svg>{/snippet}
{#snippet iBookmark()}<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M4 2.5h8V14l-4-2.8L4 14z"/></svg>{/snippet}
{#snippet iBolt()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M9 2 4 9h3.5L7 14l5-7H8.5z"/></svg>{/snippet}
{#snippet iSplitH()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="12" height="10" rx="2"/><path d="M8 3v10"/></svg>{/snippet}
{#snippet iSplitV()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="12" height="10" rx="2"/><path d="M2 8h12"/></svg>{/snippet}
{#snippet iZoom()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2H2v4M10 2h4v4M6 14H2v-4M10 14h4v-4"/></svg>{/snippet}
{#snippet iZoomOut()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 6h4V2M14 6h-4V2M2 10h4v4M14 10h-4v4"/></svg>{/snippet}
{#snippet iSpinner(size = 16)}<svg class="spin" width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M8 1.5A6.5 6.5 0 1 1 1.5 8"/></svg>{/snippet}
{#snippet iFolder()}<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1.5 2h4.5A1.5 1.5 0 0 1 14 6.5v5A1.5 1.5 0 0 1 12.5 13h-9A1.5 1.5 0 0 1 2 11.5z"/></svg>{/snippet}
{#snippet iFile()}<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><path d="M4 2h5l3 3v9H4zM9 2v3h3"/></svg>{/snippet}
{#snippet iDownload()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2.5V10M5 7.5 8 10.5l3-3M3 13h10"/></svg>{/snippet}
{#snippet iRefresh()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5V5h-2.5"/></svg>{/snippet}
{#snippet iLaptop()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3.5" width="10" height="7" rx="1.5"/><path d="M1.5 12.5h13"/></svg>{/snippet}
{#snippet iChevronR()}<svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3.5 10.5 8 6 12.5"/></svg>{/snippet}
{#snippet iGlobe()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6"/><path d="M2 8h12M8 2c1.7 1.6 2.7 3.8 2.7 6S9.7 12.4 8 14M8 2C6.3 3.6 5.3 5.8 5.3 8S6.3 12.4 8 14"/></svg>{/snippet}
{#snippet iExternal()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 3h4v4M13 3 7.5 8.5M11 9.5V13H3V5h3.5"/></svg>{/snippet}
{#snippet iWarn()}<svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2 1.5 13.5h13zM8 6.5V10M8 11.8v.2"/></svg>{/snippet}
{#snippet iAlert()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="8" cy="8" r="6"/><path d="M8 5v3.5M8 10.8v.2"/></svg>{/snippet}
{#snippet iCheck()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8.5 6.5 11.5 12.5 4.5"/></svg>{/snippet}

{#snippet sbSection(title: string, onAdd: (() => void) | null, addDisabled = false)}
  <div class="sb-head">
    <span>{title}</span>
    {#if onAdd}<button class="icon-btn sb-add" onclick={onAdd} disabled={addDisabled} title="Ajouter">{@render iPlus()}</button>{/if}
  </div>
{/snippet}

{#snippet rowActions(id: string, onEdit: () => void, onDelete: () => void)}
  <span class="row-actions">
    {#if confirmDeleteId === id}
      <button class="confirm-del" onclick={(e) => { e.stopPropagation(); onDelete(); }}>Supprimer ?</button>
    {:else}
      <button class="icon-btn" title="Modifier" onclick={(e) => { e.stopPropagation(); onEdit(); }}>{@render iPencil()}</button>
      <button class="icon-btn" title="Supprimer" onclick={(e) => { e.stopPropagation(); confirmDeleteId = id; }}>{@render iTrash()}</button>
    {/if}
  </span>
{/snippet}

{#snippet remoteRow(r: Remote, onPick: (r: Remote) => void, withActions: boolean)}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="row" onclick={() => onPick(r)} title={r.id === "local" ? "Shell local" : `${r.user}@${r.host}:${r.port}`}>
    <span class="row-icon">{#if r.id === "local"}{@render iLaptop()}{:else}{@render iTerminal()}{/if}</span>
    <span class="row-label">{r.name}</span>
    {#if r.claude}<span class="row-tag">claude</span>{/if}
    {#if remoteAttention(r.id)}<span class="dot attention"></span>{/if}
    {#if withActions}{@render rowActions(r.id, () => editRemote(r, true), () => deleteRemote(r))}{/if}
  </div>
{/snippet}

{#snippet sessRow(tab: Tab, sid: string, sub: boolean)}
  {@const s = sessions.get(sid)}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div
    class="row"
    class:sub
    class:current={tab.id === activeTabId && tab.active === sid}
    class:dragging={dragSid === sid}
    class:drop-before={dropRow?.sid === sid && !dropRow.after}
    class:drop-after={dropRow?.sid === sid && dropRow.after}
    draggable="true"
    ondragstart={(e) => {
      dragSid = sid;
      e.dataTransfer?.setData("text/plain", sid);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
    }}
    ondragend={() => { dragSid = null; dropTarget = null; dropRow = null; }}
    {...reorderzone(sid, sub)}
    onclick={() => focusPane(tab, sid)}>
    <span class="row-icon">{#if s?.remote.id === "local"}{@render iLaptop()}{:else}{@render iTerminal()}{/if}</span>
    <span class="row-label">{s?.remote.name}{s?.cmd ? ` — ${s.cmd}` : ""}</span>
    {#if sessStatus[sid]?.status === "connecting"}<span class="row-spin">{@render iSpinner(11)}</span>{/if}
    {#if paneAttention(sid)}<span class="dot attention"></span>{/if}
    <span class="row-actions">
      <button class="icon-btn" title="Fermer" onclick={(e) => { e.stopPropagation(); closePane(sid); }}>{@render iClose()}</button>
    </span>
  </div>
{/snippet}

{#snippet paneTree(tab: Tab, node: PaneNode)}
  {#if "leaf" in node}
    {@const s = sessions.get(node.leaf)}
    {@const st = sessStatus[node.leaf]}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="pane" class:active={tab.active === node.leaf} class:zoomed={zoomedSid === node.leaf} onclick={() => (tab.active = node.leaf)}>
      <div class="pane-bar">
        <span class="pane-title">{s?.remote.name}{s?.cmd ? ` — ${s.cmd}` : ""}</span>
        <span class="pane-btns">
          {#if s && s.remote.id !== "local" && !s.cc}
            <button class="icon-btn" title="Injecter la config Claude" onclick={() => syncNow(node.leaf)}>{@render iBolt()}</button>
          {/if}
          <button class="icon-btn" title="Plein écran (⇧⌘Entrée)" onclick={() => toggleZoom(node.leaf)}>{#if zoomedSid === node.leaf}{@render iZoomOut()}{:else}{@render iZoom()}{/if}</button>
          {#if s?.cc}
            <button class="icon-btn" title="Diviser à droite (tmux)" onclick={() => ccSplit(node.leaf, "h")}>{@render iSplitH()}</button>
            <button class="icon-btn" title="Diviser en dessous (tmux)" onclick={() => ccSplit(node.leaf, "v")}>{@render iSplitV()}</button>
            <button class="icon-btn" title="Fermer le panneau (tmux)" onclick={() => ccKill(node.leaf)}>{@render iClose()}</button>
          {:else}
            <button class="icon-btn" title="Diviser à droite (⌘D)" onclick={() => openPicker({ tabId: tab.id, sid: node.leaf, dir: "h" })}>{@render iSplitH()}</button>
            <button class="icon-btn" title="Diviser en dessous (⇧⌘D)" onclick={() => openPicker({ tabId: tab.id, sid: node.leaf, dir: "v" })}>{@render iSplitV()}</button>
            <button class="icon-btn" title="Fermer (⌘W)" onclick={() => closePane(node.leaf)}>{@render iClose()}</button>
          {/if}
        </span>
      </div>
      <div class="pane-term" use:mountTerm={node.leaf} oncontextmenu={(e) => pasteInto(node.leaf, e)}>
        {#if searchState?.sid === node.leaf}
          <div class="search-bar">
            <input
              bind:this={searchInput}
              bind:value={searchState.query}
              placeholder="Rechercher…"
              onkeydown={searchKeydown}
              oninput={() => searchState && sessions.get(searchState.sid)?.search.findNext(searchState.query, { incremental: true })}
            />
            <button class="icon-btn" onclick={closeSearch}>{@render iClose()}</button>
          </div>
        {/if}
        {#if st && st.status !== "open"}
          <div class="pane-veil">
            {#if st.status === "connecting"}
              <span class="veil-spin">{@render iSpinner(18)}</span>
              <span>Connexion à {s?.remote.name}…</span>
            {:else if st.status === "error"}
              <span class="veil-warn">{@render iWarn()}</span>
              <span class="veil-msg">{st.error}</span>
              <div class="veil-actions">
                <button class="btn" onclick={() => connectSession(node.leaf)}>Réessayer</button>
                <button class="btn ghost" onclick={() => removeSession(node.leaf)}>Fermer</button>
              </div>
            {:else}
              {#if s && s.remote.id !== "local"}
                <span class="veil-spin">{@render iSpinner(18)}</span>
                <span class="veil-msg">Connexion perdue — reconnexion automatique…{s.tmux ? " (la session tmux continue sur le serveur)" : ""}</span>
              {:else}
                <span class="veil-msg">Session terminée</span>
              {/if}
              <div class="veil-actions">
                <button class="btn" onclick={() => connectSession(node.leaf)}>Reconnecter</button>
                <button class="btn ghost" onclick={() => removeSession(node.leaf)}>Fermer</button>
              </div>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  {:else}
    <div class="split {node.dir}">
      <div class="split-child" style="flex:{node.ratio}">{@render paneTree(tab, node.a)}</div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="divider {node.dir}" onpointerdown={(e) => dragDivider(e, node)}></div>
      <div class="split-child" style="flex:{1 - node.ratio}">{@render paneTree(tab, node.b)}</div>
    </div>
  {/if}
{/snippet}

{#snippet field(label: string)}
  <span class="f-label">{label}</span>
{/snippet}

{#snippet colorField(label: string, key: keyof ITheme)}
  <label class="color-field">
    <input type="color" value={(settings.customTheme[key] as string) ?? "#000000"} oninput={(e) => setCustom(key, e.currentTarget.value)} />
    <span>{label}</span>
  </label>
{/snippet}

{#snippet meter(label: string, ratio: number, detail: string)}
  <div class="meter" title="{label.toUpperCase()} — {detail}">
    <span class="meter-label">{label}</span>
    <span class="meter-track">
      <span
        class="meter-fill"
        class:warn={ratio > 0.7}
        class:crit={ratio > 0.9}
        style="width:{Math.round(Math.min(1, Math.max(0, ratio)) * 100)}%"></span>
    </span>
    <span class="meter-pct">{Math.round(Math.min(1, ratio) * 100)}%</span>
  </div>
{/snippet}

<main class:no-sidebar={!settings.sidebar}>
  {#if settings.sidebar}
    <aside class="sidebar">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-traffic" data-tauri-drag-region>
        <button class="icon-btn sb-toggle" title="Masquer la barre latérale (⌘B)" onclick={() => { settings.sidebar = false; save(); }}>{@render iSidebar()}</button>
      </div>
      <nav class="sb-scroll">
        {#if loaded}
          {#if agents.length}
            <div class="sb-section agents-section">
              <div class="sb-head"><span>Agents</span>{#if waitingCount}<span class="agent-badge">{waitingCount}</span>{/if}</div>
              {#each agents as a (a.sid)}
                <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                <div class="row agent" class:current={a.tab.id === activeTabId && a.tab.active === a.sid} onclick={() => focusPane(a.tab, a.sid)} title={a.message}>
                  <span class="row-icon agent-{a.status}">
                    {#if a.status === "waiting"}{@render iAlert()}{:else if a.status === "done"}{@render iCheck()}{:else}<span class="agent-dot"></span>{/if}
                  </span>
                  <span class="row-label">{a.name}</span>
                  {#if a.status === "waiting" && a.notif}
                    <span class="agent-acts">
                      <button class="att-btn yes" title="Autoriser (envoie 1)" onclick={(e) => { e.stopPropagation(); answerAttention(a.notif!, "1"); }}>✓</button>
                      <button class="att-btn no" title="Refuser (Échap)" onclick={(e) => { e.stopPropagation(); answerAttention(a.notif!, "\x1b"); }}>✗</button>
                    </span>
                  {:else if a.status === "done"}
                    <span class="agent-meta">fini</span>
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="sb-section" class:drop={dropTarget === "standalone"} {...dropzone("standalone")}>
            {@render sbSection("Terminaux", () => openPicker())}
            {#each tabs.filter((t) => !t.projectId && t.root) as t (t.id)}
              {#each leaves(t.root) as sid (sid)}
                {@render sessRow(t, sid, false)}
              {/each}
            {:else}
              <p class="sb-empty">{dragSid ? "Déposer ici pour sortir du projet" : "Aucun terminal — ⌘N"}</p>
            {/each}
          </div>

          <div class="sb-section">
            {@render sbSection("Projets", null)}
            {#each projects as p (p.id)}
              {@const open = openTabFor(p.id)}
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
              <div
                class="row"
                class:drop={dropTarget === p.id}
                {...dropzone(p.id)}
                onclick={() => (open ? (activeTabId = open.id) : openProject(p))}>
                <button
                  class="icon-btn chev"
                  class:open={isExpanded(p.id)}
                  onclick={(e) => { e.stopPropagation(); expanded[p.id] = !isExpanded(p.id); }}>{@render iChevronR()}</button>
                <span class="row-label strong">{p.name}</span>
                {#if open}<span class="dot live" title="Projet ouvert"></span>{/if}
                <button
                  class="icon-btn row-plus"
                  title="Ajouter un terminal au projet"
                  onclick={(e) => { e.stopPropagation(); openPicker({ projectId: p.id }); }}>{@render iPlus()}</button>
                {@render rowActions(p.id, () => (modal = { type: "project", data: $state.snapshot(p) }), () => deleteProject(p))}
              </div>
              {#if isExpanded(p.id)}
                {#if open}
                  {#each leaves(open.root) as sid (sid)}
                    {@render sessRow(open, sid, true)}
                  {/each}
                {:else}
                  {#each projLeaves(p.root) as leaf, n (n)}
                    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                    <div class="row sub dim" onclick={() => openProject(p)}>
                      <span class="row-icon">{#if leaf.remoteId === "local"}{@render iLaptop()}{:else}{@render iTerminal()}{/if}</span>
                      <span class="row-label">{projRemote(leaf.remoteId)?.name ?? "?"}{leaf.cmd ? ` — ${leaf.cmd}` : ""}</span>
                    </div>
                  {/each}
                {/if}
              {/if}
            {:else}
              <p class="sb-empty">Enregistre un layout via l'icône signet ↗</p>
            {/each}
          </div>
        {/if}
      </nav>
      <button class="sb-settings" onclick={() => (modal = { type: "settings" })}>
        {@render iGear()}<span>Réglages</span>
      </button>
    </aside>
  {/if}

  <section class="content">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="titlebar" data-tauri-drag-region>
      {#if !settings.sidebar}
        <div class="traffic-pad"></div>
        <button class="icon-btn" title="Afficher la barre latérale (⌘B)" onclick={() => { settings.sidebar = true; save(); }}>{@render iSidebar()}</button>
      {/if}
      <span class="tb-title">{activeTab && activeTab.root ? tabTitle(activeTab) : "arabel"}</span>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="tb-space" data-tauri-drag-region></div>
      {#if activeMetrics}
        {@render meter("cpu", Math.min(1, activeMetrics.load / activeMetrics.cpus), `${activeMetrics.load.toFixed(1)} / ${activeMetrics.cpus}`)}
        {@render meter("ram", activeMetrics.memUsed / (activeMetrics.memTotal || 1), `${gb(activeMetrics.memUsed)} / ${gb(activeMetrics.memTotal)} Go`)}
        {@render meter("dsk", activeMetrics.diskUsed / (activeMetrics.diskTotal || 1), `${gb(activeMetrics.diskUsed)} / ${gb(activeMetrics.diskTotal)} Go`)}
      {/if}
      {#if activeSshRemote()}
        <button class="icon-btn" class:active-btn={forwardsOpen} title="Redirections de ports" onclick={() => (forwardsOpen = !forwardsOpen)}>{@render iGlobe()}</button>
        <button class="icon-btn" class:active-btn={files.open} title="Fichiers du serveur (SFTP)" onclick={toggleFiles}>{@render iFolder()}</button>
      {/if}
      {#if forwards.length}
        <span class="fwd-count" title="Tunnels actifs">{forwards.length}</span>
      {/if}
      {#if activeTab?.root}
        <button
          class="icon-btn"
          title="Enregistrer comme projet"
          onclick={() => {
            const proj = activeTab.projectId && projects.find((p) => p.id === activeTab.projectId);
            modal = { type: "saveProject", tabId: activeTab.id, name: proj ? proj.name : "" };
          }}>{@render iBookmark()}</button>
      {/if}
    </header>

    <div class="body">
    {#each tabs as t (t.id)}
      <div class="tab-content" class:hidden={t.id !== activeTabId} data-tab={t.id}>
        {#if t.root}
          {@render paneTree(t, t.root)}
        {:else}
          <div class="welcome">
            <div class="wordmark">arabel</div>
            <p class="hint">Terminal local &amp; SSH pour piloter tes agents IA</p>
            {#if loaded}
              <div class="welcome-list">
                {@render remoteRow(LOCAL, (x) => openInTab(t, x), false)}
                {#each remotes.slice(0, 4) as r (r.id)}
                  {@render remoteRow(r, (x) => openInTab(t, x), false)}
                {/each}
              </div>
              {#if !remotes.length}
                <button class="btn" onclick={() => (identities.length ? editRemote() : editIdentity())}>
                  {identities.length ? "Ajouter un remote SSH" : "Ajouter une identité SSH"}
                </button>
              {/if}
            {/if}
          </div>
        {/if}
      </div>
    {/each}

    {#if files.open}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <aside
        class="files"
        class:over={files.over}
        ondragover={(e) => { e.preventDefault(); files.over = true; }}
        ondragleave={() => (files.over = false)}
        ondrop={filesDrop}>
        <div class="files-head">
          <span class="files-title">{files.remote?.name}</span>
          <span class="files-btns">
            <button class="icon-btn" title="Actualiser" onclick={() => filesLoad(files.path)}>{@render iRefresh()}</button>
            <button class="icon-btn" title="Fermer" onclick={() => (files.open = false)}>{@render iClose()}</button>
          </span>
        </div>
        <div class="files-path" title={files.path}>{files.path}</div>
        <div class="files-list">
          {#if files.path !== "/"}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row" onclick={() => filesLoad(parentPath(files.path))}>
              <span class="row-icon">{@render iFolder()}</span>
              <span class="row-label">..</span>
            </div>
          {/if}
          {#each files.entries as entry (entry.name)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row" onclick={() => entry.isDir && filesLoad(joinPath(files.path, entry.name))}>
              <span class="row-icon" class:file-icon={!entry.isDir}>{#if entry.isDir}{@render iFolder()}{:else}{@render iFile()}{/if}</span>
              <span class="row-label">{entry.name}</span>
              {#if !entry.isDir}
                <span class="row-meta">{humanSize(entry.size)}</span>
                <span class="row-actions">
                  <button class="icon-btn" title="Télécharger dans ~/Downloads" onclick={(e) => { e.stopPropagation(); fileDownload(entry); }}>{@render iDownload()}</button>
                </span>
              {/if}
            </div>
          {:else}
            <p class="sb-empty">Dossier vide</p>
          {/each}
        </div>
        <div class="files-hint">Dépose des fichiers ici pour les envoyer</div>
        {#if files.busy}
          <div class="files-veil"><span class="veil-spin">{@render iSpinner(16)}</span></div>
        {/if}
      </aside>
    {/if}

    {#if forwardsOpen}
      <aside class="files fwd-panel">
        <div class="files-head">
          <span class="files-title">Redirections de ports</span>
          <button class="icon-btn" title="Fermer" onclick={() => (forwardsOpen = false)}>{@render iClose()}</button>
        </div>
        <form class="fwd-add" onsubmit={(e) => { e.preventDefault(); addForward(); }}>
          <input placeholder="port distant (ex. 3000)" bind:value={newFwd.remotePort} />
          <button type="submit" class="btn fwd-go" disabled={!newFwd.remotePort} title="Ouvrir le tunnel">{@render iPlus()}</button>
        </form>
        <div class="files-list">
          {#each forwards as f (f.id)}
            <div class="fwd-row">
              <span class="fwd-label" title="localhost:{f.localPort} → {f.remoteName}:{f.remotePort}">
                <b>:{f.localPort}</b> <span class="fwd-arrow">→</span> {f.remoteName}:{f.remotePort}
              </span>
              <span class="fwd-btns">
                <button class="icon-btn" title="Aperçu intégré" onclick={() => openPreview(f)}>{@render iGlobe()}</button>
                <button class="icon-btn" title="Ouvrir dans le navigateur" onclick={() => openInBrowser(`http://localhost:${f.localPort}`)}>{@render iExternal()}</button>
                <button class="icon-btn" title="Arrêter le tunnel" onclick={() => stopForward(f)}>{@render iClose()}</button>
              </span>
            </div>
          {:else}
            <p class="sb-empty">Aucun tunnel — indique un port distant ↑</p>
          {/each}
        </div>
        <div class="files-hint">Le port distant devient accessible sur localhost</div>
      </aside>
    {/if}

    {#if preview}
      <aside class="preview">
        <div class="preview-bar">
          <button class="icon-btn" title="Recharger" onclick={reloadPreview}>{@render iRefresh()}</button>
          <span class="preview-url">{preview.url}</span>
          <button class="icon-btn" title="Ouvrir dans le navigateur" onclick={() => openInBrowser(preview!.url)}>{@render iExternal()}</button>
          <button class="icon-btn" title="Fermer l'aperçu" onclick={() => (preview = null)}>{@render iClose()}</button>
        </div>
        <iframe class="preview-frame" title="Aperçu" src={preview.url} bind:this={previewFrame}></iframe>
      </aside>
    {/if}
    </div>
  </section>
</main>

<!-- ─── toasts ─────────────────────────────────────────────────────────── -->
{#if toasts.length}
  <div class="toasts">
    {#each toasts as t (t.id)}
      <div class="toast {t.kind}">{t.msg}</div>
    {/each}
  </div>
{/if}

<!-- ─── attentions ─────────────────────────────────────────────────────── -->
{#if attentions.length}
  <div class="attentions">
    {#each attentions.slice(-4) as a (a.id)}
      <div class="attention" class:stop={a.kind === "stop"}>
        <button class="att-msg" onclick={() => { gotoAttention(a); dismissAttention(a); }} title="Aller au panneau">
          {a.message}
        </button>
        {#if a.kind === "notif"}
          <button class="att-btn yes" title="Autoriser (envoie 1)" onclick={() => answerAttention(a, "1")}>✓</button>
          <button class="att-btn no" title="Refuser (envoie Échap)" onclick={() => answerAttention(a, "\x1b")}>✗</button>
        {/if}
        <button class="icon-btn" onclick={() => dismissAttention(a)}>{@render iClose()}</button>
      </div>
    {/each}
    {#if attentions.length > 4}<div class="att-more">+{attentions.length - 4} autres</div>{/if}
  </div>
{/if}

<!-- ─── modales ────────────────────────────────────────────────────────── -->
{#if modal}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="overlay" onclick={() => (modal = null)}>
    <div class="sheet" onclick={(e) => e.stopPropagation()}>
      {#if modal.type === "remote"}
        <h2>{remotes.some((r) => r.id === (modal as any).data.id) ? "Modifier le remote" : "Nouveau remote"}</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveRemote(); }}>
          <label>{@render field("Nom")}<input bind:value={modal.data.name} placeholder="mon-vps (auto si vide)" use:autofocus /></label>
          <div class="f-pair">
            <label class="grow">{@render field("Hôte")}<input bind:value={modal.data.host} placeholder="vps.exemple.com" required /></label>
            <label class="f-port">{@render field("Port")}<input type="number" bind:value={modal.data.port} /></label>
          </div>
          <label>{@render field("Utilisateur")}<input bind:value={modal.data.user} required /></label>
          <label>{@render field("Authentification")}
            <select bind:value={modal.data.auth}>
              <option value="key">Clé privée</option>
              <option value="password">Mot de passe</option>
              <option value="agent">ssh-agent</option>
            </select>
          </label>
          {#if (modal.data.auth ?? "key") === "key"}
            <label>{@render field("Identité")}
              <select bind:value={modal.data.identityId} required>
                {#each identities as i (i.id)}<option value={i.id}>{i.name}</option>{/each}
              </select>
            </label>
            {#if !identities.length}<p class="f-hint">Aucune identité — ajoutes-en une, ou choisis ssh-agent / mot de passe.</p>{/if}
          {:else if modal.data.auth === "password"}
            <label>{@render field("Mot de passe")}
              <input type="password" bind:value={modal.password} placeholder={remotes.some((x) => x.id === (modal as any).data.id) ? "(inchangé)" : ""} />
            </label>
            <p class="f-hint">Stocké dans le Keychain macOS, jamais sur disque.</p>
          {:else}
            <p class="f-hint">Utilise les clés chargées dans ton ssh-agent (<code>ssh-add</code>).</p>
          {/if}
          <label class="f-check">
            <input type="checkbox" bind:checked={modal.data.claude} />
            <span>Claude Code — synchroniser la config et lancer <code>claude</code> à la connexion</span>
          </label>
          <label class="f-check">
            <input type="checkbox" checked={modal.data.tmux !== false} onchange={(e) => { if (modal?.type === "remote") modal.data.tmux = e.currentTarget.checked; }} />
            <span>tmux — sessions persistantes (survivent aux déconnexions)</span>
          </label>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Annuler</button>
            <button type="submit" class="btn">Enregistrer</button>
          </div>
        </form>
      {:else if modal.type === "identity"}
        <h2>{identities.some((i) => i.id === (modal as any).data.id) ? "Modifier l'identité" : "Nouvelle identité"}</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveIdentity(); }}>
          <label>{@render field("Nom")}<input bind:value={modal.data.name} placeholder="(auto si vide)" use:autofocus /></label>
          <label>{@render field("Clé privée")}<input bind:value={modal.data.keyPath} required /></label>
          <label>{@render field("Passphrase")}
            <input type="password" bind:value={modal.passphrase} placeholder={modal.data.hasPassphrase ? "(inchangée)" : "(aucune)"} />
          </label>
          <p class="f-hint">Stockée dans le Keychain macOS, jamais sur disque.</p>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Annuler</button>
            <button type="submit" class="btn">Enregistrer</button>
          </div>
        </form>
      {:else if modal.type === "project"}
        <h2>Modifier le projet</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveProjectEdit(); }}>
          <label>{@render field("Nom")}<input bind:value={modal.data.name} required use:autofocus /></label>
          {#each projLeaves(modal.data.root) as leaf, n}
            <label>
              {@render field(`Panneau ${n + 1} · ${remotes.find((r) => r.id === leaf.remoteId)?.name ?? "?"} — commande initiale`)}
              <input bind:value={leaf.cmd} placeholder="(aucune)" />
            </label>
          {/each}
          <p class="f-hint">La commande est envoyée au shell à l'ouverture du panneau.</p>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Annuler</button>
            <button type="submit" class="btn">Enregistrer</button>
          </div>
        </form>
      {:else if modal.type === "saveProject"}
        <h2>Enregistrer le projet</h2>
        <form onsubmit={(e) => { e.preventDefault(); confirmSaveProject(); }}>
          <label>{@render field("Nom du projet")}<input bind:value={modal.name} required use:autofocus /></label>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Annuler</button>
            <button type="submit" class="btn">Enregistrer</button>
          </div>
        </form>
      {:else if modal.type === "picker"}
        <h2>{modal.dir ? "Ouvrir dans le nouveau panneau" : "Nouvelle connexion"}</h2>
        {#if remotes.length > 6}
          <input class="split-filter" bind:value={modal.filter} placeholder="Filtrer…" use:autofocus />
        {/if}
        <div class="split-list">
          {@render remoteRow(LOCAL, doPick, false)}
          {#each remotes.filter((r) => r.name.toLowerCase().includes((modal as any).filter.toLowerCase())) as r (r.id)}
            {@render remoteRow(r, doPick, false)}
          {/each}
        </div>
        {#if !modal.dir && !modal.projectId}
          <label class="f-check cc-check">
            <input type="checkbox" bind:checked={modal.cc} />
            <span>Mode <b>tmux natif</b> — panneaux miroir des splits tmux (expérimental)</span>
          </label>
        {/if}
        <div class="sheet-actions spread">
          <button class="btn ghost" onclick={() => (modal = { type: "connections" })}>Gérer les connexions…</button>
          <button class="btn ghost" onclick={() => (modal = null)}>Annuler</button>
        </div>
      {:else if modal.type === "connections"}
        <h2>Connexions</h2>
        <div class="mgr-head">
          <span>Remotes SSH</span>
          <span class="mgr-btns">
            <button class="icon-btn" title="Importer depuis ~/.ssh/config" onclick={openSshImport}>{@render iDownload()}</button>
            <button class="icon-btn" title="Nouveau remote" onclick={() => editRemote(undefined, true)}>{@render iPlus()}</button>
          </span>
        </div>
        <div class="split-list">
          {#each remotes as r (r.id)}
            {@render remoteRow(r, (x) => editRemote(x, true), true)}
          {:else}
            <p class="sb-empty">{identities.length ? "Aucun remote" : "Crée d'abord une identité ↓"}</p>
          {/each}
        </div>
        <div class="mgr-head">
          <span>Identités SSH</span>
          <button class="icon-btn" title="Nouvelle identité" onclick={() => editIdentity(undefined, true)}>{@render iPlus()}</button>
        </div>
        <div class="split-list">
          {#each identities as i (i.id)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row" onclick={() => editIdentity(i, true)} title={i.keyPath}>
              <span class="row-icon">{@render iKey()}</span>
              <span class="row-label">{i.name}</span>
              {#if i.hasPassphrase}<span class="row-meta">🔒</span>{/if}
              {@render rowActions(i.id, () => editIdentity(i, true), () => deleteIdentity(i))}
            </div>
          {:else}
            <p class="sb-empty">Aucune identité</p>
          {/each}
        </div>
        <div class="sheet-actions">
          <button class="btn" onclick={() => (modal = null)}>Fermer</button>
        </div>
      {:else if modal.type === "sshImport"}
        <h2>Importer depuis ~/.ssh/config</h2>
        <div class="split-list">
          {#each modal.hosts as h (h.host)}
            <div class="row import-row">
              <span class="row-icon">{@render iTerminal()}</span>
              <span class="row-label">{h.host}<span class="row-meta"> · {h.user || "?"}@{h.hostName}:{h.port}</span></span>
              <button class="btn ghost import-btn" onclick={() => importHost(h)}>Importer</button>
            </div>
          {:else}
            <p class="sb-empty">Tout est importé ✓</p>
          {/each}
        </div>
        <div class="sheet-actions">
          <button class="btn" onclick={() => (modal = { type: "connections" })}>Fermer</button>
        </div>
      {:else if modal.type === "palette"}
        {@const q = modal.filter.toLowerCase()}
        {@const items = paletteItems().filter((it) => (it.label + " " + it.sub).toLowerCase().includes(q))}
        <input
          class="palette-input"
          bind:value={modal.filter}
          placeholder="Aller à… (terminal, projet, remote)"
          use:autofocus
          onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); items[0]?.run(); modal = null; } }} />
        <div class="split-list palette-list">
          {#each items.slice(0, 40) as it, i (i)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row" onclick={() => { it.run(); modal = null; }}>
              <span class="row-icon">
                {#if it.icon === "local"}{@render iLaptop()}{:else if it.icon === "project"}{@render iBookmark()}{:else}{@render iTerminal()}{/if}
              </span>
              <span class="row-label">{it.label}</span>
              <span class="row-meta">{it.sub}</span>
            </div>
          {:else}
            <p class="sb-empty">Aucun résultat</p>
          {/each}
        </div>
      {:else if modal.type === "settings"}
        <h2>Réglages</h2>
        <form onsubmit={(e) => { e.preventDefault(); modal = null; }}>
          <div class="f-pair">
            <label class="grow">{@render field("Police du terminal")}
              <input
                bind:value={settings.fontFamily}
                onchange={applySettings}
                onfocus={() => (fontOpen = true)}
                oninput={() => (fontOpen = true)}
                onblur={() => setTimeout(() => (fontOpen = false), 160)}
                placeholder="Nom de la police" />
            </label>
            <label class="f-port">{@render field("Taille")}<input type="number" min="9" max="32" bind:value={settings.fontSize} onchange={applySettings} /></label>
          </div>
          {#if fontOpen}
            {@const q = settings.fontFamily.includes(",") ? "" : settings.fontFamily.toLowerCase().trim()}
            {@const matches = fontList.filter((f) => f.toLowerCase().includes(q)).slice(0, 60)}
            <div class="font-list">
              {#each matches as f (f)}
                <button type="button" class="font-opt" style="font-family:'{f.replace(/'/g, '')}', monospace" onmousedown={(e) => { e.preventDefault(); settings.fontFamily = f; fontOpen = false; applySettings(); }}>{f}</button>
              {:else}
                <div class="font-empty">{fontList.length ? "Aucune correspondance" : "Chargement…"}</div>
              {/each}
            </div>
          {/if}
          <label>{@render field("Thème du terminal")}
            <select bind:value={settings.theme} onchange={applySettings}>
              {#each Object.keys(THEMES) as th}<option value={th}>{th}</option>{/each}
              <option value="Personnalisé">Personnalisé</option>
            </select>
          </label>
          {#if settings.theme === "Personnalisé"}
            <div class="theme-editor">
              <div class="color-row">
                {@render colorField("Fond", "background")}
                {@render colorField("Texte", "foreground")}
                {@render colorField("Curseur", "cursor")}
                {@render colorField("Sélection", "selectionBackground")}
              </div>
              <div class="ansi-line">
                <span class="f-label">Palette ANSI</span>
                <div class="ansi-grid">
                  {#each ANSI_KEYS as k (k)}
                    <input type="color" class="ansi-dot" title={k} value={(settings.customTheme[k] as string) ?? '#000000'} oninput={(e) => setCustom(k, e.currentTarget.value)} />
                  {/each}
                </div>
              </div>
              <label class="seed">
                <span class="f-label">Partir d'un preset</span>
                <select onchange={(e) => { if (e.currentTarget.value) { settings.customTheme = { ...THEMES[e.currentTarget.value] }; applySettings(); } e.currentTarget.selectedIndex = 0; }}>
                  <option value="">Choisir…</option>
                  {#each Object.keys(THEMES) as th}<option value={th}>{th}</option>{/each}
                </select>
              </label>
            </div>
          {/if}
          <label class="f-check">
            <input type="checkbox" bind:checked={settings.copyOnSelect} onchange={applySettings} />
            <span>Copier automatiquement la sélection</span>
          </label>
          <div class="sheet-actions">
            <button type="submit" class="btn">Fermer</button>
          </div>
        </form>
      {/if}
    </div>
  </div>
{/if}

<style>
  /* ─── design tokens ─────────────────────────────────────────────────── */
  /* Charte Apple HIG dark mode — valeurs NSColor darkAqua exactes */
  :global(:root) {
    --bg-app: #1e1e1e;                          /* controlBackgroundColor */
    --surface: #282828;                          /* underPageBackgroundColor */
    --surface-raised: #323232;                   /* windowBackgroundColor (sheets) */
    --surface-hover: rgba(255, 255, 255, 0.065);
    --surface-active: rgba(255, 255, 255, 0.1);  /* sélection inactive */
    --border: rgba(255, 255, 255, 0.098);        /* separatorColor */
    --border-strong: rgba(255, 255, 255, 0.14);
    --text-primary: rgba(255, 255, 255, 0.847);  /* labelColor */
    --text-secondary: rgba(255, 255, 255, 0.549);/* secondaryLabelColor */
    --text-tertiary: rgba(255, 255, 255, 0.247); /* tertiaryLabelColor */
    --accent: #007aff;                           /* controlAccentColor */
    --accent-hover: #1a88ff;
    --selected: #0059d1;                         /* selectedContentBackgroundColor */
    --attention: #ff9f0a;                        /* systemOrange */
    --success: #32d74b;                          /* systemGreen */
    --danger: #ff453a;                           /* systemRed */
    --radius-sm: 5px;
    --radius-md: 6px;
    --radius-lg: 10px;                           /* sheets/fenêtres Big Sur+ */
    --focus-ring: 0 0 0 3.5px rgba(26, 169, 255, 0.5); /* keyboardFocusIndicatorColor */
    --ease: cubic-bezier(0.2, 0, 0, 1);
  }
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: var(--bg-app);
    color: var(--text-primary);
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif;
    font-size: 13px;
    -webkit-font-smoothing: antialiased;
    overflow: hidden;
    user-select: none;
  }
  :global(::selection) {
    background: #3f638b; /* selectedTextBackgroundColor */
  }
  :global(*:focus-visible) {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  :global(::-webkit-scrollbar) { width: 8px; height: 8px; }
  :global(::-webkit-scrollbar-thumb) { background: rgba(255,255,255,0.12); border-radius: 4px; }
  :global(::-webkit-scrollbar-track) { background: transparent; }

  main {
    display: grid;
    grid-template-columns: 224px 1fr;
    height: 100vh;
  }
  main.no-sidebar { grid-template-columns: 1fr; }

  /* ─── sidebar ────────────────────────────────────────────────────────── */
  .sidebar {
    display: flex;
    flex-direction: column;
    background: var(--bg-app); /* même fond que le contenu, look unifié */
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  .sb-traffic {
    height: 52px;
    flex: none;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding: 0 10px;
  }
  .sb-toggle { opacity: 0; transition: opacity 120ms; }
  .sidebar:hover .sb-toggle { opacity: 1; }
  .sb-scroll { flex: 1; overflow-y: auto; min-height: 0; padding-bottom: 8px; }
  .sb-section { margin-top: 10px; }
  .sb-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 3px 16px;
    font-size: 11px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.35); /* en-tête source list, sans capitales (style Finder/Music) */
  }
  .sb-add { opacity: 0; }
  .sb-section:hover .sb-add { opacity: 1; }
  .sb-empty {
    margin: 2px 16px;
    font-size: 12px;
    font-style: italic;
    color: var(--text-tertiary);
  }
  .sb-settings {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px;
    padding: 7px 10px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: 13px;
    font-family: inherit;
    transition: background 100ms;
  }
  .sb-settings:hover { background: var(--surface-hover); color: var(--text-primary); }

  /* rows */
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px; /* source list medium */
    margin: 0 10px;
    padding: 0 7px;
    border-radius: var(--radius-sm);
    color: var(--text-primary);
    transition: background 100ms;
    cursor: default;
    min-width: 0;
  }
  .row:hover { background: var(--surface-hover); }
  .row-icon { display: flex; color: var(--accent); flex: none; } /* icônes teintées accent, comme Finder/Music */
  .row-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; }
  .row-meta { font-size: 11px; color: var(--text-tertiary); flex: none; }
  .row-tag {
    flex: none;
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--accent);
    border: 1px solid rgba(94, 124, 226, 0.35);
    border-radius: 4px;
    padding: 1px 4px;
  }
  .row-actions { display: none; align-items: center; gap: 2px; flex: none; }
  .row:hover .row-actions { display: flex; }
  .row:hover .row-tag, .row:hover .row-meta { display: none; }
  .confirm-del {
    background: rgba(229, 83, 75, 0.15);
    color: var(--danger);
    border: none;
    border-radius: 4px;
    font-size: 11px;
    padding: 2px 6px;
    font-family: inherit;
  }
  .dot { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .dot.attention { background: var(--attention); animation: pop 150ms var(--ease); }
  .dot.live { background: var(--success); }
  @keyframes pop { from { transform: scale(0); } to { transform: scale(1); } }

  .row.sub { margin-left: 26px; height: 26px; }
  .row.dragging { opacity: 0.4; }
  .row.drop { box-shadow: inset 0 0 0 1.5px var(--accent); background: rgba(0, 122, 255, 0.12); }
  .row.drop-before { box-shadow: inset 0 2px 0 var(--accent); }
  .row.drop-after { box-shadow: inset 0 -2px 0 var(--accent); }
  .sb-section.drop { box-shadow: inset 0 0 0 1.5px var(--accent); border-radius: var(--radius-md); }
  .row .row-plus { display: none; flex: none; }
  .row:hover .row-plus { display: inline-flex; }
  .row.dim .row-label { color: var(--text-tertiary); }
  .row.dim .row-icon { color: var(--text-tertiary); }
  /* sélection source list : accent plein, texte et icônes blancs */
  .row.current { background: var(--accent); }
  .row.current .row-label, .row.current .row-icon, .row.current .row-spin { color: #fff; }
  .row.current .dot.attention { background: #fff; }
  .row.current .row-actions .icon-btn { color: rgba(255, 255, 255, 0.8); }
  .row-label.strong { font-weight: 500; }
  .row-spin { color: var(--text-tertiary); display: flex; flex: none; }
  .chev { width: 16px; height: 16px; flex: none; transition: transform 150ms var(--ease); }
  .chev.open { transform: rotate(90deg); }

  /* dashboard agents (compact, dans la barre latérale) */
  .agents-section { margin-top: 8px; }
  .agent-badge {
    min-width: 15px; height: 15px; padding: 0 4px;
    border-radius: 8px; background: var(--attention); color: #201400;
    font-size: 10px; font-weight: 700; display: flex; align-items: center; justify-content: center;
  }
  .row.agent .row-icon.agent-waiting { color: var(--attention); }
  .row.agent .row-icon.agent-done { color: var(--success); }
  .row.agent .row-icon.agent-working { color: var(--accent); }
  .agent-dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; animation: agent-pulse 1.4s ease-in-out infinite; }
  @keyframes agent-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
  .agent-acts { display: flex; gap: 2px; flex: none; }
  .agent-acts .att-btn { width: 20px; height: 20px; font-size: 11px; }
  .agent-meta { font-size: 10.5px; color: var(--success); flex: none; }
  .row.agent.current .agent-meta { color: #fff; }

  .mgr-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 14px 0 4px;
    font-size: 11px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.35);
  }
  .mgr-head:first-of-type { margin-top: 0; }
  .mgr-btns { display: flex; gap: 2px; }
  .import-row { margin: 0; }
  .import-btn { height: 22px; padding: 0 10px; font-size: 12px; flex: none; }
  .sheet .row { margin: 0; }
  .sheet-actions.spread { justify-content: space-between; }

  /* ─── titlebar + tabs ────────────────────────────────────────────────── */
  .content { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--bg-app); }
  .titlebar {
    height: 48px; /* toolbar unifiée, style Music */
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border);
  }
  .traffic-pad { width: 68px; flex: none; }
  .tb-title {
    font-size: 13px;
    font-weight: 600; /* headline */
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
    pointer-events: none;
  }
  .tb-space { flex: 1; height: 100%; }

  .meter {
    display: flex;
    align-items: center;
    gap: 5px;
    flex: none;
    padding: 0 6px;
  }
  .meter-label {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }
  .meter-track {
    width: 46px;
    height: 4px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.08);
    overflow: hidden;
  }
  .meter-fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 600ms var(--ease), background 300ms;
  }
  .meter-fill.warn { background: var(--attention); }
  .meter-fill.crit { background: var(--danger); }
  .meter-pct {
    font-size: 10.5px;
    font-variant-numeric: tabular-nums;
    color: var(--text-tertiary);
    width: 30px;
  }

  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    background: none;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--text-tertiary);
    padding: 0;
    line-height: 0;
    transition: background 100ms, color 100ms, opacity 120ms;
  }
  .icon-btn:hover:not(:disabled) { background: var(--surface-hover); color: var(--text-primary); }
  .icon-btn:disabled { opacity: 0.4; }

  /* ─── panes ──────────────────────────────────────────────────────────── */
  .body { flex: 1; min-height: 0; display: flex; }
  .tab-content { flex: 1; min-width: 0; min-height: 0; display: flex; position: relative; }
  .tab-content.hidden { display: none; }
  /* zoom : le panneau recouvre toute la zone terminal */
  .pane.zoomed { position: absolute; inset: 0; z-index: 6; }

  /* panneau fichiers SFTP */
  .files {
    width: 280px;
    flex: none;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: var(--bg-app);
    position: relative;
    min-height: 0;
  }
  .files.over { box-shadow: inset 0 0 0 2px var(--accent); }
  .files-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px 2px;
    font-size: 13px;
    font-weight: 600;
  }
  .files-btns { display: flex; gap: 2px; }
  .files-path {
    padding: 0 12px 6px;
    font-size: 11px;
    color: var(--text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl; /* ellipse à gauche pour les chemins longs */
    unicode-bidi: plaintext;
    text-align: left;
    border-bottom: 1px solid var(--border);
  }
  .files-list { flex: 1; overflow-y: auto; padding: 6px 0; min-height: 0; }
  .files .row { margin: 0 6px; }
  .file-icon { color: var(--text-tertiary) !important; }
  .files-hint {
    padding: 8px 12px;
    font-size: 11px;
    color: var(--text-tertiary);
    border-top: 1px solid var(--border);
    text-align: center;
  }
  .files-veil {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: color-mix(in srgb, var(--bg-app) 60%, transparent);
    z-index: 3;
  }
  .active-btn { color: var(--accent); }

  /* redirections de ports + aperçu */
  .fwd-count {
    flex: none;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--accent);
    color: #fff;
    font-size: 10.5px;
    font-weight: 600;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .fwd-panel { width: 300px; }
  .fwd-add { display: flex; gap: 6px; padding: 8px 10px; border-bottom: 1px solid var(--border); }
  .fwd-add input { flex: 1; }
  .fwd-go { width: 30px; padding: 0; flex: none; display: inline-flex; align-items: center; justify-content: center; }
  .fwd-row { display: flex; align-items: center; gap: 6px; margin: 0 6px; padding: 4px 6px; border-radius: var(--radius-sm); font-size: 12px; }
  .fwd-row:hover { background: var(--surface-hover); }
  .fwd-label { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .fwd-label b { color: var(--accent); font-weight: 600; }
  .fwd-arrow { color: var(--text-tertiary); }
  .fwd-btns { display: flex; gap: 1px; flex: none; }

  .preview {
    flex: none;
    width: 46%;
    min-width: 320px;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--border);
    background: #fff;
    min-height: 0;
  }
  .preview-bar {
    height: 34px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    background: var(--bg-app);
    border-bottom: 1px solid var(--border);
  }
  .preview-url {
    flex: 1;
    font-size: 11px;
    color: var(--text-tertiary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .preview-frame { flex: 1; width: 100%; border: none; background: #fff; }
  .split { display: flex; flex: 1; min-width: 0; min-height: 0; }
  .split.h { flex-direction: row; }
  .split.v { flex-direction: column; }
  .split-child { display: flex; min-width: 0; min-height: 0; }
  .divider { flex: none; position: relative; z-index: 2; }
  .divider.h { width: 5px; margin: 0 -2px; cursor: col-resize; }
  .divider.v { height: 5px; margin: -2px 0; cursor: row-resize; }
  .divider::after {
    content: "";
    position: absolute;
    background: var(--border);
    transition: background 120ms;
  }
  .divider.h::after { left: 2px; top: 0; bottom: 0; width: 1px; }
  .divider.v::after { top: 2px; left: 0; right: 0; height: 1px; }
  .divider:hover::after { background: var(--accent); }

  .pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg-app);
  }
  .pane.active { box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.1); }
  .pane-bar {
    height: 26px;
    flex: none;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-tertiary);
  }
  .pane.active .pane-bar { color: var(--text-secondary); }
  .pane-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pane-btns { display: flex; align-items: center; gap: 1px; opacity: 0; transition: opacity 120ms; }
  .pane:hover .pane-btns { opacity: 1; }
  .pane-btns .icon-btn { width: 20px; height: 20px; }
  .pane-term { flex: 1; min-height: 0; padding: 0 10px 8px 10px; position: relative; }

  .pane-veil {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    background: color-mix(in srgb, var(--bg-app) 82%, transparent);
    backdrop-filter: blur(2px);
    z-index: 4;
    font-size: 12px;
    color: var(--text-secondary);
    padding: 20px;
    text-align: center;
  }
  .veil-spin { color: var(--text-tertiary); display: flex; }
  .veil-warn { color: var(--danger); display: flex; }
  .veil-msg { max-width: 420px; user-select: text; }
  .veil-actions { display: flex; gap: 8px; margin-top: 4px; }
  :global(.spin) { animation: rot 800ms linear infinite; }
  @keyframes rot { to { transform: rotate(360deg); } }

  /* ─── welcome ────────────────────────────────────────────────────────── */
  .welcome {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
  }
  .wordmark { font-size: 26px; font-weight: 700; color: var(--text-secondary); letter-spacing: -0.3px; } /* Large Title */
  .welcome .hint { margin: 0 0 12px; font-size: 13px; color: var(--text-tertiary); }
  .welcome-list { width: 300px; display: flex; flex-direction: column; gap: 1px; }

  /* ─── boutons / formulaires ──────────────────────────────────────────── */
  /* push buttons macOS : accent plein / bezel gris */
  .btn {
    height: 26px;
    padding: 0 14px;
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-md);
    font-size: 13px;
    font-weight: 500;
    font-family: inherit;
    box-shadow: inset 0 0.5px 0 rgba(255, 255, 255, 0.25);
    transition: background 100ms;
  }
  .btn:hover { background: var(--accent-hover); }
  .btn:active { background: var(--selected); }
  .btn.ghost {
    background: rgba(255, 255, 255, 0.12); /* bouton bezel secondaire */
    color: var(--text-primary);
    box-shadow: inset 0 0.5px 0 rgba(255, 255, 255, 0.12);
  }
  .btn.ghost:hover { background: rgba(255, 255, 255, 0.17); }

  input, select {
    height: 26px;
    box-sizing: border-box;
    width: 100%;
    background: var(--bg-app); /* textBackgroundColor */
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: var(--radius-sm);
    padding: 0 8px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
    transition: box-shadow 120ms;
  }
  input::placeholder { color: var(--text-tertiary); }
  input:focus, select:focus { box-shadow: var(--focus-ring); outline: none; }
  input[type="checkbox"] { width: auto; height: auto; accent-color: var(--accent); }
  input[type="number"] { appearance: textfield; }

  /* ─── modales ────────────────────────────────────────────────────────── */
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(2px);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 30;
    animation: fade 150ms var(--ease);
  }
  @keyframes fade { from { opacity: 0; } }
  .sheet {
    width: 420px;
    max-height: 80vh;
    overflow-y: auto;
    background: var(--surface-raised); /* windowBackgroundColor */
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: var(--radius-lg);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.55), 0 0 0 0.5px rgba(0, 0, 0, 0.4);
    padding: 20px;
    animation: sheet-in 150ms var(--ease);
  }
  @keyframes sheet-in { from { opacity: 0; transform: scale(0.97); } }
  .sheet h2 { margin: 0 0 16px; font-size: 15px; font-weight: 600; } /* Title 3 emphasized */
  .sheet form { display: flex; flex-direction: column; gap: 12px; }
  .sheet label { display: flex; flex-direction: column; gap: 4px; }
  .f-label { font-size: 12px; color: var(--text-secondary); }
  .f-pair { display: flex; gap: 10px; }
  .f-pair .grow { flex: 1; }
  .f-port { width: 80px; flex: none; }
  .f-check { flex-direction: row !important; align-items: center; gap: 8px !important; font-size: 12.5px; color: var(--text-secondary); }
  .f-check code { font-size: 11px; background: var(--surface); border-radius: 3px; padding: 0 4px; }
  .f-hint { margin: -4px 0 0; font-size: 11.5px; color: var(--text-tertiary); }
  .sheet-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
  .split-list { display: flex; flex-direction: column; gap: 1px; margin: 0 -8px; }
  .split-filter { margin-bottom: 10px; }
  .palette-input { margin-bottom: 10px; }
  .palette-list { max-height: 52vh; overflow-y: auto; }
  .palette-list .row { cursor: pointer; }

  /* combobox fontes */
  .font-list {
    margin-top: -4px;
    max-height: 180px;
    overflow-y: auto;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg-app);
    display: flex;
    flex-direction: column;
  }
  .font-opt {
    text-align: left;
    background: none;
    border: none;
    color: var(--text-primary);
    padding: 6px 10px;
    font-size: 14px;
    font-family: inherit;
  }
  .font-opt:hover { background: var(--accent); color: #fff; }
  .font-empty { padding: 8px 10px; font-size: 12px; color: var(--text-tertiary); }

  /* éditeur de thème personnalisé */
  .theme-editor { display: flex; flex-direction: column; gap: 10px; padding: 10px; background: var(--surface); border-radius: var(--radius-sm); }
  .color-row { display: flex; gap: 12px; flex-wrap: wrap; }
  .color-field { display: flex; flex-direction: row !important; align-items: center; gap: 6px; font-size: 12px; color: var(--text-secondary); }
  .color-field input[type="color"] { width: 26px; height: 22px; padding: 0; border-radius: 5px; border: 1px solid var(--border); background: none; cursor: pointer; }
  .ansi-line { display: flex; flex-direction: column; gap: 5px; }
  .ansi-grid { display: grid; grid-template-columns: repeat(8, 1fr); gap: 5px; }
  .ansi-dot { width: 100%; height: 22px; padding: 0; border-radius: 5px; border: 1px solid var(--border); background: none; cursor: pointer; }
  .seed { flex-direction: row !important; align-items: center; justify-content: space-between; gap: 8px; }
  .seed select { width: 140px; }

  /* ─── toasts ─────────────────────────────────────────────────────────── */
  .toasts {
    position: fixed;
    bottom: 16px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    z-index: 40;
  }
  .toast {
    background: rgba(50, 50, 50, 0.85);
    backdrop-filter: blur(30px) saturate(180%);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: var(--radius-lg); /* notification macOS */
    padding: 9px 14px;
    font-size: 13px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    animation: toast-in 150ms var(--ease);
    max-width: 480px;
  }
  .toast.error { border-left: 2px solid var(--danger); }
  .toast.success { border-left: 2px solid var(--success); }
  @keyframes toast-in { from { opacity: 0; transform: translateY(8px); } }

  /* ─── attentions ─────────────────────────────────────────────────────── */
  .attentions {
    position: fixed;
    right: 12px;
    bottom: 12px;
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 380px;
  }
  .attention {
    display: flex;
    align-items: center;
    gap: 4px;
    background: rgba(50, 50, 50, 0.85);
    backdrop-filter: blur(30px) saturate(180%);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-left: 2px solid var(--attention);
    border-radius: var(--radius-lg);
    padding: 7px 8px;
    font-size: 12.5px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    animation: toast-in 150ms var(--ease);
  }
  .attention.stop { border-left-color: var(--success); }
  .att-msg {
    flex: 1;
    background: none;
    border: none;
    color: var(--text-primary);
    text-align: left;
    font-size: 12.5px;
    font-family: inherit;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 4px;
  }
  .att-btn {
    width: 22px;
    height: 22px;
    border: none;
    border-radius: var(--radius-sm);
    font-size: 12px;
    line-height: 0;
    font-family: inherit;
  }
  .att-btn.yes { background: rgba(63, 182, 104, 0.15); color: var(--success); }
  .att-btn.no { background: rgba(229, 83, 75, 0.15); color: var(--danger); }
  .att-more { font-size: 11px; color: var(--text-tertiary); text-align: center; }

  /* ─── recherche ──────────────────────────────────────────────────────── */
  .search-bar {
    position: absolute;
    top: 4px;
    right: 10px;
    z-index: 5;
    display: flex;
    gap: 4px;
    align-items: center;
    background: var(--surface-raised);
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    padding: 4px 6px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.35);
    animation: search-in 120ms var(--ease);
  }
  @keyframes search-in { from { opacity: 0; transform: translateY(-4px); } }
  .search-bar input { width: 170px; height: 24px; }

  /* terminal : sélection de texte autorisée */
  .pane-term :global(.xterm) { user-select: text; }
</style>
