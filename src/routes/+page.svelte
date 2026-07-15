<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
  import { readText, writeText, readImage } from "@tauri-apps/plugin-clipboard-manager";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { notify, playSound, setBadge, claudeTurn, prefs as notifPrefs, demo as notifyDemo } from "$lib/notify";
  import { Terminal, type ITheme } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { SearchAddon } from "@xterm/addon-search";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import { WebglAddon } from "@xterm/addon-webgl";
  import "@xterm/xterm/css/xterm.css";
  import { onDestroy, tick, type Snippet } from "svelte";
  import { fade, fly, scale } from "svelte/transition";
  import { flip } from "svelte/animate";
  import { TmuxControl, parseLayout, layToTree, layPanes, layPaneSizes, toHexKeys, demo as tmuxccDemo, type Lay } from "$lib/tmuxcc";
  import Tgs from "$lib/Tgs.svelte";
  import { version as appVersion } from "../../package.json";

  // ─── mode démo (aperçu navigateur sans Tauri) ─────────────────────────────
  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  if (!inTauri) try { tmuxccDemo(); } catch (e) { console.error("tmuxcc:", e); } // auto-test parseur en dev
  if (!inTauri) try { notifyDemo(); } catch (e) { console.error("notify:", e); } // auto-test statut de tour
  if (!inTauri) try { gitStatusDemo(); } catch (e) { console.error("git:", e); }

  // ─── plateforme : macOS vs Windows/Linux ──────────────────────────────────
  const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform || navigator.userAgent || "");
  // Modificateur applicatif : ⌘ sur macOS, Ctrl+Maj ailleurs. Ctrl seul reste au
  // terminal (^C, ^F, ^K…), donc on ne le capture jamais côté Windows/Linux.
  const appMod = (e: KeyboardEvent) => (isMac ? e.metaKey : e.ctrlKey && e.shiftKey);
  // Libellé UI du coffre où arabel range les secrets (fichier chiffré local).
  const secretStore = "Arabel's encrypted vault";
  // Sans les feux macOS (fenêtre décorée nativement sur Windows), on récupère
  // l'espace réservé en haut de la barre latérale / titlebar.
  if (typeof document !== "undefined") document.body.classList.toggle("win", !isMac);

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
    "\x1b[1m>\x1b[0m fix the pagination bug then run the tests\r\n\r\n" +
    "\x1b[90m✻ Thinking…\x1b[0m\r\n\r\n" +
    "\x1b[32m●\x1b[0m \x1b[1mRead\x1b[0m src/routes/users.ts\r\n" +
    "\x1b[32m●\x1b[0m \x1b[1mEdit\x1b[0m src/routes/users.ts \x1b[90m(+4 -2)\x1b[0m\r\n" +
    "\x1b[33m●\x1b[0m \x1b[1mBash\x1b[0m npm test\r\n" +
    "  \x1b[90m⎿ Waiting for permission…\x1b[0m\r\n";

  let demoFetched = false; // démo seulement : le fetch a-t-il déjà tourné (voir git_run)
  async function rpc<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    if (inTauri) return invoke<T>(cmd, args);
    // démo : réponses factices
    await new Promise((r) => setTimeout(r, cmd === "ssh_connect" ? 700 : 30));
    if (cmd === "store_load") return JSON.stringify(DEMO_STORE) as T;
    if (cmd === "claude_probe") return true as T; // démo : le VPS est déjà équipé
    if (cmd === "claude_sync") return "claude present · 5 config item(s) pushed · hooks installed" as T;
    if (cmd === "shell_enhance") return "suggestions enabled — open a new terminal." as T;
    if (cmd === "mosh_available") return true as T;
    if (cmd === "wsl_distros") return ["Ubuntu", "Debian"] as T;
    if (cmd === "sftp_paste_image") return "/home/deploy/.arabel/paste/demo.png" as T;
    if (cmd === "sftp_home") return "/home/deploy" as T;
    if (cmd === "sftp_list")
      return [
        { name: "apps", isDir: true, size: 0 },
        { name: ".claude", isDir: true, size: 0 },
        { name: "docker-compose.yml", isDir: false, size: 1893 },
        { name: "deploy.log", isDir: false, size: 482133 },
      ] as T;
    if (cmd === "sftp_download") return "/Users/luss/Downloads/fichier" as T;
    if (cmd === "sftp_upload") return (args?.path as string).split("/").pop() as T;
    if (cmd === "git_run") {
      // on saute les « -c core.x=y » de tête pour matcher la sous-commande
      const raw = (args?.args as string[]) ?? [];
      const j = raw.filter((x, i) => x !== "-c" && raw[i - 1] !== "-c").join(" ");
      // démo : le fetch révèle du retard (que le status seul ne peut pas voir), le pull le résorbe
      if (j.startsWith("fetch")) { demoFetched = true; return [0, ""] as T; }
      if (j.startsWith("pull")) { demoFetched = false; return [0, "Fast-forward\n 2 files changed, 18 insertions(+)"] as T; }
      if (j.startsWith("diff"))
        return [0, j.includes("--no-index")
          ? "diff --git a/notes.txt b/notes.txt\nnew file mode 100644\nindex 0000000..d5c8467\n--- /dev/null\n+++ b/notes.txt\n@@ -0,0 +1,2 @@\n+relancer le déploiement après la migration\n+vérifier les quotas côté API\n"
          : j.includes("réglages")
            ? "diff --cc src/config/réglages.ts\nindex ba2906d,e45c9c2..0000000\n--- a/src/config/réglages.ts\n+++ b/src/config/réglages.ts\n@@@ -8,1 -8,1 +8,5 @@@ export const config = {\n++<<<<<<< HEAD\n +  timeout: 30_000,\n++=======\n+   timeout: 5_000,\n++>>>>>>> origin/master\n"
            : "diff --git a/src/lib/api.ts b/src/lib/api.ts\nindex 22a7af1..f489e8f 100644\n--- a/src/lib/api.ts\n+++ b/src/lib/api.ts\n@@ -12,7 +12,7 @@ export async function fetchUser(id: string) {\n   const res = await fetch(`/api/users/${id}`);\n-  if (!res.ok) throw new Error(\"failed\");\n+  if (!res.ok) throw new Error(`user ${id}: ${res.status}`);\n   return res.json();\n }\n@@ -30,3 +30,6 @@ export function logout() {\n   session.clear();\n }\n+\n+/** Purge le cache local — appelé après un changement de rôle. */\n+export const purge = () => cache.clear();\n"] as T;
      if (j.startsWith("rev-parse")) return [0, "/home/deploy/synaptyx"] as T;
      if (j.startsWith("status"))
        return [0,
          // ab : le fetch de démo « découvre » 2 commits distants (voir plus bas)
          `# branch.oid abc123\n# branch.head master\n# branch.upstream origin/master\n# branch.ab +1 -${demoFetched ? 2 : 0}\n` +
          "1 .M N... 100644 100644 100644 aaa bbb frontend/package-lock.json\n" +
          "1 M. N... 100644 100644 100644 ccc ddd src/lib/api.ts\n" +
          "u UU N... 100644 100644 100644 100644 aaa bbb ccc src/config/réglages.ts\n" +
          "? notes.txt\n"] as T;
      if (j.startsWith("branch")) return [0, "master\ndevelop\nfeat/auth\nfeat/billing\nhotfix/1\nrelease/2\nstaging"] as T;
      if (j.startsWith("log")) return [0, "a1b2c3d (HEAD -> master) Fix billing rounding\ne4f5g6h Add auth guard\ni7j8k9l Release v0.2.0"] as T;
      return [0, ""] as T;
    }
    return undefined as T;
  }
  const listen: typeof tauriListen = inTauri
    ? tauriListen
    : (async () => () => {}) as unknown as typeof tauriListen;

  // ─── types ────────────────────────────────────────────────────────────────
  type Identity = { id: string; name: string; keyPath: string; hasPassphrase: boolean };
  type AuthKind = "key" | "password" | "agent";
  type Remote = { id: string; name: string; host: string; port: number; user: string; identityId: string; auth?: AuthKind; claude?: boolean; autoLaunch?: boolean; dir?: string; tmux?: boolean; mosh?: boolean; sysSsh?: boolean };
  type ImportHost = { host: string; hostName: string; user: string; port: number; identityFile: string };
  type PaneNode = { leaf: string } | { dir: "h" | "v"; ratio: number; a: PaneNode; b: PaneNode };
  type Tab = { id: string; root: PaneNode | null; active: string | null; projectId: string | null; cc?: string };
  // `cmd` (legacy, commande par pane) est migré vers Project.cmd au chargement.
  type ProjLeaf = { remoteId: string; cmd?: string; id?: string };
  type ProjNode = ProjLeaf | { dir: "h" | "v"; ratio?: number; a: ProjNode; b: ProjNode };
  // un projet = plusieurs vues (chaque vue = un onglet ; une vue peut être un split).
  // `root` (legacy, une seule vue) est migré à la lecture via projectViews().
  // `cmd` : commandes de démarrage du projet, rejouées sur CHAQUE terminal qu'on
  // y crée — les panes vont et viennent, la commande appartient au projet.
  type Project = { id: string; name: string; emoji?: string; cmd?: string; views?: ProjNode[]; root?: ProjNode };
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
    | { type: "configImport"; text: string }
    | { type: "diff"; path: string; mode: string; text: string; loading: boolean }
    | null;

  // pseudo-remotes pour les terminaux locaux : la machine + une par distro WSL
  // (id `wsl:<distro>`, Windows only). Même forme qu'un Remote → remoteRow, la
  // palette et les projets les traitent sans cas particulier.
  const LOCAL: Remote = { id: "local", name: isMac ? "This Mac" : "This PC", host: "", port: 0, user: "", identityId: "" };
  const isLocal = (id: string) => id === "local" || id.startsWith("wsl:");
  let wslDistros = $state<string[]>([]);
  const locals = $derived<Remote[]>([
    LOCAL,
    ...wslDistros.map((d) => ({ ...LOCAL, id: `wsl:${d}`, name: d })),
  ]);

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
    // OLED : fond #000 pur (pixels éteints). L'ANSI black reste relevé, sinon le texte noir disparaît.
    "Arabel OLED": {
      background: "#000000", foreground: "#d8d8dc", cursor: "#0a84ff", cursorAccent: "#000000",
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
  // ponytail: pack NewsEmoji = static/emoji/001..100.tgs, numérotation figée, pas d'index à maintenir.
  const EMOJI = Array.from({ length: 100 }, (_, i) => String(i + 1).padStart(3, "0"));
  let hoverEmoji = $state<string | null>(null);
  const ANSI_KEYS = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"] as const;

  // ─── raccourcis clavier configurables ─────────────────────────────────────
  // Combos normalisés « Meta+Shift+D », ordre canonique Meta→Ctrl→Alt→Shift+touche.
  // Défauts : ⌘ sur macOS, Ctrl+Maj ailleurs (Ctrl seul reste au terminal).
  const KMOD = isMac ? "Meta" : "Ctrl+Shift";
  const DEFAULT_KEYS: Record<string, string> = {
    palette: `${KMOD}+P`,
    search: `${KMOD}+F`,
    "new-connection": `${KMOD}+N`,
    "close-pane": `${KMOD}+W`,
    "split-h": `${KMOD}+D`,
    "split-v": isMac ? "Meta+Shift+D" : "Ctrl+Shift+S",
    clear: `${KMOD}+K`,
    "toggle-sidebar": `${KMOD}+B`,
    settings: `${KMOD}+Comma`,
    zoom: isMac ? "Meta+Shift+Enter" : "Ctrl+Shift+Enter",
    copy: `${KMOD}+C`,
    paste: `${KMOD}+V`,
    "next-tab": isMac ? "Meta+Shift+BracketRight" : "Ctrl+Alt+BracketRight",
    "prev-tab": isMac ? "Meta+Shift+BracketLeft" : "Ctrl+Alt+BracketLeft",
    "next-pane": isMac ? "Meta+Ctrl+ArrowDown" : "Ctrl+Alt+ArrowDown",
    "prev-pane": isMac ? "Meta+Ctrl+ArrowUp" : "Ctrl+Alt+ArrowUp",
  };

  let settings = $state({
    fontSize: 13,
    fontFamily: "SF Mono",
    theme: "Arabel Dark",
    copyOnSelect: true,
    sidebar: true,
    sounds: true, // sons d'événements agent (validation demandée / terminé / refus)
    notifications: true, // notifications système sur les mêmes événements
    tmuxStatus: false, // barre de statut tmux masquée par défaut (doublon de la sidebar)
    // « agent teams » de Claude Code : le lead peut lancer des coéquipiers, chacun
    // sa session. Expérimental ET nettement plus cher en tokens → opt-in explicite.
    agentTeams: false,
    // décoché = teammateMode "in-process" : les coéquipiers restent dans le pane du
    // lead au lieu d'ouvrir chacun son panneau tmux (miroir dans la grille en -CC).
    agentTeamPanes: true,
    projects: true, // décoché = terminaux seuls ; les projets restent en mémoire, juste masqués
    emojiAnim: true, // emojis de projet animés en continu ; décoché = figés sur la 1re frame
    cursorStyle: "bar" as "bar" | "block" | "underline",
    cursorBlink: true,
    scrollback: 10000,
    lineHeight: 1.25,
    keymap: { ...DEFAULT_KEYS } as Record<string, string>,
    customTheme: { ...THEMES["Arabel Dark"] } as ITheme,
  });
  function activeTheme(): ITheme {
    return settings.theme === "Custom" ? settings.customTheme : (THEMES[settings.theme] ?? THEMES["Arabel Dark"]);
  }
  function setCustom(key: keyof ITheme, val: string) {
    settings.customTheme = { ...settings.customTheme, [key]: val };
    applySettings();
  }

  // fontes système (chargées à l'ouverture des réglages)
  let fontList = $state<string[]>([]);
  let fontOpen = $state(false);
  let fontQuery = $state("");
  const DEFAULT_FONT = "SF Mono";
  async function loadFonts() {
    if (fontList.length) return;
    fontList = (await rpc<string[]>("list_fonts")) ?? ["Menlo", "Monaco", "SF Mono", "Courier New", "Fira Code", "JetBrains Mono", "Hack", "Cascadia Code"];
  }
  /** Pile CSS toujours valide : une famille seule reçoit un fallback monospace ; vide → défaut. */
  function fontStack(f: string): string {
    const t = (f || "").trim();
    if (!t) return `"${DEFAULT_FONT}", ui-monospace, Menlo, monospace`;
    if (t.includes(",")) return t; // déjà une pile
    return `"${t}", ui-monospace, Menlo, monospace`;
  }
  function pickFont(f: string) {
    settings.fontFamily = f.trim();
    fontOpen = false;
    fontQuery = "";
    applySettings();
  }
  async function importVscodeFont() {
    const t = await rpc<{ fontFamily?: string; fontSize?: number }>("vscode_terminal").catch(() => null);
    if (!t?.fontFamily && !t?.fontSize) return toast("Nothing to import from VS Code", "info");
    if (t.fontFamily) settings.fontFamily = t.fontFamily.split(",")[0].replace(/["']/g, "").trim();
    if (t.fontSize) settings.fontSize = Math.round(t.fontSize);
    applySettings();
    toast("Font imported from VS Code", "success");
  }
  let loaded = $state(false);
  let modal = $state<Modal>(null);
  let confirmDeleteId = $state<string | null>(null);
  let moshOk = $state(false); // binaire mosh présent → propose le transport mosh
  rpc<boolean>("mosh_available").then((v) => (moshOk = v)).catch(() => {});
  rpc<string[]>("wsl_distros").then((v) => (wslDistros = v ?? [])).catch(() => {});

  rpc<string>("store_load").then((json) => {
    let data: any;
    try { data = JSON.parse(json || "{}"); }
    catch { toast("Config illisible — sauvegarde suspendue pour ne pas l'écraser", "error", 12000); return; }
    identities = data.identities ?? [];
    remotes = data.remotes ?? [];
    projects = data.projects ?? [];
    // migration : la commande de démarrage était par pane, elle est désormais au
    // projet. ponytail: si plusieurs panes en avaient une différente, seule la
    // première survit — l'ancienne UI ne servait qu'à en poser une.
    for (const p of projects) {
      const pleaves = projectViews(p).flatMap(projLeaves);
      p.cmd ??= pleaves.map((l) => l.cmd).find(Boolean) ?? "";
      for (const l of pleaves) delete l.cmd;
    }
    settings = { ...settings, ...data.settings };
    savedTitles = data.titles ?? {}; // noms de terminaux (avant restore : sessLabel les relira par key)
    restoreWorkspace(data.workspace);
    loaded = true;
  });

  async function save() {
    if (!loaded) return; // pas encore chargé (ou config illisible) : ne jamais écraser
    // purge les noms de terminaux fermés : on ne garde que les keys encore
    // référencées — sessions ouvertes + vues de projet (fermées mais persistées).
    const keep = new Set<string>();
    for (const s of sessions.values()) keep.add(s.key);
    for (const p of projects) for (const v of projectViews(p)) for (const leaf of projLeaves(v)) if (leaf.id) keep.add(leaf.id);
    const pruned = Object.fromEntries(Object.entries(savedTitles).filter(([k]) => keep.has(k)));
    if (Object.keys(pruned).length !== Object.keys(savedTitles).length) savedTitles = pruned;
    await rpc("store_save", {
      data: JSON.stringify({
        identities, remotes, projects,
        settings: $state.snapshot(settings),
        workspace: snapshotWorkspace(),
        titles: $state.snapshot(savedTitles),
      }),
    });
  }

  // ─── partage de config entre postes (Mac ↔ Windows…) ─────────────────────
  // Copie/colle un instantané de la config via le presse-papiers. AUCUN secret
  // dedans : mots de passe & passphrases restent dans le coffre chiffré local
  // (à ressaisir sur l'autre poste, ou utiliser ssh-agent qui ne stocke rien côté arabel).
  function exportConfig() {
    // le keymap est spécifique au poste (Meta = ⌘ sur Mac, touche Windows ailleurs) :
    // le partager casserait tous les raccourcis sur l'autre plateforme.
    const { keymap, ...shared } = $state.snapshot(settings);
    const json = JSON.stringify({ v: 1, remotes, identities, projects, settings: shared }, null, 2);
    const done = () => toast("Config copied — paste it on your other PC to import", "success");
    if (inTauri) writeText(json).then(done).catch((e) => toast(String(e), "error"));
    else navigator.clipboard?.writeText(json).then(done).catch(() => toast("Copy failed", "error"));
  }
  function importConfig(json: string) {
    let data: any;
    try { data = JSON.parse(json); } catch { return toast("Invalid config — not valid JSON", "error"); }
    if (!data || typeof data !== "object") return toast("Invalid config", "error");
    // union par id, la version importée gagne ; les entrées propres au poste restent.
    const mergeById = <T extends { id: string }>(cur: T[], inc: unknown): T[] => {
      if (!Array.isArray(inc)) return cur;
      const m = new Map(cur.map((x) => [x.id, x]));
      for (const x of inc as T[]) if (x && x.id) m.set(x.id, x);
      return [...m.values()];
    };
    const n = Array.isArray(data.remotes) ? data.remotes.length : 0;
    remotes = mergeById(remotes, data.remotes);
    identities = mergeById(identities, data.identities);
    projects = mergeById(projects, data.projects);
    if (data.settings && typeof data.settings === "object") {
      const { keymap, ...s } = data.settings; // jamais le keymap d'un autre poste (compat vieux exports)
      settings = { ...settings, ...s };
    }
    save();
    applySettings();
    modal = null;
    toast(`Imported ${n} remote${n === 1 ? "" : "s"} + identities, projects & settings`, "success");
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
      const root = buildNode(t.root, projects.find((p) => p.id === t.projectId)?.cmd);
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
    return (sid && sessLabel(sid)) || "new tab";
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

  /** Rend le clavier au terminal visé APRÈS le flush Svelte : tant que son onglet
   *  est en display:none, term.focus() est ignoré — c'est ce qui obligeait à
   *  re-cliquer dans le terminal pour taper ou valider. */
  async function focusActive(sid: string | null) {
    if (!sid) return;
    await tick();
    if (modal || searchState || renamingSid) return; // une saisie a la priorité
    sessions.get(sid)?.term.focus();
  }

  // re-fit les terminaux quand on change d'onglet (ils étaient en display:none)
  $effect(() => {
    const t = tabs.find((x) => x.id === activeTabId);
    if (!t) return;
    focusActive(t.active); // couvre tous les chemins : sidebar, palette, ⌘1..9, cycle d'onglets
    if (t.cc) {
      // onglet tmux -CC : on réapplique le layout tmux courant (re-cale chaque
      // xterm sur la grille tmux + renvoie la taille client), pas de fit conteneur.
      const cc = ccSessions.get(t.cc);
      const lay = cc?.activeWindow ? cc.windows.get(cc.activeWindow) : null;
      if (cc && cc.activeWindow && lay) requestAnimationFrame(() => ccApplyLayout(cc, cc.activeWindow!, lay));
      return;
    }
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
    webglTries?: number; // tentatives de récupération du renderer GPU après perte de contexte
    cc?: { ctrlSid: string; paneId: string }; // panneau piloté en mode contrôle tmux
  };
  const sessions = new Map<string, Sess>();
  let sessStatus = $state<Record<string, SessStatus>>({});

  function termOptions() {
    return {
      fontFamily: fontStack(settings.fontFamily),
      fontSize: settings.fontSize,
      cursorBlink: settings.cursorBlink,
      cursorStyle: settings.cursorStyle,
      macOptionIsMeta: true,
      // dans une appli qui capte la souris (Claude Code, tmux, vim), la sélection
      // est happée par l'appli → ⌥+glisser force une sélection locale (donc le
      // copier-auto-sur-sélection remarche à l'intérieur de ces applis)
      macOptionClickForcesSelection: true,
      scrollback: settings.scrollback,
      lineHeight: settings.lineHeight,
      allowProposedApi: true, // parité Terax
      // PAS de smoothScrollDuration/scrollSensitivity : on garde les défauts xterm
      // = scroll NATIF du navigateur (inertie), le plus fluide (comme Terax). Les
      // définir fait intercepter/animer la molette par xterm → saccades.
      theme: activeTheme(),
    };
  }

  // ─── tmux : sessions persistantes côté serveur ────────────────────────────
  function shq(s: string): string {
    return `'${s.replace(/'/g, `'\\''`)}'`;
  }
  /** Commandes de démarrage d'un pane : une par ligne, lignes vides ignorées. */
  function cmdLines(cmd: string | undefined): string[] {
    return (cmd ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
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
    // options tmux scoppées à CETTE session (-t, pas -g, pour ne pas toucher les
    // autres sessions de l'utilisateur), appliquées à chaque (ré)attache :
    //  - status : barre de statut (masquée par défaut)
    //  - mouse on : molette = défilement de l'historique tmux (copy-mode)
    //  - molette rebindée à 2 lignes/cran (défaut tmux = 5) → fini le « 5 par 5 »
    //    saccadé. Le scroll tmux reste un redraw distant (pas aussi lisse que le
    //    scrollback natif d'xterm), mais nettement plus fin.
    const sopt =
      `tmux set -t ${n} status ${settings.tmuxStatus ? "on" : "off"} 2>/dev/null; ` +
      `tmux set -t ${n} mouse on 2>/dev/null; ` +
      // mouse on = tmux capte le glisser → la sélection est la SIENNE, xterm n'en a
      // aucune. set-clipboard fait émettre OSC 52 à tmux quand il copie ; le
      // handler OSC 52 côté xterm (setupTerm) le pousse dans le presse-papier macOS.
      // Options serveur (donc -s / -as) : appliquées aussi aux sessions réattachées.
      `tmux set -s set-clipboard on 2>/dev/null; ` +
      `tmux set -as terminal-features 'xterm*:clipboard' 2>/dev/null; ` +
      `tmux bind -T copy-mode WheelUpPane send -N2 -X scroll-up 2>/dev/null; ` +
      `tmux bind -T copy-mode WheelDownPane send -N2 -X scroll-down 2>/dev/null; ` +
      `tmux bind -T copy-mode-vi WheelUpPane send -N2 -X scroll-up 2>/dev/null; ` +
      `tmux bind -T copy-mode-vi WheelDownPane send -N2 -X scroll-down 2>/dev/null; `;
    return (
      `export PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"; ` + // exec SSH non-interactif = PATH minimal
      `if command -v tmux >/dev/null 2>&1; then ` +
      `if tmux has-session -t ${n} 2>/dev/null; then ${setenv}${sopt}exec tmux -u attach-session -t ${n}; ` +
      `else tmux -u new-session -d -s ${n} && ${tset}${send}${setenv}${sopt}exec tmux -u attach-session -t ${n}; fi; ` +
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
    // OSC 52 = « mets ça dans le presse-papier », émis par tmux (set-clipboard on),
    // vim, etc. xterm.js ne le gère pas nativement. C'est LA voie de copie quand une
    // appli capte la souris : sans ça, la sélection reste dans le buffer tmux.
    term.parser.registerOscHandler(52, (data) => {
      const b64 = data.slice(data.indexOf(";") + 1);
      if (b64 === "?" || !b64 || !inTauri) return true; // lecture (?) non supportée
      try {
        const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
        writeText(new TextDecoder().decode(bytes)).catch(() => {});
      } catch { /* base64 invalide */ }
      return true;
    });
    // Une frappe qu'on consomme doit AUSSI être annulée côté navigateur. xterm
    // ne le fait pas : son `_keyDown` fait `if (handler(e) === false) return`,
    // sans preventDefault. L'action native suivait donc son cours — et comme
    // xterm écoute `paste` sur son textarea ET sur son élément, ⌘V collait deux
    // fois : une par pasteClipboard(), une par le collage natif du webview.
    // `consume()` doit être utilisé partout où l'on renvoie false.
    const consume = (e: KeyboardEvent) => {
      e.preventDefault();
      return false;
    };
    // OSC 7770 = verbes arabel, émis par `~/.arabel/arabel` sur le serveur. La
    // séquence arrive par le PTY de CE panneau : c'est ça qui identifie
    // l'émetteur — rien à exporter, et ça marche aussi dans un panneau tmux -CC.
    term.parser.registerOscHandler(7770, (data) => {
      arabelVerb(sid, data);
      return true;
    });
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      // Shift+Entrée → nouvelle ligne sans envoyer (attendu par Claude Code) :
      // on émet ESC+CR, reconnu comme saut de ligne même à travers tmux
      if (e.key === "Enter" && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey) {
        send("\x1b\r");
        return consume(e);
      }
      // Ctrl+V « nu » (la touche de collage de Claude Code, y compris sur Mac) :
      // si une image locale est dispo on l'upload, sinon on restitue la frappe.
      if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "v") {
        pasteClipboard(sid, "\x16");
        return consume(e);
      }
      // raccourci configurable ? term = exécuté ici (accès sélection/pane) ;
      // window = laissé remonter à globalKeydown, mais rien n'est envoyé au pty.
      // preventDefault n'empêche pas la remontée : globalKeydown reçoit toujours.
      const id = actionCombos.get(comboOf(e));
      if (id) {
        const b = bindById.get(id)!;
        if (b.scope === "term") b.run(sid);
        return consume(e);
      }
      return true;
    });
    term.onData(send);
    // titre auto : le shell/programme peut émettre un titre (OSC 0/2), souvent le
    // cwd ou la commande — plus parlant que « vps-snpx » répété. Le renommage
    // manuel reste prioritaire (voir sessLabel).
    term.onTitleChange((t) => { const v = t.trim(); if (v) autoTitles[sid] = v; });
    return { term, fit, search };
  }

  function newSession(remote: Remote, cmd = "", key?: string): string {
    const sid = crypto.randomUUID();
    const { term, fit, search } = setupTerm(sid, (data) => {
      lastInput[sid] = Date.now();
      // « il repart » : SEULEMENT sur une vraie soumission (Entrée = \r), pas à
      // chaque frappe ni sur les séquences (focus \x1b[I, Maj+Entrée \x1b\r…),
      // sinon cliquer/écrire affichait « running » à tort.
      if (data.includes("\r") && !data.includes("\x1b")) lastSubmit[sid] = Date.now();
      onClaudeInput(sid);
      rpc("ssh_write", { sessionId: sid, data });
    });
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
    if (isLocal(s.remote.id)) {
      try {
        const distro = s.remote.id.startsWith("wsl:") ? s.remote.id.slice(4) : null;
        await rpc("local_connect", { sessionId: sid, cols: s.term.cols, rows: s.term.rows, distro });
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
          sessStatus[sid] = { status: "error", error: "This remote has no valid identity." };
          return;
        }
        keyPath = identity.keyPath;
        identityId = identity.id;
      }
      // Le drapeau `claude` est une préférence stockée dans le store LOCAL, sur un
      // Remote dont l'id est un UUID tiré par machine : un 2e poste voyait le même
      // VPS comme neuf et perdait tag, dashboard et hooks. On demande au VPS s'il
      // est déjà équipé (~/.arabel) — une fois par remote et par lancement, et
      // seulement tant qu'on ne le sait pas : c'est un cache dérivable, pas une
      // vérité déclarée. Best-effort : si le probe échoue, on garde l'existant.
      if (!s.remote.claude && !probedRemotes.has(s.remote.id)) {
        probedRemotes.add(s.remote.id);
        try {
          if (await rpc<boolean>("claude_probe", remoteParams(s.remote))) {
            s.remote.claude = true;
            remotes = [...remotes]; // réactivité : tag « claude » + dashboard Agents
            await save();
            s.term.write("\x1b[90m[arabel] Claude already set up here — tracking enabled\x1b[0m\r\n");
          }
        } catch { /* probe indisponible : on n'empêche pas la connexion */ }
      }
      const useTmux = s.remote.tmux !== false; // activé par défaut : survie aux déconnexions
      let execCmd: string | null = null;
      if (useTmux) {
        // la sync config doit précéder le premier lancement de claude (hooks)
        if (s.remote.claude && !syncedRemotes.has(s.remote.id)) {
          try {
            const msg = await rpc<string>("claude_sync", syncParams(s.remote));
            syncedRemotes.add(s.remote.id);
            s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
          } catch (e) {
            s.term.write(`\x1b[31m[arabel] sync failed: ${e}\x1b[0m\r\n`);
          }
        }
        const init: string[] = [];
        if (s.remote.dir) init.push(`cd ${shq(s.remote.dir)} 2>/dev/null`); // répertoire de travail
        if (s.remote.claude) init.push(`export ARABEL_PANE=${sid}`); // suivi : claude lancé plus tard sera tracké
        const lines = cmdLines(s.cmd);
        if (lines.length) init.push(...lines);
        else if (s.remote.claude && s.remote.autoLaunch) init.push("claude"); // lancement auto uniquement si demandé
        execCmd = tmuxCmd(`arabel-${s.key.slice(0, 8)}`, init.join("; "), s.remote.claude ? sid : null);
      }
      try {
        if (s.remote.sysSsh) {
          // transport « ssh système » : délègue à OpenSSH (compat parfaite des clés).
          // pas de metrics/hooks/SFTP (ils vivent sur le canal de contrôle russh).
          await rpc("ssh_pty_connect", {
            sessionId: sid,
            cols: s.term.cols,
            rows: s.term.rows,
            host: s.remote.host,
            port: Number(s.remote.port),
            user: s.remote.user,
            keyPath,
            auth: authKind,
            execCmd,
          });
        } else if (s.remote.mosh) {
          // transport mosh : UDP, écho prédictif, reprise après coupure (Unix).
          // metrics/hooks/SFTP restent sur russh (canaux indépendants).
          await rpc("mosh_connect", {
            sessionId: sid,
            cols: s.term.cols,
            rows: s.term.rows,
            host: s.remote.host,
            port: Number(s.remote.port),
            user: s.remote.user,
            keyPath,
            auth: authKind,
            execCmd,
          });
        } else {
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
        }
      } catch (e) {
        sessStatus[sid] = { status: "error", error: String(e) };
        return;
      }
      s.tmux = useTmux;
    }
    sessStatus[sid] = { status: "open", error: "" };
    s.unlisteners.push(
      await listen<string>(`ssh-output-${sid}`, (ev) => { lastOut[sid] = Date.now(); s.term.write(b64ToBytes(ev.payload)); }),
      await listen(`ssh-closed-${sid}`, () => {
        if (!sessStatus[sid]) return; // fermeture volontaire déjà nettoyée
        sessStatus[sid] = { status: "closed", error: "" };
        s.unlisteners.forEach((u) => u());
        s.unlisteners = [];
        if (!isLocal(s.remote.id)) scheduleReconnect(sid); // auto-reconnexion
      }),
    );
    // taille réelle du pane (la connexion est partie en 80x24 par défaut)
    s.fit.fit();
    rpc("ssh_resize", { sessionId: sid, cols: s.term.cols, rows: s.term.rows });
    if (!isLocal(s.remote.id)) {
      // métriques & hooks passent par des canaux russh indépendants → indisponibles
      // en transport « ssh système » (compromis assumé pour la compat des clés).
      if (!s.remote.sysSsh) {
        rpc("metrics_watch", { remoteId: s.remote.id, ...remoteParams(s.remote) }).catch(() => {});
        if (s.remote.claude) rpc("events_watch", { remoteId: s.remote.id, ...remoteParams(s.remote) }).catch(() => {});
      }
      if (!s.tmux) {
        // sans tmux : init envoyé dans le shell après connexion (comportement historique)
        if (s.remote.dir) await rpc("ssh_write", { sessionId: sid, data: `cd ${shq(s.remote.dir)} 2>/dev/null\n` });
        if (s.remote.claude && !s.remote.sysSsh) await rpc("ssh_write", { sessionId: sid, data: `export ARABEL_PANE=${sid}\n` });
        const lines = cmdLines(s.cmd);
        if (lines.length) await rpc("ssh_write", { sessionId: sid, data: lines.join("\n") + "\n" });
        else if (s.remote.claude && s.remote.autoLaunch && !s.remote.sysSsh) claudeSetup(sid, s.remote);
      }
    } else {
      if (!inTauri) {
        metrics[s.remote.id] = { load: 3.2, cpus: 10, memTotal: 34359738368, memUsed: 21474836480, diskTotal: 994662584320, diskUsed: 703687441776 };
      }
      const lines = cmdLines(s.cmd);
      if (lines.length) await rpc("ssh_write", { sessionId: sid, data: lines.join("\n") + "\n" });
    }
    if (!inTauri) {
      s.term.write(DEMO_OUTPUT);
      if (!isLocal(s.remote.id))
        metrics[s.remote.id] = { load: 1.4, cpus: 4, memTotal: 8321499136, memUsed: 5100273664, diskTotal: 84825923584, diskUsed: 39728447488 };
      // démo : activité live puis question à choix (états « travaille » / « attend »)
      if (s.remote.claude) {
        activity = { ...activity, [sid]: { label: "npm test", tool: "Bash", at: Date.now() } };
        paneStatus = { ...paneStatus, [sid]: "working" };
        setTimeout(() => {
          paneStatus = { ...paneStatus, [sid]: "waiting" };
          attentions = [...attentions, {
            id: crypto.randomUUID(), sid, remoteId: s.remote.id, kind: "notif" as const,
            message: "Waiting for permission to run: Bash 'npm test'",
            question: "Do you want to proceed?",
            options: ["Yes", "Yes, and don't ask again for npm", "No, and tell Claude what to do"],
          }].slice(-20);
        }, 1200);
      }
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
    { const { [sid]: _a, ...ra } = activity; activity = ra; const { [sid]: _p, ...rp } = paneStatus; paneStatus = rp; }
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
    if (isBrowser(sid)) return closeBrowser(sid); // panneau navigateur : pas de session SSH
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
    lastSize?: { c: number; r: number }; // dernière taille client envoyée (évite une boucle de layout)
  };
  const ccSessions = new Map<string, CcSession>();
  const ccSid = (ctrlSid: string, paneId: string) => `cc:${ctrlSid}:${paneId}`;
  const ccOf = (sid: string): CcSession | undefined => {
    const c = sessions.get(sid)?.cc;
    return c ? ccSessions.get(c.ctrlSid) : undefined;
  };
  const ccPaneId = (sid: string) => sessions.get(sid)?.cc?.paneId ?? "";
  /** UNIQUE voie d'envoi d'une commande à tmux -CC. tmux répond `%begin…%end` à
   *  CHAQUE commande : tout envoi doit donc réserver sa place dans `pending`,
   *  sinon la file se décale et les réponses partent au mauvais handler (le
   *  bootstrap `list-windows` était mangé par la réponse vide de refresh-client).
   *  Ne jamais écrire sur `ctrlSid` par `ssh_write` directement. */
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
      ccExec(cc, `send-keys -t %${paneId} -H ${toHexKeys(data)}`),
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
    // xterm AUTORITAIRE = tmux : chaque panneau adopte EXACTEMENT la grille que
    // tmux lui donne dans le layout (et non un fit au conteneur qui divergerait).
    // C'est ce qui aligne rendu et curseur — y compris en splits multi-panneaux.
    const sizes = layPaneSizes(lay);
    requestAnimationFrame(() => {
      for (const [paneId, { w, h }] of Object.entries(sizes)) {
        const t = sessions.get(ccSid(cc.ctrlSid, paneId))?.term;
        if (t && (t.cols !== w || t.rows !== h)) t.resize(w, h);
      }
      ccResize(cc); // le conteneur a pu changer → renvoie la taille client à tmux
    });
  }
  /** Taille de cellule RÉELLE de la police du terminal (px). Mesurée dans le DOM
   *  pour ne pas dépendre de constantes fausses : c'est ça qui désalignait tmux
   *  et le rendu xterm (le TUI se réécrivait par-dessus). */
  function cellPx(): { w: number; h: number } {
    const probe = document.createElement("span");
    probe.textContent = "0".repeat(80);
    probe.style.cssText = "position:absolute;top:-9999px;white-space:pre;visibility:hidden";
    probe.style.fontFamily = fontStack(settings.fontFamily);
    probe.style.fontSize = settings.fontSize + "px";
    document.body.appendChild(probe);
    const w = probe.getBoundingClientRect().width / 80;
    probe.remove();
    return { w: w || 8, h: Math.round(settings.fontSize * settings.lineHeight) || 16 };
  }
  /** Annonce à tmux la taille du CLIENT (en cellules) = l'espace pixel réellement
   *  dispo ÷ cellule réelle. tmux découpe ensuite les panneaux ; chaque xterm
   *  adopte la grille annoncée (voir ccApplyLayout). On NE fit PAS ici : xterm est
   *  piloté par tmux, pas par le conteneur. Le chrome d'un panneau (barre + marges)
   *  est retiré pour que les panneaux tiennent dans leur conteneur. */
  function ccResize(cc: CcSession) {
    const el = document.querySelector(`[data-tab="${cc.tabId}"]`) as HTMLElement | null;
    if (!el || !el.clientWidth) return;
    const { w, h } = cellPx();
    const cols = Math.max(20, Math.floor((el.clientWidth - 20) / w)); // -20 : marges G/D du pane-term
    const rows = Math.max(5, Math.floor((el.clientHeight - 34) / h)); // -34 : barre du panneau (26) + marge bas (8)
    if (cc.lastSize?.c === cols && cc.lastSize?.r === rows) return; // même taille → pas de refresh (évite une boucle de %layout-change)
    cc.lastSize = { c: cols, r: rows };
    ccExec(cc, `refresh-client -C ${cols}x${rows}`);
  }
  async function openTmuxNative(remote: Remote) {
    if (isLocal(remote.id)) return toast("Native tmux mode is for SSH remotes.", "error");
    const authKind: AuthKind = remote.auth ?? "key";
    let keyPath = "";
    let identityId = remote.id;
    if (authKind === "key") {
      const identity = identities.find((i) => i.id === remote.identityId);
      if (!identity) return toast("This remote has no valid identity.", "error");
      keyPath = identity.keyPath;
      identityId = identity.id;
    }
    const ctrlSid = crypto.randomUUID();
    const tabId = crypto.randomUUID();
    const cc: CcSession = { ctrlSid, remote, tabId, ctrl: null as unknown as TmuxControl, unlisteners: [], pending: [], windows: new Map(), activeWindow: null, winName: {} };
    cc.ctrl = new TmuxControl({
      // tmux émet le premier %output (prompt du shell) AVANT tout %layout-change :
      // sans création à la demande, cette sortie est écrite dans le vide et le
      // panneau reste noir jusqu'à la première frappe. ccApplyLayout garde le
      // panneau (il est dans le layout) et le placera dans la grille.
      output: (pane, bytes) => { const cs = ccEnsurePane(cc, pane); lastOut[cs] = Date.now(); sessions.get(cs)?.term.write(bytes); },
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
      toast(`native tmux: ${e}`, "error");
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
  function openLeaf(sid: string) {
    const tab = activeTab && !activeTab.root ? activeTab : null;
    if (tab) { tab.root = { leaf: sid }; tab.active = sid; }
    else { newTab(); const t = tabs[tabs.length - 1]; t.root = { leaf: sid }; t.active = sid; }
  }
  /** Insère un panneau (terminal OU navigateur) selon le mode du picker. */
  function placeLeaf(makeSid: () => string) {
    if (modal?.type !== "picker") return;
    const m = modal;
    modal = null;
    if (m.projectId) {
      const p = projects.find((x) => x.id === m.projectId);
      if (!p) return;
      const wasOpen = projectTabs(p.id).length > 0;
      const sid = makeSid();
      addProjectTerminal(p, sid); // un terminal = un onglet ; regrouper en split reste un geste explicite
      // projet fermé : ne pas reserialiser (écraserait ses vues), ajouter la vue.
      if (wasOpen) persistProject(p); else appendProjectView(p, sid);
    } else if (m.dir && m.sid && m.tabId) {
      const tab = tabs.find((t) => t.id === m.tabId);
      if (!tab?.root) return;
      const newSid = makeSid();
      tab.root = withSplit(tab.root, m.sid, m.dir, newSid);
      tab.active = newSid;
      if (tab.projectId) persistProject(projects.find((p) => p.id === tab.projectId)!);
    } else if (m.tabId) {
      const tab = tabs.find((t) => t.id === m.tabId);
      if (tab && !tab.root) { const sid = makeSid(); tab.root = { leaf: sid }; tab.active = sid; }
      else openLeaf(makeSid());
    } else {
      openLeaf(makeSid());
    }
  }
  function doPick(remote: Remote) {
    if (modal?.type !== "picker") return;
    const m = modal;
    if (m.cc && !m.dir && !m.projectId && !isLocal(remote.id)) {
      modal = null;
      openTmuxNative(remote);
      return;
    }
    // le terminal rejoue les commandes du projet où il atterrit : soit le projet
    // visé par le « + », soit celui de l'onglet qu'on splitte.
    const pid = m.projectId ?? tabs.find((t) => t.id === m.tabId)?.projectId;
    const cmd = (pid && projects.find((x) => x.id === pid)?.cmd) || "";
    placeLeaf(() => newSession(remote, cmd));
  }

  // ─── panneaux navigateur (aperçu web intégré à la grille) ────────────────
  function newBrowser(url = ""): string {
    const sid = crypto.randomUUID();
    browsers[sid] = { url, bar: url, reloadKey: 0 };
    return sid;
  }
  function doPickBrowser() { placeLeaf(() => newBrowser()); }
  function normUrl(u: string): string {
    u = u.trim();
    if (!u) return "";
    if (/^https?:\/\//i.test(u)) return u;
    // localhost / IP / :port → http (cas dev) ; sinon https
    const local = /localhost|127\.0\.0\.1|0\.0\.0\.0|^\d{1,3}(\.\d{1,3}){3}|^:\d/.test(u);
    return (local ? "http://" : "https://") + u;
  }
  function browserGo(sid: string) {
    const b = browsers[sid];
    if (!b) return;
    const u = normUrl(b.bar);
    if (u) browsers[sid] = { ...b, url: u, bar: u };
  }
  function reloadBrowser(sid: string) {
    const b = browsers[sid];
    if (b) browsers[sid] = { ...b, reloadKey: b.reloadKey + 1 };
  }
  function closeBrowser(sid: string) {
    delete browsers[sid];
    if (zoomedSid === sid) zoomedSid = null;
    for (const t of tabs) {
      if (t.root && leaves(t.root).includes(sid)) {
        t.root = withoutLeaf(t.root, sid);
        if (t.active === sid) t.active = firstLeaf(t.root);
        if (!t.root && tabs.length > 1) closeTab(t);
      }
    }
  }

  // ─── déplacement de panneaux (drag & drop, + sur projet) ─────────────────
  let dragSid = $state<string | null>(null);
  let dropTarget = $state<string | null>(null); // id de projet ou "standalone"
  let dropRow = $state<{ sid: string; mode: "before" | "after" | "merge" } | null>(null); // réordonner / fusionner

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
  /** Ajoute un terminal à un projet comme une vue séparée (nouvel onglet), pas un split. */
  function addProjectTerminal(p: Project, sid: string) {
    const tab: Tab = { id: crypto.randomUUID(), root: { leaf: sid }, active: sid, projectId: p.id };
    tabs.push(tab);
    activeTabId = tab.id;
  }
  /** Ré-enregistre les vues ouvertes d'un projet dans sa définition. À n'appeler
   *  QUE si le projet est réellement ouvert : sinon les onglets ouverts ne
   *  représentent pas toutes ses vues et on effacerait celles restées fermées.
   *  ponytail: ne reconcilie pas une ouverture PARTIELLE (projet ouvert puis un de
   *  ses onglets fermé, puis modifié) ; cas rare, à traiter si ça mord. */
  function persistProject(p: Project) {
    const views = projectTabs(p.id)
      .map((t) => (t.root ? serializeTree(t.root) : null))
      .filter((v): v is ProjNode => !!v);
    if (views.length) {
      p.views = views;
      delete p.root; // migre l'ancien format
    }
    save();
  }
  /** Ajoute un pane comme NOUVELLE vue sans reserialiser : à utiliser quand le
   *  projet n'est pas ouvert (persistProject ne verrait que ce pane et écraserait
   *  les vues sauvegardées — glisser un pane sur un projet fermé les perdait). */
  function appendProjectView(p: Project, sid: string) {
    const view = serializeTree({ leaf: sid });
    if (view) { p.views = [...projectViews(p), view]; delete p.root; }
    save();
  }
  function movePaneToProject(sid: string, p: Project) {
    const from = tabs.find((t) => leaves(t.root).includes(sid));
    if (from?.projectId === p.id) return; // déjà dans ce projet
    const fromProject = from?.projectId ? projects.find((x) => x.id === from.projectId) : null;
    const wasOpen = projectTabs(p.id).length > 0;
    extractPane(sid);
    addProjectTerminal(p, sid); // devient une vue séparée du projet
    if (fromProject) persistProject(fromProject);
    // projet fermé : ne pas reserialiser (écraserait ses vues), ajouter la vue.
    if (wasOpen) persistProject(p); else appendProjectView(p, sid);
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
  /** Fusionne deux terminaux : `fromSid` rejoint la vue de `toSid` en split. */
  function mergeTerminal(fromSid: string, toSid: string) {
    if (fromSid === toSid) return;
    const fromTab = tabs.find((t) => leaves(t.root).includes(fromSid));
    const toTab = tabs.find((t) => leaves(t.root).includes(toSid));
    if (!fromTab || !toTab || fromTab === toTab || !toTab.root) return; // déjà dans la même vue
    const fromProject = fromTab.projectId ? projects.find((p) => p.id === fromTab.projectId) : null;
    const toProject = toTab.projectId ? projects.find((p) => p.id === toTab.projectId) : null;
    extractPane(fromSid); // retire de sa vue (supprime l'onglet s'il devient vide)
    toTab.root = withSplit(toTab.root, toSid, "h", fromSid); // à côté de toSid
    toTab.active = fromSid;
    activeTabId = toTab.id;
    if (fromProject && fromProject !== toProject) persistProject(fromProject);
    if (toProject) persistProject(toProject);
  }
  /** Dépôt sur une ligne de terminal : bord haut/bas → réordonner (autonomes), milieu → fusionner. */
  function paneDropzone(sid: string, sub: boolean) {
    return {
      ondragover: (e: DragEvent) => {
        if (!dragSid || dragSid === sid) return;
        e.preventDefault();
        e.stopPropagation();
        const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
        const y = (e.clientY - r.top) / r.height;
        // réordonner seulement entre deux terminaux autonomes ; fusion partout ailleurs
        const canReorder = !sub && isStandalone(dragSid);
        dropRow = { sid, mode: canReorder && y < 0.3 ? "before" : canReorder && y > 0.7 ? "after" : "merge" };
        dropTarget = null;
      },
      ondragleave: () => {
        if (dropRow?.sid === sid) dropRow = null;
      },
      ondrop: (e: DragEvent) => {
        if (!dragSid || dragSid === sid) return;
        e.preventDefault();
        e.stopPropagation();
        const mode = dropRow?.mode ?? "merge";
        if (mode === "merge") mergeTerminal(dragSid, sid);
        else reorderTerminal(dragSid, sid, mode === "after");
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
      return s && !s.cc ? { remoteId: s.remote.id, id: s.key } : null;
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
  /** Vues d'un projet (compat : ancien format `root` = une seule vue). */
  function projectViews(p: Project): ProjNode[] {
    return p.views ?? (p.root ? [p.root] : []);
  }
  function projectTabs(pid: string): Tab[] {
    return tabs.filter((t) => t.projectId === pid);
  }
  function projRemote(remoteId: string): Remote | undefined {
    return isLocal(remoteId) ? locals.find((l) => l.id === remoteId) : remotes.find((r) => r.id === remoteId);
  }
  function buildNode(n: ProjNode, cmd = ""): PaneNode | null {
    if ("remoteId" in n) {
      const remote = projRemote(n.remoteId);
      if (!remote) return null;
      return { leaf: newSession(remote, cmd, n.id) };
    }
    const a = buildNode(n.a, cmd);
    const b = buildNode(n.b, cmd);
    if (!a) return b;
    if (!b) return a;
    return { dir: n.dir, ratio: n.ratio ?? 0.5, a, b };
  }
  function openProject(p: Project) {
    let last: string | null = null;
    for (const view of projectViews(p)) {
      const root = buildNode(view, p.cmd);
      if (!root) continue;
      const tab: Tab = { id: crypto.randomUUID(), root, active: firstLeaf(root), projectId: p.id };
      tabs.push(tab);
      last = tab.id;
    }
    if (!last) {
      toast("None of this project's remotes exist yet.", "error");
      return;
    }
    activeTabId = last;
  }
  /** L'inverse d'openProject : range le projet en fermant ses onglets. Sa
   *  définition n'est pas touchée (aucun persistProject sur une fermeture) et
   *  tmux garde les sessions côté serveur — la réouverture les réattache par
   *  `key`. Ranger ne perd donc rien, ni layout ni travail en cours. */
  function closeProject(p: Project) {
    for (const t of projectTabs(p.id)) closeTab(t);
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
      persistProject(existing); // enregistre toutes ses vues ouvertes
    } else {
      const p: Project = { id: crypto.randomUUID(), name, views: [root] };
      projects = [...projects, p];
      tab.projectId = p.id;
    }
    toast(`Project "${name}" saved`, "success");
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
    // isLocal plutôt que `locals` : la détection WSL est async, un projet ne doit
    // jamais être purgé parce qu'elle n'a pas encore répondu.
    projects = projects.filter((p) => projectViews(p).flatMap(projLeaves).some((l) => isLocal(l.remoteId) || remotes.some((x) => x.id === l.remoteId)));
    confirmDeleteId = null;
    await save();
  }
  async function openSshImport() {
    try {
      const hosts = (await rpc<ImportHost[]>("ssh_config_parse")) ?? [];
      const fresh = hosts.filter((h) => !remotes.some((r) => r.host === h.hostName && r.user === h.user));
      if (!fresh.length) return toast("No new hosts to import (SSH / VS Code)", "info");
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
        const id: Identity = { id: crypto.randomUUID(), name: h.identityFile.split("/").pop() ?? "key", keyPath: h.identityFile, hasPassphrase: false };
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
    if (!i.name) i.name = i.keyPath.split("/").pop() ?? "key";
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
      toast(`"${i.name}" is used by a remote.`, "error");
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
    projects = projects.map((x) => (x.id === p.id ? p : x)); // remplace sur place : garde l'ordre du sidebar
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
      s.term.options.fontFamily = fontStack(settings.fontFamily);
      s.term.options.theme = activeTheme();
      s.term.options.cursorStyle = settings.cursorStyle;
      s.term.options.cursorBlink = settings.cursorBlink;
      s.term.options.scrollback = settings.scrollback;
      s.term.options.lineHeight = settings.lineHeight;
      s.fit.fit();
    }
    save();
  }

  // ─── sync config Claude Code ──────────────────────────────────────────────
  const syncedRemotes = new Set<string>();
  const probedRemotes = new Set<string>(); // « déjà équipé ? » demandé au VPS : 1 fois par lancement
  function remoteParams(remote: Remote) {
    const auth: AuthKind = remote.auth ?? "key";
    if (auth !== "key") {
      // password : identityId = id du remote (clé Keychain) ; agent : ignoré
      return { host: remote.host, port: Number(remote.port), user: remote.user, keyPath: "", identityId: remote.id, auth };
    }
    const identity = identities.find((i) => i.id === remote.identityId)!;
    return { host: remote.host, port: Number(remote.port), user: remote.user, keyPath: identity.keyPath, identityId: identity.id, auth };
  }
  /** Params de `claude_sync` : la connexion + les réglages arabel qui atterrissent
   *  dans le settings.json distant. Un seul endroit, sinon un appel oublie un champ. */
  function syncParams(remote: Remote) {
    return { ...remoteParams(remote), agentTeams: settings.agentTeams, agentTeamPanes: settings.agentTeamPanes };
  }
  async function claudeSetup(sid: string, remote: Remote) {
    const s = sessions.get(sid);
    if (!s) return;
    try {
      if (!syncedRemotes.has(remote.id)) {
        s.term.write("\x1b[90m[arabel] syncing Claude Code config…\x1b[0m\r\n");
        const msg = await rpc<string>("claude_sync", syncParams(remote));
        syncedRemotes.add(remote.id);
        s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
      }
      await rpc("ssh_write", { sessionId: sid, data: "claude\n" });
    } catch (e) {
      s.term.write(`\x1b[31m[arabel] sync failed: ${e}\x1b[0m\r\n`);
      toast(`Claude sync failed: ${e}`, "error");
    }
  }
  async function enhanceShell(sid: string) {
    const s = sessions.get(sid);
    if (!s || isLocal(s.remote.id)) return;
    s.term.write("\r\n\x1b[90m[arabel] installing autosuggestions…\x1b[0m\r\n");
    try {
      const msg = await rpc<string>("shell_enhance", remoteParams(s.remote));
      s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
      toast("Autosuggestions installed", "success");
    } catch (e) {
      s.term.write(`\x1b[31m[arabel] ${e}\x1b[0m\r\n`);
      toast(`Autosuggestions: ${e}`, "error");
    }
  }
  async function syncNow(sid: string) {
    const s = sessions.get(sid);
    if (!s || isLocal(s.remote.id)) return;
    s.term.write("\r\n\x1b[90m[arabel] injecting config…\x1b[0m\r\n");
    try {
      const msg = await rpc<string>("claude_sync", syncParams(s.remote));
      syncedRemotes.add(s.remote.id);
      s.term.write(`\x1b[90m[arabel] ${msg}\x1b[0m\r\n`);
      // active le suivi Claude pour ce remote : flag persistant + écoute des hooks
      // → le panneau apparaît dans le dashboard Agents et son état remonte
      const wasTracked = s.remote.claude;
      if (!wasTracked) {
        s.remote.claude = true;
        remotes = [...remotes]; // force la réactivité du dashboard
        await save();
      }
      rpc("events_watch", { remoteId: s.remote.id, ...remoteParams(s.remote) }).catch(() => {});
      s.term.write("\x1b[90m[arabel] agent tracking enabled (see the Agents section)\x1b[0m\r\n");
      toast(wasTracked ? "Claude config synced" : "Claude tracking enabled for this remote", "success");
    } catch (e) {
      s.term.write(`\x1b[31m[arabel] ${e}\x1b[0m\r\n`);
      toast(String(e), "error");
    }
  }

  // ─── attentions (hooks Claude Code) ──────────────────────────────────────
  // question/options : extraits du buffer du terminal quand Claude pose un choix
  // (aucun hook ne les expose — ils ne vivent que dans le rendu de la TUI).
  type Attention = { id: string; sid: string; remoteId: string; kind: "stop" | "notif"; message: string; question?: string; options?: string[] };
  let attentions = $state<Attention[]>([]);
  // activité live par pane, alimentée par PreToolUse/UserPromptSubmit : ce que
  // Claude fait en ce moment, visible dans le dashboard même onglet non focus.
  let activity = $state<Record<string, { label: string; tool: string; at: number }>>({});
  // état persistant du pane (dernier hook gagne). « done » est collant : une
  // frappe au clavier ne doit PAS le refaire passer en « working » — sinon un
  // agent qui a fini réaffiche « travaille » dès qu'on touche le terminal.
  let paneStatus = $state<Record<string, "working" | "waiting" | "done">>({});
  // noms de terminaux : renommage manuel (prioritaire) + titre émis par le shell
  // (séquence OSC, ex. cwd/commande) en repli. Sinon → nom du remote (+ cmd).
  // savedTitles est indexé par la key STABLE du panneau (nom tmux, persistée dans
  // le workspace via serializeTree.id) → les noms survivent aux redémarrages.
  let savedTitles = $state<Record<string, string>>({});
  let autoTitles = $state<Record<string, string>>({});
  let renamingSid = $state<string | null>(null);
  let renameValue = $state("");
  // panneaux navigateur (iframe) : vivent dans l'arbre de splits comme un terminal
  let browsers = $state<Record<string, { url: string; bar: string; reloadKey: number }>>({});
  const isBrowser = (sid: string) => sid in browsers;
  function sessLabel(sid: string): string {
    if (isBrowser(sid)) {
      const u = browsers[sid]?.url;
      if (!u) return "New tab";
      try { return new URL(u).host || u; } catch { return u; }
    }
    const s = sessions.get(sid);
    if (!s) return "";
    // pas de suffixe de commande : elle est au projet, donc identique sur tous
    // ses panes — autoTitles (cwd/commande via OSC) les distingue déjà.
    return savedTitles[s.key] || autoTitles[sid] || s.remote.name;
  }
  function startRename(sid: string) {
    if (isBrowser(sid)) return; // un navigateur s'intitule par son URL
    renamingSid = sid;
    renameValue = sessLabel(sid); // pré-rempli avec le nom affiché, éditable
  }
  function commitRename() {
    if (!renamingSid) return;
    const s = sessions.get(renamingSid);
    const v = renameValue.trim();
    if (s) {
      if (v) savedTitles[s.key] = v;
      else delete savedTitles[s.key]; // vide → repli sur titre auto / nom du remote
      save(); // persiste le nom tout de suite (un renommage ne change pas la structure)
    }
    renamingSid = null;
  }
  // valide au blur, mais ignore un blur transitoire (re-render) : on ne conclut
  // qu'au frame suivant, si aucun champ de renommage n'a repris le focus.
  function renameBlur() {
    requestAnimationFrame(() => {
      const a = document.activeElement;
      const stillEditing = a instanceof HTMLInputElement && (a.classList.contains("row-rename") || a.classList.contains("pane-rename"));
      if (renamingSid && !stillEditing) commitRename();
    });
  }
  // horloge de rafraîchissement : le buffer xterm change sans passer par notre
  // code, donc on ré-évalue le statut visuel du terminal toutes les 500 ms.
  let liveTick = $state(0);
  $effect(() => {
    const id = setInterval(() => liveTick++, 500);
    return () => clearInterval(id);
  });
  // horodatage de la dernière sortie / entrée par pane (non réactif : relu à
  // chaque liveTick, sinon on re-rendrait à chaque octet). Sert au spinner
  // générique (process qui tourne) et à tuer le faux « done » après un envoi.
  const lastOut: Record<string, number> = {};
  const lastInput: Record<string, number> = {};
  const lastSubmit: Record<string, number> = {}; // dernière vraie soumission (Entrée)
  // dernier instant où le pane a affiché « esc to interrupt » (= Claude générait).
  // Sa disparition est ce qui nous dit qu'un tour est fini, sans dépendre des hooks.
  const sawWorking: Record<string, number> = {};
  // préférences → module d'alertes (seul endroit qui parle son/notif système)
  $effect(() => {
    notifPrefs.sounds = settings.sounds;
    notifPrefs.notifications = settings.notifications;
  });

  const recentHooks = new Map<string, number>(); // dédoublonnage (plusieurs watchers/listeners possibles)
  listen<{ remoteId: string; line: string }>("arabel-hook", (ev) => {
    let parsed: any = {};
    try {
      parsed = JSON.parse(ev.payload.line);
    } catch {
      return;
    }
    const hook = parsed.event ?? {};
    const name: string = hook.hook_event_name ?? "";
    const sid: string = parsed.pane ?? "";
    const remoteId = ev.payload.remoteId;

    // Claude bosse : on met à jour l'activité live du pane et on efface une
    // éventuelle attente (il a repris la main).
    if (name === "PreToolUse" || name === "UserPromptSubmit") {
      const tool = name === "UserPromptSubmit" ? "" : (hook.tool_name ?? "");
      const label = name === "UserPromptSubmit" ? "thinking…" : toolLabel(hook.tool_name, hook.tool_input);
      if (sid) { activity = { ...activity, [sid]: { label, tool, at: Date.now() } }; paneStatus = { ...paneStatus, [sid]: "working" }; }
      clearPaneAttentions(sid);
      return;
    }
    if (name !== "Stop" && name !== "Notification") return; // PostToolUse & co : ignorés

    const kind: "stop" | "notif" = name === "Stop" ? "stop" : "notif";
    const message = hook.message ?? (kind === "stop" ? "Claude finished" : "Claude is waiting for a response");
    // le même événement peut arriver en double (tail relancé, listener HMR) → on ignore les répétitions
    const dsig = `${remoteId}|${sid}|${kind}|${message}`;
    const now = Date.now();
    if (now - (recentHooks.get(dsig) ?? 0) < 2000) return;
    recentHooks.set(dsig, now);
    // Stop qui tombe juste après un envoi clavier → c'est le tour PRÉCÉDENT qui
    // se termine alors qu'on relance déjà : pas une vraie fin, on l'ignore
    // (sinon « done » (son + toast) alors que Claude repart aussitôt).
    if (kind === "stop" && sid && now - (lastInput[sid] ?? 0) < 1500) return;
    if (sid) paneStatus = { ...paneStatus, [sid]: kind === "stop" ? "done" : "waiting" };
    if (kind === "stop") {
      attentions = attentions.filter((a) => a.sid !== sid || a.kind !== "notif");
      if (sid) { const { [sid]: _, ...rest } = activity; activity = rest; } // plus d'activité en cours
    }
    const att: Attention = { id: crypto.randomUUID(), sid, remoteId, kind, message };
    attentions = [...attentions, att].slice(-20);
    // question à choix : on lit les options dans le terminal (léger différé, le
    // temps que la TUI ait fini de rendre le menu).
    if (kind === "notif" && sid) setTimeout(() => patchMenu(att.id, sid), 140);
    // notif système + son : pas ici, dans l'effet lié à liveStatus (plus bas)
  });

  // ─── statut vivant dérivé du terminal (indépendant des hooks) ────────────
  // Claude Code imprime des marqueurs fiables dans son TUI : « esc to interrupt »
  // quand il travaille, un menu « ❯ 1. … » / « esc to cancel » quand il attend.
  // On lit ça dans le buffer xterm → l'indicateur marche même sans hooks.
  function bottomText(sid: string, n: number): string {
    const term = sessions.get(sid)?.term;
    if (!term) return "";
    const buf = term.buffer.active;
    // Fenêtre ancrée sur le CURSEUR, pas sur buf.length : tant que l'écran n'est
    // pas rempli, buf.length compte les lignes vides du bas de la vue (40 lignes
    // pour ~20 écrites) et les 16 dernières étaient toutes blanches → marqueur
    // jamais vu en début de session. On lit n lignes au-dessus du curseur, et on
    // va jusqu'au bout du buffer car la TUI dessine son footer sous le curseur.
    const from = Math.max(0, Math.min(buf.baseY + buf.cursorY, buf.length - 1) - n + 1);
    let out = "";
    for (let i = from; i < buf.length; i++)
      out += (buf.getLine(i)?.translateToString(true) ?? "") + "\n";
    return out;
  }
  function liveStatus(sid: string): "working" | "waiting" | "done" | null {
    void liveTick; // dépendance réactive : ré-évalué à chaque tick
    const s = sessions.get(sid);
    if (!s) return null;
    const now = Date.now();
    // marqueurs du TUI Claude lus dans le buffer — fiables quel que soit le pane,
    // et seule source de « done » sans hooks (cf. $lib/notify).
    const turn = claudeTurn(bottomText(sid, 16), sawWorking[sid] ?? 0, lastSubmit[sid] ?? 0, now);
    sawWorking[sid] = turn.seen;
    if (turn.status) return turn.status;
    if (s.remote.claude) {
      // pane Claude : PAS de détection par volume de sortie — sa TUI se redessine
      // en continu (curseur, footer), ce qui faisait clignoter « running » en
      // permanence. On s'appuie sur les marqueurs ci-dessus + les hooks.
      if (now - (lastSubmit[sid] ?? 0) < 2000) return "working"; // tu viens d'envoyer (Entrée) → il repart (pas « done »)
      const h = paneStatus[sid];
      if (h === "waiting") return "waiting";
      if (h === "working" && activity[sid] && now - activity[sid].at < 15000) return "working"; // hook d'outil récent
      if (h === "done") return "done";
      return null;
    }
    // pane non-Claude : spinner générique tant qu'une commande crache de la sortie
    return now - (lastOut[sid] ?? 0) < 1000 ? "working" : null;
  }

  // ─── alerte (notif système + son) ────────────────────────────────────────
  // Branchée sur liveStatus, PAS sur les hooks : les hooks ne tombent que si le
  // remote a été synchronisé, alors que l'indicateur du pane vient du buffer
  // xterm. C'est ce décalage qui donnait un badge « waiting » muet et sans notif.
  // Aucun filtre sur `remote.claude` non plus : ce drapeau n'est posé qu'à la
  // main sur un remote SSH, alors que l'indicateur vient des marqueurs du TUI
  // → un pane local affichait le badge sans jamais sonner. liveStatus ne rend
  // « waiting »/« done » que sur ces marqueurs (ou les hooks), donc un shell
  // ordinaire ne peut pas déclencher d'alerte ici.
  const alerted: Record<string, string> = {};
  $effect(() => {
    void liveTick; // ré-évalué avec l'indicateur
    for (const [sid] of sessions) {
      const st = liveStatus(sid) ?? "idle";
      const prev = alerted[sid];
      if (prev === st) continue;
      alerted[sid] = st;
      if (prev === undefined) continue; // premier passage : pas d'alerte au montage
      if (st !== "waiting" && st !== "done") continue;
      // tu regardes déjà ce panneau → tu vois l'indicateur, l'alerte n'apprend rien
      if (visiblePane(sid)) continue;
      const body = attentions.find((a) => a.sid === sid)?.message
        ?? (st === "done" ? "Claude finished" : "Claude is waiting for a response");
      notify(st, `Arabel — ${sessLabel(sid)}`, body);
    }
  });

  /** Résumé court de l'outil en cours (PreToolUse) pour le dashboard. */
  function toolLabel(tool?: string, input?: any): string {
    const base = (p?: string) => (p ? p.split("/").pop() : "") ?? "";
    const cut = (s: string, n: number) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
    switch (tool) {
      case "Bash": return cut(String(input?.command ?? ""), 42);
      case "Edit": case "MultiEdit": case "Write": case "NotebookEdit": return base(input?.file_path ?? input?.notebook_path);
      case "Read": return base(input?.file_path);
      case "Grep": case "Glob": return cut(String(input?.pattern ?? ""), 30);
      case "Task": return "subagent";
      case "WebFetch": case "WebSearch": return cut(String(input?.url ?? input?.query ?? ""), 34);
      default: return tool ? (tool.startsWith("mcp__") ? tool.slice(5).replace(/__/g, " ") : tool) : "working…";
    }
  }

  /** Lit le buffer xterm du pane et en extrait un bloc de choix numérotés. */
  function scrapeMenu(sid: string): { question: string; options: string[] } | null {
    const term = sessions.get(sid)?.term;
    if (!term) return null;
    const buf = term.buffer.active;
    const start = Math.max(0, buf.length - 40);
    const lines: string[] = [];
    for (let i = start; i < buf.length; i++) lines.push((buf.getLine(i)?.translateToString(true) ?? "").replace(/\s+$/, ""));
    // on retient le DERNIER bloc numéroté contigu de la fenêtre (le menu courant,
    // pas un menu déjà répondu resté plus haut dans le scrollback).
    let options: string[] = [], firstIdx = -1;
    let cur: string[] = [], curStart = -1;
    const flush = () => { if (cur.length >= 2) { options = cur; firstIdx = curStart; } cur = []; };
    for (let i = 0; i < lines.length; i++) {
      const m = lines[i].match(/^\s*[❯›▶>»]?\s*(\d)[.)]\s+(.+)$/); // « 1. Yes » / « ❯ 2. No »
      if (!m) { flush(); continue; }
      const n = +m[1];
      if (n === cur.length + 1) { if (!cur.length) curStart = i; cur.push(m[2].trim()); }
      else if (n === 1) { flush(); cur = [m[2].trim()]; curStart = i; } // nouveau bloc
      else flush();
    }
    flush();
    if (options.length < 2) return null;
    let question = "";
    for (let i = firstIdx - 1; i >= 0; i--) if (lines[i].trim()) { question = lines[i].trim(); break; }
    return { question, options };
  }
  function patchMenu(id: string, sid: string) {
    const menu = scrapeMenu(sid);
    if (!menu) return;
    attentions = attentions.map((a) => (a.id === id ? { ...a, question: menu.question, options: menu.options } : a));
  }
  function clearPaneAttentions(sid: string) {
    if (!sid) return;
    if (attentions.some((a) => a.sid === sid)) attentions = attentions.filter((a) => a.sid !== sid);
  }

  // badge sur l'icône du Dock
  $effect(() => setBadge(attentions.filter((a) => a.kind === "notif").length));

  function attentionTarget(a: Attention): string | null {
    if (a.sid && sessions.has(a.sid)) return a.sid;
    for (const [sid, s] of sessions) if (s.remote.id === a.remoteId) return sid;
    return null;
  }
  function gotoAttention(a: Attention) {
    const sid = attentionTarget(a);
    if (!sid) return dismissAttention(a);
    const tab = tabs.find((t) => leaves(t.root).includes(sid));
    if (tab) focusPane(tab, sid);
  }
  function answerAttention(a: Attention, keys: string) {
    const sid = attentionTarget(a);
    if (sid) rpc("ssh_write", { sessionId: sid, data: keys });
    if (keys === "\x1b") playSound("error"); // refus → ton descendant
    dismissAttention(a);
  }
  function dismissAttention(a: Attention) {
    attentions = attentions.filter((x) => x.id !== a.id);
  }
  function remoteAttention(rid: string): boolean {
    return attentions.some((a) => a.remoteId === rid);
  }
  /** Un pane du projet réclame ton attention. Surtout utile projet replié : les
   *  lignes de panes sont masquées, la pastille est le seul signal qui reste. */
  function projectAttention(pid: string): boolean {
    return attentions.some((a) => tabs.find((t) => leaves(t.root).includes(a.sid))?.projectId === pid);
  }
  /** Tu réponds au clavier dans un panneau Claude → on retire ses toasts en
   *  attente. L'état (working/waiting/done) reste piloté par les hooks. */
  function onClaudeInput(sid: string) {
    if (!sessions.get(sid)?.remote.claude) return;
    if (attentions.some((a) => attentionTarget(a) === sid))
      attentions = attentions.filter((a) => attentionTarget(a) !== sid);
  }


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
    // ponytail: WSL tourne sur le matériel de l'hôte (CPU/RAM partagés) → on
    // sert les mêmes métriques à chaque id local plutôt que de sonder la distro.
    const ids = [...new Set([...sessions.values()].map((s) => s.remote.id).filter(isLocal))];
    if (!ids.length) return;
    try {
      const m = await rpc<Metrics>("local_metrics");
      if (m) for (const id of ids) metrics[id] = m;
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
    // sysSsh : SFTP / forwards / métriques passent par russh, indisponibles → on
    // masque ces affordances pour ces remotes.
    return r && !isLocal(r.id) && !r.sysSsh ? r : null;
  }
  function joinPath(p: string, n: string): string {
    return p === "/" ? `/${n}` : `${p}/${n}`;
  }
  function parentPath(p: string): string {
    return p.split("/").slice(0, -1).join("/") || "/";
  }
  function humanSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1048576) return `${(n / 1024).toFixed(0)} KB`;
    if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`;
    return `${(n / 1073741824).toFixed(2)} GB`;
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
      toast(`Downloaded: ${local}`, "success");
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
        const sent = await rpc<string>("sftp_upload", { remoteId: r.id, ...remoteParams(r), path: joinPath(files.path, f.name), dataB64: b64 });
        toast(sent && sent !== f.name ? `${f.name} sent as ${sent} (name existed)` : `${f.name} sent to ${r.name}`, "success");
      } catch (err) {
        toast(`${f.name} : ${err}`, "error");
      }
    }
    filesLoad(files.path);
  }

  // ─── panneau source control (git sur le remote) ──────────────────────────
  type GEntry = { path: string; name: string; sub: string; staged: boolean; conflict: boolean; untracked: boolean };
  type GStatus = { branch: string; upstream: string; ahead: number; behind: number; entries: GEntry[] };
  type DLine = { t: "add" | "del" | "hunk" | "meta" | "ctx"; s: string };
  let git = $state<{
    open: boolean; remote: Remote | null; root: string; busy: boolean; isRepo: boolean;
    branch: string; upstream: string; ahead: number; behind: number;
    entries: GEntry[]; msg: string; branches: string[]; showBranches: boolean;
    log: string[]; showLog: boolean; fetching: boolean;
  }>({
    open: false, remote: null, root: "", busy: false, isRepo: true,
    branch: "", upstream: "", ahead: 0, behind: 0,
    entries: [], msg: "", branches: [], showBranches: false, log: [], showLog: false, fetching: false,
  });
  const gitStagedCount = $derived(git.entries.filter((e) => e.staged).length);

  async function gitRun(args: string[]): Promise<[number, string]> {
    const r = git.remote;
    if (!r) return [1, ""];
    // ne jette JAMAIS : les appelants finissent tous par gitRefresh() pour remettre
    // busy à false. Une coupure SSH qui rejette ici laisserait le panneau voilé à vie.
    return rpc<[number, string]>("git_run", { remoteId: r.id, ...remoteParams(r), cwd: git.root, args })
      .catch((e) => [1, String(e)] as [number, string]);
  }
  /** Parse `git status --porcelain=v2 --branch`. Pur → testable (voir gitStatusDemo). */
  function parseStatus(out: string): GStatus {
    const s: GStatus = { branch: "", upstream: "", ahead: 0, behind: 0, entries: [] };
    for (const line of out.split("\n")) {
      if (line.startsWith("# branch.head ")) s.branch = line.slice(14).trim();
      else if (line.startsWith("# branch.upstream ")) s.upstream = line.slice(18).trim();
      else if (line.startsWith("# branch.ab ")) {
        const [a, b] = line.slice(12).trim().split(" ");
        s.ahead = Math.abs(parseInt(a, 10)) || 0;
        s.behind = Math.abs(parseInt(b, 10)) || 0;
      } else if (line[0] === "1" || line[0] === "2" || line[0] === "u") {
        const sp = line.split(" ");
        // champs avant le chemin : 8 (type 1), 9 (type 2 = +score), 10 (u = +modes/hash de fusion)
        const skip = line[0] === "1" ? 8 : line[0] === "2" ? 9 : 10;
        // type 2 (renommage) : "path\torig" → on garde la destination
        const path = sp.slice(skip).join(" ").split("\t")[0];
        // « u » = conflit de fusion : jamais staged, cliquer doit faire `git add` (= résoudre)
        const conflict = line[0] === "u";
        s.entries.push({ path, ...splitPath(path), staged: !conflict && (sp[1] ?? "..")[0] !== ".", conflict, untracked: false });
      } else if (line[0] === "?") {
        const path = line.slice(2);
        s.entries.push({ path, ...splitPath(path), staged: false, conflict: false, untracked: true });
      }
    }
    s.entries.sort((a, b) => a.path.localeCompare(b.path));
    return s;
  }
  // auto-test du parseur (dev) : les 3 cas qui mordaient — chemin accentué (git le
  // « quote » en octal sans core.quotePath=false), espace dans le nom, ligne « u ».
  function gitStatusDemo() {
    const eq = (a: unknown, b: unknown, m: string) => {
      if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`git status: ${m} → ${JSON.stringify(a)}`);
    };
    const s = parseStatus(
      "# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n" +
        "1 M. N... 100644 100644 100644 aaa bbb src/a.ts\n" +
        "1 .M N... 100644 100644 100644 aaa bbb mon fichier.txt\n" +
        "2 R. N... 100644 100644 100644 aaa bbb R100 new.ts\told.ts\n" +
        "u UU N... 100644 100644 100644 100644 aaa bbb ccc café.txt\n" +
        "? untracked.txt\n",
    );
    eq([s.branch, s.upstream, s.ahead, s.behind], ["main", "origin/main", 2, 1], "branche");
    eq(s.entries.map((e) => e.path), ["café.txt", "mon fichier.txt", "new.ts", "src/a.ts", "untracked.txt"], "chemins");
    eq(s.entries.filter((e) => e.staged).map((e) => e.path), ["new.ts", "src/a.ts"], "staged");
    eq(s.entries.find((e) => e.conflict)?.path, "café.txt", "conflit");
    eq(s.entries.filter((e) => e.untracked).map((e) => e.path), ["untracked.txt"], "untracked");

    // parseDiff : une ligne supprimée « -- » ne doit PAS passer pour l'en-tête « --- »
    const d = parseDiff(
      "diff --git a/a.txt b/a.txt\nindex 111..222 100644\n--- a/a.txt\n+++ b/a.txt\n" +
        "@@ -1,3 +1,3 @@\n ctx\n-- \n+ajout\n",
    );
    eq(d.map((l) => l.t), ["meta", "meta", "meta", "meta", "hunk", "ctx", "del", "add"], "diff");
    eq(parseDiff("diff --git a/b b/b\nBinary files a/b and b/b differ\n").map((l) => l.t), ["meta", "meta"], "diff binaire");
    // conflit (--cc) : préfixe 2 colonnes → les deux côtés comptent comme des ajouts
    const cc = parseDiff("diff --cc f.txt\n@@@ -1,1 -1,1 +1,5 @@@\n++<<<<<<< HEAD\n +main\n++=======\n+ other\n++>>>>>>> other\n");
    eq(cc.map((l) => l.t), ["meta", "hunk", "add", "add", "add", "add", "add"], "diff conflit");
  }
  function splitPath(p: string): { name: string; sub: string } {
    const i = p.lastIndexOf("/");
    return i < 0 ? { name: p, sub: "" } : { name: p.slice(i + 1), sub: p.slice(0, i) };
  }
  const GIT_STATUS = ["-c", "core.quotePath=false", "status", "--porcelain=v2", "--branch"];
  /** `fetch` : rafraîchit aussi origin/* (voir gitFetch). Réservé à l'ouverture du
   *  panneau et au bouton ↻ — stage/commit/push n'ont pas besoin du réseau. */
  async function gitRefresh(fetch = false) {
    const r = git.remote;
    if (!r) return;
    git.busy = true;
    try {
      const [rc, top] = await gitRun(["rev-parse", "--show-toplevel"]);
      if (rc !== 0) { git.isRepo = false; git.entries = []; git.busy = false; return; }
      git.isRepo = true;
      git.root = top.trim();
      // quotePath=false : sinon git rend « café.txt » en "caf\303\251.txt" (guillemets
      // inclus) → nom illisible ET `git add` derrière échoue sur le pathspec.
      const [, st] = await gitRun(GIT_STATUS);
      Object.assign(git, parseStatus(st));
      const [, br] = await gitRun(["branch", "--format=%(refname:short)"]);
      git.branches = br.split("\n").map((s) => s.trim()).filter(Boolean);
      if (git.showLog) await gitLoadLog();
    } catch (e) {
      toast(String(e), "error");
    }
    git.busy = false;
    if (fetch) gitFetch(r); // hors du voile : la liste s'affiche sans attendre le réseau
  }
  /** « ↓ behind » se calcule sur origin/*, qui n'avance QUE sur un fetch. Sans ça le
   *  compteur affiche 0 en permanence, même à 10 commits derrière — mensonge poli. */
  async function gitFetch(r: Remote) {
    git.fetching = true;
    const [rc] = await gitRun(["fetch", "--quiet"]);
    git.fetching = false;
    // origin injoignable : compteur périmé, pas de quoi alerter l'utilisateur
    if (rc !== 0) return;
    // le panneau a pu changer de remote / se fermer pendant l'aller-retour
    if (!git.open || git.remote?.id !== r.id) return;
    const [, st] = await gitRun(GIT_STATUS);
    if (git.remote?.id === r.id) Object.assign(git, parseStatus(st));
  }
  async function gitPointTo(r: Remote) {
    git.remote = r; git.showBranches = false; git.showLog = false;
    const home = await rpc<string>("sftp_home", { remoteId: r.id, ...remoteParams(r) }).catch(() => "~");
    git.root = r.dir ? (r.dir.startsWith("/") ? r.dir : joinPath(home, r.dir)) : home;
    gitRefresh(true);
  }
  function toggleGit() {
    if (git.open) { git.open = false; return; }
    const r = activeSshRemote();
    if (!r) return;
    git.open = true;
    gitPointTo(r);
  }
  // suit le remote du panneau actif
  $effect(() => {
    const r = activeSshRemote();
    if (git.open && r && r.id !== git.remote?.id) gitPointTo(r);
  });
  async function gitToggleEntry(entry: GEntry) {
    git.busy = true;
    await gitRun(entry.staged ? ["restore", "--staged", "--", entry.path] : ["add", "--", entry.path]);
    await gitRefresh();
  }
  async function gitStageAll() {
    git.busy = true;
    const anyUnstaged = git.entries.some((e) => !e.staged);
    await gitRun(anyUnstaged ? ["add", "-A"] : ["reset"]);
    await gitRefresh();
  }
  async function gitCommit() {
    if (!git.msg.trim()) return;
    if (!gitStagedCount) { toast("Nothing staged", "info"); return; }
    git.busy = true;
    const [rc, out] = await gitRun(["commit", "-m", git.msg]);
    if (rc === 0) { git.msg = ""; toast("Committed", "success"); }
    else toast(gitErr(out, "commit failed"), "error");
    await gitRefresh();
  }
  async function gitPush() {
    git.busy = true;
    const args = git.upstream ? ["push"] : ["push", "-u", "origin", "HEAD"];
    const [rc, out] = await gitRun(args);
    toast(rc === 0 ? "Pushed" : gitErr(out, "push failed"), rc === 0 ? "success" : "error");
    await gitRefresh();
  }
  /** Git noie l'essentiel sous des « hint: » (10 lignes pour un pull refusé) :
   *  on garde la ligne fatal/error, seule utile dans un toast. */
  function gitErr(out: string, fallback: string): string {
    const ls = out.split("\n").map((l) => l.trim()).filter(Boolean);
    return [...ls].reverse().find((l) => /^(fatal|error):/.test(l)) ?? ls.at(-1) ?? fallback;
  }
  async function gitPull() {
    git.busy = true;
    // --ff-only : un clic ne doit jamais fabriquer un commit de merge ni un conflit
    // en douce. Branches divergentes → échec net, et ça se règle au terminal.
    const [rc, out] = await gitRun(["pull", "--ff-only"]);
    toast(rc === 0 ? "Pulled" : gitErr(out, "pull failed"), rc === 0 ? "success" : "error");
    await gitRefresh();
  }
  async function gitCheckout(name: string) {
    git.showBranches = false;
    git.busy = true;
    const [rc, out] = await gitRun(["checkout", name]);
    if (rc !== 0) toast(gitErr(out, "checkout failed"), "error");
    await gitRefresh();
  }
  /** Découpe un diff unifié. Tout ce qui précède le 1er « @@ » est de l'en-tête :
   *  ça évite de prendre une ligne supprimée « -- » pour l'en-tête « --- a/x ».
   *  Un diff de conflit (--cc) préfixe sur 1 colonne PAR parent — largeur lue sur le
   *  « @@@ » — sinon le côté HEAD passerait pour du contexte. */
  function parseDiff(out: string): DLine[] {
    const lines: DLine[] = [];
    let w = 0; // largeur du préfixe ; 0 = pas encore dans un hunk (donc en-tête)
    for (const l of out.split("\n")) {
      if (l.startsWith("diff ")) { w = 0; lines.push({ t: "meta", s: l }); }
      else if (l.startsWith("@@")) { w = (/^@+/.exec(l)?.[0].length ?? 2) - 1; lines.push({ t: "hunk", s: l }); }
      else if (!w) lines.push({ t: "meta", s: l }); // index, ---, +++, new file, Binary files…
      else {
        const p = l.slice(0, w);
        lines.push({ t: p.includes("+") ? "add" : p.includes("-") ? "del" : "ctx", s: l });
      }
    }
    while (lines.length && !lines[lines.length - 1].s) lines.pop();
    return lines;
  }
  async function gitShowDiff(entry: GEntry) {
    const mode = entry.conflict ? "conflict" : entry.untracked ? "new file" : entry.staged ? "staged" : "working tree";
    modal = { type: "diff", path: entry.path, mode, text: "", loading: true };
    // le diff suit ce que dit la case : stagée → --cached, sinon l'arbre de travail.
    // untracked : absent de l'index, seul --no-index le montre (et sort rc=1 quand il
    // y a des différences → surtout ne pas traiter le rc comme une erreur ici).
    const args = entry.untracked
      ? ["diff", "--no-index", "--", "/dev/null", entry.path]
      : entry.staged
        ? ["diff", "--cached", "--", entry.path]
        : ["diff", "--", entry.path];
    const [, out] = await gitRun(["-c", "core.quotePath=false", ...args]);
    // l'utilisateur a pu cliquer ailleurs / fermer pendant l'aller-retour SSH
    if (modal?.type === "diff" && modal.path === entry.path) { modal.text = out; modal.loading = false; }
  }
  async function gitLoadLog() {
    const [, out] = await gitRun(["log", "--oneline", "--decorate", "-40"]);
    git.log = out.split("\n").filter(Boolean);
  }
  async function toggleGitLog() {
    if (git.showLog) { git.showLog = false; return; }
    git.busy = true;
    await gitLoadLog();
    git.showLog = true;
    git.busy = false;
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

  // ─── verbes arabel : l'agent pilote l'app depuis le serveur (OSC 7770) ─────
  //
  // FRONTIÈRE DE CONFIANCE. Ces octets viennent du serveur : ce n'est pas
  // forcément Claude qui les émet — un `cat` sur un fichier piégé suffit (même
  // exposition qu'OSC 52). Donc uniquement des actions inoffensives et visibles :
  // on valide tout, on n'exécute rien, on n'écrit aucun fichier.
  function arabelVerb(sid: string, raw: string) {
    const [verb, ...args] = raw.trim().split(/\s+/);
    if (verb === "preview") verbPreview(sid, args[0]);
    // verbe inconnu : ignoré en silence — le script distant peut être plus récent
    // que l'app (ou ces octets ne nous étaient pas destinés du tout).
  }

  /** `arabel preview <port>` : ouvre un panneau navigateur sur un port du serveur,
   *  en montant le tunnel à la demande. Réutilise un forward déjà ouvert sur ce
   *  port plutôt que d'en empiler un par appel. */
  async function verbPreview(sid: string, portArg: string) {
    const port = Number(portArg);
    if (!Number.isInteger(port) || port < 1 || port > 65535) return;
    const s = sessions.get(sid);
    const tab = tabs.find((t) => t.root && leaves(t.root).includes(sid));
    if (!s || !tab) return;
    let url = `http://localhost:${port}`; // pane local : le port est déjà ici
    if (!isLocal(s.remote.id)) {
      const open = forwards.find((f) => f.remoteId === s.remote.id && f.remotePort === port);
      let local = open?.localPort;
      if (!local) {
        const id = crypto.randomUUID();
        try {
          local =
            (await rpc<number>("port_forward_start", {
              id, localPort: 0, remoteHost: "localhost", remotePort: port, ...remoteParams(s.remote),
            })) ?? port; // démo
        } catch (e) {
          return toast(`preview ${port}: ${e}`, "error");
        }
        forwards = [...forwards, { id, remoteId: s.remote.id, remoteName: s.remote.name, localPort: local, remoteHost: "localhost", remotePort: port }];
      }
      url = `http://localhost:${local}`;
    }
    addPaneToTab(tab, newBrowser(url));
    toast(`Agent opened a preview of port ${port}`, "success");
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

  /** RGBA brut → PNG base64 via canvas (encodage natif du navigateur, pas de crate). */
  async function rgbaToPngB64(rgba: Uint8Array, w: number, h: number): Promise<string> {
    const canvas = document.createElement("canvas");
    canvas.width = w;
    canvas.height = h;
    const ctx = canvas.getContext("2d")!;
    ctx.putImageData(new ImageData(new Uint8ClampedArray(rgba), w, h), 0, 0);
    const blob: Blob = await new Promise((res, rej) =>
      canvas.toBlob((b) => (b ? res(b) : rej(new Error("PNG encoding failed"))), "image/png"),
    );
    const dataUrl: string = await new Promise((res, rej) => {
      const fr = new FileReader();
      fr.onload = () => res(fr.result as string);
      fr.onerror = () => rej(fr.error);
      fr.readAsDataURL(blob);
    });
    return dataUrl.slice(dataUrl.indexOf(",") + 1);
  }

  /** Colle le presse-papier. Image (session distante) → upload SFTP + insertion
   *  du chemin distant pour Claude Code. Sinon : `rawKey` fourni (Ctrl+V) → on
   *  renvoie la frappe brute pour préserver le collage natif de l'appli distante ;
   *  sans rawKey (⌘V) → collage texte via xterm (bracketed-paste-aware).
   *  Le statut passe par des toasts, jamais par `term.write` (corromprait une TUI). */
  async function pasteClipboard(sid: string, rawKey = "") {
    const s = sessions.get(sid);
    if (!s) return;
    if (inTauri && !isLocal(s.remote.id)) {
      const img = await readImage().catch(() => null); // rejette s'il n'y a pas d'image
      if (img) {
        try {
          const [rgba, { width, height }] = await Promise.all([img.rgba(), img.size()]);
          const name = `${Date.now()}-${crypto.randomUUID()}.png`;
          toast("Sending image to the VPS…");
          const b64 = await rgbaToPngB64(new Uint8Array(rgba), width, height);
          const path = await rpc<string>("sftp_paste_image", { ...remoteParams(s.remote), remoteId: s.remote.id, name, dataB64: b64 });
          // Claude Code attache l'image dès que son chemin absolu apparaît dans
          // l'invite. On l'insère, encadré d'espaces pour le délimiter.
          await rpc("ssh_write", { sessionId: sid, data: ` ${path} ` });
          toast("Image sent — path inserted for Claude", "success");
        } catch (e) {
          toast(`Image : ${e}`, "error");
        }
        return;
      }
    }
    // pas d'image
    if (rawKey) {
      // Ctrl+V : la frappe a été consommée pour tester le presse-papier ; comme il
      // n'y a pas d'image, on la restitue telle quelle à l'appli distante (Claude
      // Code fait alors son propre collage, readline son « quoted-insert », etc.)
      rpc("ssh_write", { sessionId: sid, data: rawKey }).catch(() => {});
    } else if (inTauri) {
      const t = await readText().catch(() => "");
      if (t) s.term.paste(t);
    }
  }

  async function pasteInto(sid: string, e: MouseEvent) {
    e.preventDefault();
    if (!inTauri) return;
    await pasteClipboard(sid);
    sessions.get(sid)?.term.focus();
  }

  /** Coupe le menu contextuel natif du webview (Recharger, Inspecter…) : look
   *  d'app native. On garde le menu natif dans les champs de formulaire (hors
   *  terminal) pour le copier/coller ; le terminal gère son propre clic droit. */
  function globalContextMenu(e: MouseEvent) {
    const t = e.target as HTMLElement | null;
    if (t && !t.closest(".pane-term")) {
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable) return;
    }
    e.preventDefault();
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
      case "enhance-shell": if (sid) enhanceShell(sid); break;
    }
  });

  // ─── clavier : combo → action ─────────────────────────────────────────────
  const MOD_CODES = new Set(["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"]);
  function keyName(e: KeyboardEvent): string {
    const c = e.code;
    if (c.startsWith("Key")) return c.slice(3); // KeyF → F
    if (c.startsWith("Digit")) return c.slice(5); // Digit1 → 1
    return c; // Comma, Enter, BracketRight…
  }
  /** Combo normalisé (ordre canonique) ou "" pour une touche modificatrice seule. */
  function comboOf(e: KeyboardEvent): string {
    if (MOD_CODES.has(e.code)) return "";
    const p: string[] = [];
    if (e.metaKey) p.push("Meta");
    if (e.ctrlKey) p.push("Ctrl");
    if (e.altKey) p.push("Alt");
    if (e.shiftKey) p.push("Shift");
    p.push(keyName(e));
    return p.join("+");
  }
  function keyOf(id: string): string {
    return settings.keymap?.[id] ?? DEFAULT_KEYS[id] ?? "";
  }
  function cycleTab(delta: number) {
    const i = tabs.findIndex((t) => t.id === activeTabId);
    if (i >= 0) activeTabId = tabs[(i + delta + tabs.length) % tabs.length].id;
  }
  /** Tous les panneaux dans l'ordre de la sidebar — terminaux libres, puis les
   *  projets. PAS l'ordre du tableau `tabs` : la sidebar regroupe par projet, et
   *  une flèche doit aller là où l'œil l'attend. */
  function paneOrder(): { tab: Tab; sid: string }[] {
    const ordered = [...tabs.filter((t) => !t.projectId), ...projects.flatMap((p) => projectTabs(p.id))];
    return ordered.flatMap((t) => leaves(t.root).map((sid) => ({ tab: t, sid })));
  }
  /** Panneau suivant / précédent, en traversant les projets et en bouclant. */
  function cyclePane(delta: number) {
    const all = paneOrder();
    if (all.length < 2) return;
    const i = all.findIndex((p) => p.tab.id === activeTabId && p.sid === activeTab?.active);
    const next = all[i < 0 ? 0 : (i + delta + all.length) % all.length];
    focusPane(next.tab, next.sid);
  }
  type Binding = { id: string; label: string; scope: "window" | "term"; run: (sid: string | null) => void };
  const KEYBINDINGS: Binding[] = [
    { id: "palette", label: "Command palette", scope: "window", run: () => { modal = modal?.type === "palette" ? null : { type: "palette", filter: "" }; paletteSel = 0; } },
    { id: "new-connection", label: "New terminal / connection", scope: "window", run: () => openPicker() },
    { id: "close-pane", label: "Close pane", scope: "window", run: () => { const sid = activeTab?.active; if (sid) closePane(sid); else if (activeTab) closeTab(activeTab); } },
    { id: "split-h", label: "Split right", scope: "window", run: () => { const sid = activeTab?.active; if (sid && activeTab) openPicker({ tabId: activeTab.id, sid, dir: "h" }); } },
    { id: "split-v", label: "Split down", scope: "window", run: () => { const sid = activeTab?.active; if (sid && activeTab) openPicker({ tabId: activeTab.id, sid, dir: "v" }); } },
    { id: "next-tab", label: "Next tab", scope: "window", run: () => cycleTab(1) },
    { id: "prev-tab", label: "Previous tab", scope: "window", run: () => cycleTab(-1) },
    { id: "next-pane", label: "Next terminal (down, across projects)", scope: "window", run: () => cyclePane(1) },
    { id: "prev-pane", label: "Previous terminal (up, across projects)", scope: "window", run: () => cyclePane(-1) },
    { id: "toggle-sidebar", label: "Toggle sidebar", scope: "window", run: () => { settings.sidebar = !settings.sidebar; save(); } },
    { id: "clear", label: "Clear terminal", scope: "window", run: () => { const sid = activeTab?.active; if (sid) sessions.get(sid)?.term.clear(); } },
    { id: "settings", label: "Open settings", scope: "window", run: () => (modal = { type: "settings" }) },
    { id: "search", label: "Find in terminal", scope: "window", run: () => { const sid = activeTab?.active; if (sid) openSearch(sid); } },
    { id: "zoom", label: "Zoom pane (fullscreen)", scope: "term", run: (sid) => sid && toggleZoom(sid) },
    { id: "copy", label: "Copy selection", scope: "term", run: (sid) => { const s = sid ? sessions.get(sid) : null; if (s?.term.hasSelection() && inTauri) writeText(s.term.getSelection()).catch(() => {}); } },
    { id: "paste", label: "Paste", scope: "term", run: (sid) => { if (sid && inTauri) pasteClipboard(sid); } },
  ];
  const bindById = new Map(KEYBINDINGS.map((b) => [b.id, b]));
  const actionCombos = $derived.by(() => {
    const m = new Map<string, string>();
    for (const b of KEYBINDINGS) { const c = keyOf(b.id); if (c) m.set(c, b.id); }
    return m;
  });

  // réattribution : l'utilisateur clique un raccourci puis presse la combo
  let recordingBind = $state<string | null>(null);
  let settingsTab = $state<"appearance" | "terminal" | "shortcuts">("appearance");
  // Capture de la combo en phase CAPTURE, sur window : sur macOS (WKWebView) cliquer
  // un <button> ne lui donne pas le focus clavier, donc son onkeydown ne se déclenchait
  // jamais → recordingBind restait armé et globalKeydown gelait tout le clavier. La
  // phase capture passe aussi AVANT xterm (qui sinon avale la touche via stopPropagation).
  $effect(() => {
    if (!recordingBind) return;
    const onKey = (e: KeyboardEvent) => recordKey(e, recordingBind!);
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });
  // quitter les réglages annule un enregistrement en cours (sinon recordingBind
  // resterait armé après fermeture de la modale).
  $effect(() => { if (modal?.type !== "settings") recordingBind = null; });
  function recordKey(e: KeyboardEvent, id: string) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") { recordingBind = null; return; }
    if (MOD_CODES.has(e.code)) return; // attend une vraie touche
    const combo = comboOf(e);
    if (!combo) return;
    // libère toute action qui détenait déjà cette combo (une combo = une action)
    const next: Record<string, string> = {};
    for (const b of KEYBINDINGS) { const cur = keyOf(b.id); next[b.id] = cur === combo && b.id !== id ? "" : cur; }
    next[id] = combo;
    settings.keymap = next;
    recordingBind = null;
    save();
  }
  function resetKey(id: string) { settings.keymap = { ...settings.keymap, [id]: DEFAULT_KEYS[id] }; save(); }
  function resetAllKeys() { settings.keymap = { ...DEFAULT_KEYS }; save(); }
  const MAC_SYM: Record<string, string> = { Meta: "⌘", Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Comma: ",", Enter: "↵", Space: "␣", BracketLeft: "[", BracketRight: "]", Backslash: "\\", Slash: "/", Minus: "-", Equal: "=", Backquote: "`", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
  const WIN_NAME: Record<string, string> = { Meta: "Win", Comma: ",", Enter: "Enter", Space: "Space", BracketLeft: "[", BracketRight: "]", ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→" };
  function formatCombo(combo: string): string {
    if (!combo) return "—";
    const parts = combo.split("+");
    return isMac ? parts.map((p) => MAC_SYM[p] ?? p).join("") : parts.map((p) => WIN_NAME[p] ?? p).join("+");
  }

  function globalKeydown(e: KeyboardEvent) {
    if (recordingBind) return; // capture gérée par l'input de réglage
    if (!e.metaKey && !e.ctrlKey && !e.altKey) {
      if (e.key === "Escape" && modal) modal = null;
      return;
    }
    // ⌘1-9 / Ctrl+Maj+1-9 : bascule d'onglet (famille fixe, e.code robuste au Maj)
    const digit = e.code.startsWith("Digit") ? Number(e.code.slice(5)) : NaN;
    if (appMod(e) && digit >= 1 && digit <= 9) {
      if (tabs[digit - 1]) { activeTabId = tabs[digit - 1].id; e.preventDefault(); }
      return;
    }
    const id = actionCombos.get(comboOf(e));
    if (id) {
      const b = bindById.get(id)!;
      if (b.scope === "window") { e.preventDefault(); b.run(activeTab?.active ?? null); }
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
    if (zoomedSid && zoomedSid !== sid) zoomedSid = null; // sinon on taperait dans un pane caché sous le zoom
    // le focus clavier est posé par l'$effect d'onglet (après le flush Svelte)
  }
  /** Panneau sous les yeux : fenêtre au premier plan et panneau affiché. */
  function visiblePane(sid: string): boolean {
    if (!document.hasFocus()) return false;
    return activeTab?.active === sid && (!zoomedSid || zoomedSid === sid);
  }

  // zoom : le panneau actif occupe tout l'onglet (⌘⇧Entrée)
  let zoomedSid = $state<string | null>(null);
  function toggleZoom(sid: string) {
    zoomedSid = zoomedSid === sid ? null : sid;
    requestAnimationFrame(() => sessions.get(sid)?.fit.fit());
  }

  // palette ⌘P : panneaux ouverts, projets, remotes, commandes
  let paletteSel = $state(0);
  type PaletteItem = { icon: "remote" | "project" | "pane" | "local" | "action"; label: string; sub: string; run: () => void; kbd?: string };
  function paletteItems(): PaletteItem[] {
    const items: PaletteItem[] = [];
    for (const t of tabs) {
      for (const sid of leaves(t.root)) {
        const s = sessions.get(sid);
        if (!s) continue;
        items.push({
          icon: isLocal(s.remote.id) ? "local" : "pane",
          label: sessLabel(sid), // même nom que la sidebar (renommage / titre auto)
          sub: settings.projects && t.projectId ? projects.find((p) => p.id === t.projectId)?.name ?? "project" : "open terminal",
          run: () => focusPane(t, sid),
        });
      }
    }
    if (settings.projects)
      for (const p of projects) {
        items.push({ icon: "project", label: p.name, sub: "project", run: () => { const o = openTabFor(p.id); o ? (activeTabId = o.id) : openProject(p); } });
        // « ranger » vit ici plutôt que sur la ligne : un 4e bouton au survol ne
        // laissait plus lire le nom du projet. Sans objet si le projet est fermé.
        if (projectTabs(p.id).length) items.push({ icon: "project", label: p.name, sub: "put away", run: () => closeProject(p) });
      }
    for (const l of locals)
      items.push({ icon: "local", label: l.name, sub: "new terminal", run: () => openRemote(l) });
    for (const r of remotes)
      items.push({ icon: "remote", label: r.name, sub: `${r.user}@${r.host}`, run: () => openRemote(r) });
    for (const b of KEYBINDINGS)
      if (b.scope === "window" && b.id !== "palette")
        items.push({ icon: "action", label: b.label, sub: "command", kbd: formatCombo(keyOf(b.id)), run: () => b.run(activeTab?.active ?? null) });
    return items;
  }
  function filteredPalette(filter: string): PaletteItem[] {
    const q = filter.toLowerCase();
    return paletteItems().filter((it) => (it.label + " " + it.sub).toLowerCase().includes(q)).slice(0, 40);
  }
  // ferme la palette AVANT d'exécuter : l'action peut rouvrir sa propre modale (picker, réglages)
  function runPalette(it: PaletteItem) { modal = null; it.run(); }
  function paletteNav(e: KeyboardEvent, filter: string) {
    const items = filteredPalette(filter);
    const n = items.length;
    if (e.key === "ArrowDown") { e.preventDefault(); paletteSel = n ? (paletteSel + 1) % n : 0; }
    else if (e.key === "ArrowUp") { e.preventDefault(); paletteSel = n ? (paletteSel - 1 + n) % n : 0; }
    else if (e.key === "Enter") { e.preventDefault(); const it = items[paletteSel] ?? items[0]; if (it) runPalette(it); }
  }
  // garde la ligne sélectionnée visible quand on navigue au clavier
  $effect(() => {
    if (modal?.type !== "palette") return;
    paletteSel;
    requestAnimationFrame(() => document.querySelector(".palette-list .row.sel")?.scrollIntoView({ block: "nearest" }));
  });
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
      attachWebgl(s);
    }
    s.fit.fit();
    s.term.focus();
  }
  /** Branche le renderer GPU (WebGL) et, en cas de perte de contexte, le
   *  RE-CRÉE au lieu de rester bloqué sur le renderer DOM (lent au scroll) —
   *  c'est ce qui manquait vs Terax. Cap à 3 tentatives pour éviter la boucle. */
  function attachWebgl(s: Sess) {
    if (s.webgl || !s.term.element) return;
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        s.webgl = false;
        if ((s.webglTries ?? 0) < 3) {
          s.webglTries = (s.webglTries ?? 0) + 1;
          setTimeout(() => attachWebgl(s), 400); // récupère le renderer GPU
        } else {
          console.warn("[arabel] WebGL keeps losing its context → staying on the DOM renderer (scroll less smooth)");
        }
      });
      s.term.loadAddon(webgl);
      s.webgl = true;
    } catch (e) {
      // WebGL indisponible (ex. WKWebView capricieux) → renderer DOM, scroll moins fluide
      console.warn("[arabel] WebGL renderer unavailable → DOM renderer (scroll may be less smooth):", e);
    }
  }
  function mountTerm(node: HTMLElement, sid: string) {
    attach(node, sid);
    let current = sid;
    const ro = new ResizeObserver(() => {
      const s = sessions.get(current);
      // panneau tmux -CC : xterm est piloté par tmux → on prévient tmux (il
      // redécoupe et renvoie un layout), on ne fit PAS au conteneur.
      if (s?.cc) { const cc = ccSessions.get(s.cc.ctrlSid); if (cc) ccResize(cc); }
      else s?.fit.fit();
    });
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
<svelte:window onkeydown={globalKeydown} oncontextmenu={globalContextMenu} onresize={onWindowResize} ondragover={(e) => e.preventDefault()} ondrop={(e) => e.preventDefault()} />

<!-- ─── icônes ─────────────────────────────────────────────────────────── -->
{#snippet choiceBtns(att: Attention)}
  {#if att.question}<span class="agent-q">{att.question}</span>{/if}
  <div class="choice-row wrap">
    {#each att.options ?? [] as opt, i}
      <button class="choice" title={opt} onclick={(e) => { e.stopPropagation(); answerAttention(att, String(i + 1)); }}><b>{i + 1}</b>&nbsp;{opt}</button>
    {/each}
  </div>
{/snippet}
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
{#snippet iBranch()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="4.5" cy="3.5" r="1.7"/><circle cx="4.5" cy="12.5" r="1.7"/><circle cx="11.5" cy="4.5" r="1.7"/><path d="M4.5 5.2v5.6M11.5 6.2c0 3-2.5 3.4-4.5 4"/></svg>{/snippet}
{#snippet iGlobe()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6"/><path d="M2 8h12M8 2c1.7 1.6 2.7 3.8 2.7 6S9.7 12.4 8 14M8 2C6.3 3.6 5.3 5.8 5.3 8S6.3 12.4 8 14"/></svg>{/snippet}
{#snippet iExternal()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M9 3h4v4M13 3 7.5 8.5M11 9.5V13H3V5h3.5"/></svg>{/snippet}
{#snippet iWarn()}<svg width="18" height="18" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M8 2 1.5 13.5h13zM8 6.5V10M8 11.8v.2"/></svg>{/snippet}
{#snippet iAlert()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="8" cy="8" r="6"/><path d="M8 5v3.5M8 10.8v.2"/></svg>{/snippet}
{#snippet iCheck()}<svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 8.5 6.5 11.5 12.5 4.5"/></svg>{/snippet}
{#snippet iSearch()}<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="7" cy="7" r="4.5"/><path d="M10.5 10.5 14 14"/></svg>{/snippet}
{#snippet iLogo()}<svg width="18" height="18" viewBox="0 0 20 20" fill="none"><rect x="1" y="1" width="18" height="18" rx="5.5" fill="var(--accent)"/><path d="M6 7l3 3-3 3M11 13.5h3.5" stroke="#fff" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg>{/snippet}
{#snippet iCopy()}<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 5.5V4a1.5 1.5 0 0 0-1.5-1.5H4A1.5 1.5 0 0 0 2.5 4v5A1.5 1.5 0 0 0 4 10.5h1.5"/></svg>{/snippet}
{#snippet iClipboard()}<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round"><rect x="3.5" y="3" width="9" height="11" rx="1.5"/><path d="M6 3V2.2c0-.4.3-.7.7-.7h2.6c.4 0 .7.3.7.7V3"/></svg>{/snippet}

<!-- icône par outil : « ce que l'agent fait » d'un coup d'œil -->
{#snippet toolIcon(tool: string)}
  {#if tool === "Bash"}{@render iTerminal()}
  {:else if tool === "Edit" || tool === "MultiEdit" || tool === "Write" || tool === "NotebookEdit"}{@render iPencil()}
  {:else if tool === "Read"}{@render iFile()}
  {:else if tool === "Grep" || tool === "Glob"}{@render iSearch()}
  {:else if tool === "WebFetch" || tool === "WebSearch"}{@render iGlobe()}
  {:else if tool === "Task"}{@render iBolt()}
  {:else}{@render iBolt()}{/if}
{/snippet}

<!-- statut vivant d'un pane agent (working / waiting / done), piloté par les hooks -->
{#snippet paneStat(sid: string)}
  {@const ps = liveStatus(sid)}
  {#if ps === "waiting"}
    <span class="pstat waiting" title="Waiting for your input">{@render iAlert()}<span class="pstat-label">waiting…</span></span>
  {:else if ps === "done"}
    <span class="pstat done" title="Finished">{@render iCheck()}<span class="pstat-label">done</span></span>
  {:else if ps === "working"}
    <span class="pstat working" title={activity[sid]?.label ?? "running…"}>
      {@render iSpinner(11)}
      {#if activity[sid]?.tool}{@render toolIcon(activity[sid].tool)}{/if}
      <span class="pstat-label">{activity[sid]?.label ?? "running…"}</span>
    </span>
  {/if}
{/snippet}

{#snippet sbSection(title: string, onAdd: (() => void) | null, addDisabled = false)}
  <div class="sb-head">
    <span>{title}</span>
    {#if onAdd}<button class="icon-btn sb-add" onclick={onAdd} disabled={addDisabled} title="Add">{@render iPlus()}</button>{/if}
  </div>
{/snippet}

{#snippet rowActions(id: string, onEdit: () => void, onDelete: () => void)}
  <span class="row-actions">
    {#if confirmDeleteId === id}
      <button class="confirm-del" onclick={(e) => { e.stopPropagation(); onDelete(); }}>Delete?</button>
    {:else}
      <button class="icon-btn" title="Edit" onclick={(e) => { e.stopPropagation(); onEdit(); }}>{@render iPencil()}</button>
      <button class="icon-btn" title="Delete" onclick={(e) => { e.stopPropagation(); confirmDeleteId = id; }}>{@render iTrash()}</button>
    {/if}
  </span>
{/snippet}

{#snippet remoteRow(r: Remote, onPick: (r: Remote) => void, withActions: boolean)}
  {@const auth = r.auth ?? "key"}
  {@const sub = auth === "key" ? (identities.find((i) => i.id === r.identityId)?.name ?? "no key") : auth}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="row" class:mgr={withActions} onclick={() => onPick(r)} title={r.id.startsWith("wsl:") ? `WSL · ${r.name}` : r.id === "local" ? "Local shell" : `${r.user}@${r.host}:${r.port}`}>
    <span class="row-icon">{#if isLocal(r.id)}{@render iLaptop()}{:else}{@render iTerminal()}{/if}</span>
    {#if withActions}
      <span class="row-main">
        <span class="row-label">{r.name}</span>
        <span class="row-sub">{r.user}@{r.host}:{r.port} · {sub}</span>
      </span>
    {:else}
      <span class="row-label">{r.name}</span>
    {/if}
    {#if r.claude}<span class="row-tag">claude</span>{/if}
    {#if remoteAttention(r.id)}<span class="dot attention"></span>{/if}
    {#if withActions}{@render rowActions(r.id, () => editRemote(r, true), () => deleteRemote(r))}{/if}
  </div>
{/snippet}

{#snippet sessRow(tab: Tab, sid: string, sub: boolean)}
  {@const s = sessions.get(sid)}
  {@const st = liveStatus(sid)}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div
    class="row"
    class:sub
    class:current={tab.id === activeTabId && tab.active === sid}
    class:dragging={dragSid === sid}
    class:drop-before={dropRow?.sid === sid && dropRow.mode === "before"}
    class:drop-after={dropRow?.sid === sid && dropRow.mode === "after"}
    class:drop-merge={dropRow?.sid === sid && dropRow.mode === "merge"}
    draggable={renamingSid !== sid}
    ondragstart={(e) => {
      dragSid = sid;
      e.dataTransfer?.setData("text/plain", sid);
      if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
    }}
    ondragend={() => { dragSid = null; dropTarget = null; dropRow = null; }}
    {...paneDropzone(sid, sub)}
    onclick={() => focusPane(tab, sid)}>
    <span class="row-icon">{#if isBrowser(sid)}{@render iGlobe()}{:else if s && isLocal(s.remote.id)}{@render iLaptop()}{:else}{@render iTerminal()}{/if}</span>
    {#if renamingSid === sid}
      <input class="row-rename" bind:value={renameValue} use:autofocus
        onclick={(e) => e.stopPropagation()}
        onkeydown={(e) => { e.stopPropagation(); if (e.key === "Enter") commitRename(); else if (e.key === "Escape") renamingSid = null; }}
        onblur={renameBlur} />
    {:else}
      <span class="row-label" ondblclick={(e) => { e.stopPropagation(); startRename(sid); }} title="Double-click to rename">{sessLabel(sid)}</span>
    {/if}
    {#if isBrowser(sid)}<!-- navigateur : pas de statut -->
    {:else if sessStatus[sid]?.status === "connecting"}<span class="row-spin">{@render iSpinner(11)}</span>
    {:else if sessStatus[sid]?.status === "error" || sessStatus[sid]?.status === "closed"}<span class="sstat error" title="Disconnected — reconnecting">{@render iAlert()}</span>
    {:else if st === "waiting"}<span class="sstat waiting" title="Waiting for your input">{@render iAlert()}</span>
    {:else if st === "working"}<span class="sstat working" title={activity[sid]?.label ?? "running…"}>{@render iSpinner(11)}</span>
    {:else if st === "done"}<span class="sstat done" title="Finished">{@render iCheck()}</span>{/if}
    <span class="row-actions">
      {#if !isBrowser(sid)}<button class="icon-btn" title="Rename (or double-click)" onclick={(e) => { e.stopPropagation(); startRename(sid); }}>{@render iPencil()}</button>{/if}
      <button class="icon-btn" title="Close" onclick={(e) => { e.stopPropagation(); closePane(sid); }}>{@render iClose()}</button>
    </span>
  </div>
{/snippet}

{#snippet browserPane(tab: Tab, sid: string)}
  {@const b = browsers[sid]}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="pane" class:active={tab.active === sid} class:zoomed={zoomedSid === sid} onclick={() => (tab.active = sid)}>
    <div class="pane-bar">
      <span class="pane-left">
        <span class="browser-ico">{@render iGlobe()}</span>
        <input
          class="url-input"
          bind:value={b.bar}
          placeholder="localhost:5173 — type a URL, ⏎ to go"
          onclick={(e) => e.stopPropagation()}
          onkeydown={(e) => { e.stopPropagation(); if (e.key === "Enter") browserGo(sid); }} />
      </span>
      <span class="pane-btns">
        <button class="icon-btn" title="Reload" onclick={() => reloadBrowser(sid)}>{@render iRefresh()}</button>
        <button class="icon-btn" title="Open in system browser" onclick={() => b.url && openInBrowser(b.url)}>{@render iExternal()}</button>
        <button class="icon-btn" title="Fullscreen (⇧⌘Enter)" onclick={() => toggleZoom(sid)}>{#if zoomedSid === sid}{@render iZoomOut()}{:else}{@render iZoom()}{/if}</button>
        <button class="icon-btn" title="Split right (⌘D)" onclick={() => openPicker({ tabId: tab.id, sid, dir: "h" })}>{@render iSplitH()}</button>
        <button class="icon-btn" title="Split down (⇧⌘D)" onclick={() => openPicker({ tabId: tab.id, sid, dir: "v" })}>{@render iSplitV()}</button>
        <button class="icon-btn" title="Close (⌘W)" onclick={() => closePane(sid)}>{@render iClose()}</button>
      </span>
    </div>
    <div class="browser-body">
      {#if b.url}
        {#key b.reloadKey}
          <iframe class="browser-frame" title="In-app browser" src={b.url}></iframe>
        {/key}
      {:else}
        <div class="browser-empty">{@render iGlobe()}<span>Type a URL above to preview — e.g. <code>localhost:5173</code></span></div>
      {/if}
    </div>
  </div>
{/snippet}

{#snippet paneTree(tab: Tab, node: PaneNode)}
  {#if "leaf" in node && isBrowser(node.leaf)}
    {@render browserPane(tab, node.leaf)}
  {:else if "leaf" in node}
    {@const s = sessions.get(node.leaf)}
    {@const st = sessStatus[node.leaf]}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="pane" class:active={tab.active === node.leaf} class:zoomed={zoomedSid === node.leaf} onclick={() => focusPane(tab, node.leaf)}>
      <div class="pane-bar">
        <span class="pane-left">
          {@render paneStat(node.leaf)}
          {#if renamingSid === node.leaf}
            <input class="pane-rename" bind:value={renameValue} use:autofocus
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => { e.stopPropagation(); if (e.key === "Enter") commitRename(); else if (e.key === "Escape") renamingSid = null; }}
              onblur={renameBlur} />
          {:else}
            <span class="pane-title" ondblclick={(e) => { e.stopPropagation(); startRename(node.leaf); }} title="Double-click to rename">{sessLabel(node.leaf)}</span>
          {/if}
        </span>
        <span class="pane-btns">
          {#if s && !isLocal(s.remote.id) && !s.cc}
            <button class="icon-btn" title="Enable Claude tracking (config + Agents dashboard)" onclick={() => syncNow(node.leaf)}>{@render iBolt()}</button>
          {/if}
          <button class="icon-btn" title="Fullscreen (⇧⌘Enter)" onclick={() => toggleZoom(node.leaf)}>{#if zoomedSid === node.leaf}{@render iZoomOut()}{:else}{@render iZoom()}{/if}</button>
          {#if s?.cc}
            <button class="icon-btn" title="Split right (tmux)" onclick={() => ccSplit(node.leaf, "h")}>{@render iSplitH()}</button>
            <button class="icon-btn" title="Split down (tmux)" onclick={() => ccSplit(node.leaf, "v")}>{@render iSplitV()}</button>
            <button class="icon-btn" title="Close pane (tmux)" onclick={() => ccKill(node.leaf)}>{@render iClose()}</button>
          {:else}
            <button class="icon-btn" title="Split right (⌘D)" onclick={() => openPicker({ tabId: tab.id, sid: node.leaf, dir: "h" })}>{@render iSplitH()}</button>
            <button class="icon-btn" title="Split down (⇧⌘D)" onclick={() => openPicker({ tabId: tab.id, sid: node.leaf, dir: "v" })}>{@render iSplitV()}</button>
            <button class="icon-btn" title="Close (⌘W)" onclick={() => closePane(node.leaf)}>{@render iClose()}</button>
          {/if}
        </span>
      </div>
      <div class="pane-term" use:mountTerm={node.leaf} oncontextmenu={(e) => pasteInto(node.leaf, e)}>
        {#if searchState?.sid === node.leaf}
          <div class="search-bar" transition:fly={{ y: -4, duration: 120 }}>
            <input
              bind:this={searchInput}
              bind:value={searchState.query}
              placeholder="Search…"
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
              <span>Connecting to {s?.remote.name}…</span>
              {#if s && !isLocal(s.remote.id)}<span class="veil-target">{s.remote.user}@{s.remote.host}:{s.remote.port}</span>{/if}
            {:else if st.status === "error"}
              <span class="veil-warn">{@render iWarn()}</span>
              <span class="veil-msg">{st.error}</span>
              {#if s && !isLocal(s.remote.id)}<span class="veil-target">{s.remote.user}@{s.remote.host}:{s.remote.port}{s.remote.auth ? ` · ${s.remote.auth}` : ""}</span>{/if}
              <div class="veil-actions">
                <button class="btn" onclick={() => connectSession(node.leaf)}>Retry</button>
                <button class="btn ghost" onclick={() => removeSession(node.leaf)}>Close</button>
              </div>
            {:else}
              {#if s && !isLocal(s.remote.id)}
                <span class="veil-spin">{@render iSpinner(18)}</span>
                <span class="veil-msg">Connection lost — reconnecting automatically…{s.tmux ? " (the tmux session keeps running on the server)" : ""}</span>
              {:else}
                <span class="veil-msg">Session ended</span>
              {/if}
              <div class="veil-actions">
                <button class="btn" onclick={() => connectSession(node.leaf)}>Reconnect</button>
                <button class="btn ghost" onclick={() => removeSession(node.leaf)}>Close</button>
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

{#snippet emptyState(icon: Snippet, title: string, hint: string, action: { label: string; run: () => void } | null)}
  <div class="empty">
    <span class="empty-icon">{@render icon()}</span>
    <p class="empty-title">{title}</p>
    {#if hint}<p class="empty-hint">{hint}</p>{/if}
    {#if action}<button class="btn ghost empty-btn" onclick={action.run}>{action.label}</button>{/if}
  </div>
{/snippet}

<main class:no-sidebar={!settings.sidebar} style="--term-bg: {activeTheme().background}">
  {#if settings.sidebar}
    <aside class="sidebar">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="sb-traffic" data-tauri-drag-region>
        <button class="icon-btn sb-toggle" title="Hide sidebar (⌘B)" onclick={() => { settings.sidebar = false; save(); }}>{@render iSidebar()}</button>
      </div>
      <nav class="sb-scroll">
        {#if loaded}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="sb-section" class:drop={dropTarget === "standalone"} {...dropzone("standalone")}>
            {@render sbSection("Terminals", () => openPicker())}
            {#each tabs.filter((t) => t.root && (!settings.projects || !t.projectId)) as t (t.id)}
              {#each leaves(t.root) as sid (sid)}
                {@render sessRow(t, sid, false)}
              {/each}
            {:else}
              {#if dragSid && settings.projects}
                <p class="sb-empty">Drop here to remove from project</p>
              {:else if !tabs.some((t) => t.root)}
                <!-- rien nulle part (1er lancement) : on guide ; sinon la section reste compacte -->
                {@render emptyState(iTerminal, "No terminals", "Open a local or SSH session to get started.", { label: "New terminal", run: () => openPicker() })}
              {/if}
            {/each}
          </div>

          {#if settings.projects}
          <div class="sb-section">
            {@render sbSection("Projects", null)}
            {#each projects as p (p.id)}
              {@const ptabs = projectTabs(p.id)}
              {@const open = ptabs.length > 0}
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
              <div
                class="row"
                class:drop={dropTarget === p.id}
                {...dropzone(p.id)}
                title={p.name}
                onclick={() => (open ? (activeTabId = ptabs[0].id) : openProject(p))}>
                <button
                  class="icon-btn chev"
                  class:open={isExpanded(p.id)}
                  onclick={(e) => { e.stopPropagation(); expanded[p.id] = !isExpanded(p.id); }}>{@render iChevronR()}</button>
                {#if p.emoji}<Tgs name={p.emoji} size={18} play={settings.emojiAnim} />{/if}
                <span class="row-label strong">{p.name}</span>
                <!-- l'attention prime sur « ouvert » : c'est le seul des deux qui demande un geste -->
                {#if projectAttention(p.id)}<span class="dot attention" title="A terminal needs you"></span>
                {:else if open}<span class="dot live" title="Project open"></span>{/if}
                <button
                  class="icon-btn row-plus"
                  title="Add a terminal to the project"
                  onclick={(e) => { e.stopPropagation(); openPicker({ projectId: p.id }); }}>{@render iPlus()}</button>
                {@render rowActions(p.id, () => (modal = { type: "project", data: $state.snapshot(p) }), () => deleteProject(p))}
              </div>
              {#if isExpanded(p.id)}
                {#if open}
                  {#each ptabs as ptab (ptab.id)}
                    {#each leaves(ptab.root) as sid (sid)}
                      {@render sessRow(ptab, sid, true)}
                    {/each}
                  {/each}
                {:else}
                  {#each projectViews(p).flatMap(projLeaves) as leaf, n (n)}
                    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                    <div class="row sub dim" title="Reopen this project" onclick={() => openProject(p)}>
                      <span class="row-icon">{@render iRefresh()}</span>
                      <span class="row-label">{projRemote(leaf.remoteId)?.name ?? "?"}</span>
                    </div>
                  {/each}
                {/if}
              {/if}
            {:else}
              {@render emptyState(iBookmark, "No projects", "Open some terminals, then save the layout with the bookmark button.", null)}
            {/each}
          </div>
          {/if}
        {/if}
      </nav>
      <div class="sb-foot">
        <button class="sb-settings" onclick={() => (modal = { type: "connections" })}>
          {@render iTerminal()}<span>Connections</span>
        </button>
        <button class="sb-settings" onclick={() => (modal = { type: "settings" })}>
          {@render iGear()}<span>Settings</span>
        </button>
        <div class="sb-brand">
          <span class="sb-logo">{@render iLogo()}</span>
          <span class="sb-wordmark">arabel</span>
          <span class="sb-ver">v{appVersion}</span>
        </div>
      </div>
    </aside>
  {/if}

  <section class="content">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="titlebar" data-tauri-drag-region>
      {#if !settings.sidebar}
        <div class="traffic-pad"></div>
        <button class="icon-btn" title="Show sidebar (⌘B)" onclick={() => { settings.sidebar = true; save(); }}>{@render iSidebar()}</button>
      {/if}
      <span class="tb-title">{activeTab && activeTab.root ? tabTitle(activeTab) : "arabel"}</span>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="tb-space" data-tauri-drag-region></div>
      {#if activeMetrics}
        {@render meter("cpu", Math.min(1, activeMetrics.load / activeMetrics.cpus), `${activeMetrics.load.toFixed(1)} / ${activeMetrics.cpus}`)}
        {@render meter("ram", activeMetrics.memUsed / (activeMetrics.memTotal || 1), `${gb(activeMetrics.memUsed)} / ${gb(activeMetrics.memTotal)} GB`)}
        {@render meter("dsk", activeMetrics.diskUsed / (activeMetrics.diskTotal || 1), `${gb(activeMetrics.diskUsed)} / ${gb(activeMetrics.diskTotal)} GB`)}
      {/if}
      {#if activeSshRemote()}
        <button class="icon-btn" class:active-btn={forwardsOpen} title="Port forwards" onclick={() => (forwardsOpen = !forwardsOpen)}>{@render iGlobe()}</button>
        <button class="icon-btn" class:active-btn={files.open} title="Server files (SFTP)" onclick={toggleFiles}>{@render iFolder()}</button>
        <button class="icon-btn" class:active-btn={git.open} title="Source control (git)" onclick={toggleGit}>{@render iBranch()}</button>
      {/if}
      {#if forwards.length}
        <span class="fwd-count" title="Active tunnels">{forwards.length}</span>
      {/if}
      {#if activeTab?.root && settings.projects}
        <button
          class="icon-btn"
          title="Save as project"
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
            <p class="hint">Local &amp; SSH terminal to drive your AI agents</p>
            {#if loaded}
              <div class="welcome-list">
                {#each locals as l (l.id)}
                  {@render remoteRow(l, (x) => openInTab(t, x), false)}
                {/each}
                {#each remotes.slice(0, 4) as r (r.id)}
                  {@render remoteRow(r, (x) => openInTab(t, x), false)}
                {/each}
              </div>
              {#if !remotes.length}
                <button class="btn" onclick={() => (identities.length ? editRemote() : editIdentity())}>
                  {identities.length ? "Add an SSH remote" : "Add an SSH identity"}
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
            <button class="icon-btn" title="Refresh" onclick={() => filesLoad(files.path)}>{@render iRefresh()}</button>
            <button class="icon-btn" title="Close" onclick={() => (files.open = false)}>{@render iClose()}</button>
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
                  <button class="icon-btn" title="Download to ~/Downloads" onclick={(e) => { e.stopPropagation(); fileDownload(entry); }}>{@render iDownload()}</button>
                </span>
              {/if}
            </div>
          {:else}
            <p class="sb-empty">Empty folder</p>
          {/each}
        </div>
        <div class="files-hint">Drop files here to upload them</div>
        {#if files.busy}
          <div class="files-veil"><span class="veil-spin">{@render iSpinner(16)}</span></div>
        {/if}
      </aside>
    {/if}

    {#if git.open}
      <aside class="files git-panel">
        <div class="files-head">
          <span class="git-branch-wrap">
            <button class="git-branch" title="Current branch" onclick={() => (git.showBranches = !git.showBranches)}>
              {@render iBranch()}<span class="git-branch-name">{git.branch || "—"}</span>
              {#if git.branches.length > 1}<span class="git-count">{git.branches.length}</span>{/if}
            </button>
            {#if git.showBranches}
              <div class="git-branches">
                {#each git.branches as b (b)}
                  <button class="git-branch-item" class:current={b === git.branch} onclick={() => gitCheckout(b)}>{b}</button>
                {/each}
              </div>
            {/if}
          </span>
          <span class="files-btns">
            <button class="icon-btn" class:fetching={git.fetching} title="Refresh (fetch)" onclick={() => gitRefresh(true)}>{@render iRefresh()}</button>
            <button class="icon-btn" title="Close" onclick={() => (git.open = false)}>{@render iClose()}</button>
          </span>
        </div>

        {#if !git.isRepo}
          <p class="sb-empty">Not a git repository<br /><span class="git-dim">{git.root}</span></p>
        {:else}
          <button class="git-graph-row" onclick={toggleGitLog}>
            <span class="git-graph-chev" class:open={git.showLog}>{@render iChevronR()}</span>
            <span>Commit Graph</span>
          </button>
          {#if git.showLog}
            <div class="git-log">
              {#each git.log as l (l)}<div class="git-log-line">{l}</div>{:else}<div class="git-dim git-log-line">No commits</div>{/each}
            </div>
          {/if}

          <div class="git-commit-box">
            <textarea
              class="git-msg"
              placeholder="Commit message"
              bind:value={git.msg}
              onkeydown={(e) => { if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); gitCommit(); } }}></textarea>
            <div class="git-commit-hint">{KMOD === "Meta" ? "⌘" : "Ctrl"}↵ to commit</div>
          </div>

          <div class="git-status-line">
            <span class="git-dot" class:staged={gitStagedCount > 0}></span>
            <span>{gitStagedCount ? `${gitStagedCount} staged` : "Nothing staged"}</span>
            <span class="git-upstream" title="Upstream">
              {git.upstream || "no upstream"}{#if git.ahead || git.behind}&nbsp;↑{git.ahead} ↓{git.behind}{/if}
            </span>
          </div>

          <div class="git-actions">
            <button class="btn git-commit-btn" disabled={!git.msg.trim() || !gitStagedCount} onclick={gitCommit}>Commit</button>
            <button class="btn git-push-btn" disabled={!git.branch} onclick={gitPush}>Push{#if git.ahead}&nbsp;↑{git.ahead}{/if}</button>
            <!-- pas d'upstream = rien à tirer (et `pull` échouerait faute de savoir d'où) -->
            <button class="btn git-pull-btn" disabled={!git.upstream} onclick={gitPull}>Pull{#if git.behind}&nbsp;↓{git.behind}{/if}</button>
          </div>

          <div class="git-changes-head">
            <span>CHANGES{#if git.entries.length}&nbsp;<span class="git-count">{git.entries.length}</span>{/if}</span>
            {#if git.entries.length}
              <button class="git-all" onclick={gitStageAll}>{git.entries.every((e) => e.staged) ? "Unstage all" : "Stage all"}</button>
            {/if}
          </div>

          <div class="files-list">
            {#each git.entries as entry (entry.path)}
              <div class="row git-row">
                <button class="git-file" title="Show diff" onclick={() => gitShowDiff(entry)}>
                  <span class="row-icon file-icon">{@render iFile()}</span>
                  <span class="git-name" class:conflict={entry.conflict}>{entry.name}</span>
                  {#if entry.sub}<span class="git-sub">{entry.sub}</span>{/if}
                </button>
                {#if entry.conflict}<span class="git-confl" title="Merge conflict">!</span>{/if}
                <button
                  class="git-check"
                  class:on={entry.staged}
                  title={entry.conflict ? "Mark resolved" : entry.staged ? "Unstage" : "Stage"}
                  onclick={() => gitToggleEntry(entry)}>{#if entry.staged}{@render iCheck()}{/if}</button>
              </div>
            {:else}
              <p class="sb-empty">Working tree clean</p>
            {/each}
          </div>
        {/if}
        {#if git.busy}
          <div class="files-veil"><span class="veil-spin">{@render iSpinner(16)}</span></div>
        {/if}
      </aside>
    {/if}

    {#if forwardsOpen}
      <aside class="files fwd-panel">
        <div class="files-head">
          <span class="files-title">Port forwards</span>
          <button class="icon-btn" title="Close" onclick={() => (forwardsOpen = false)}>{@render iClose()}</button>
        </div>
        <form class="fwd-add" onsubmit={(e) => { e.preventDefault(); addForward(); }}>
          <input placeholder="remote port (e.g. 3000)" bind:value={newFwd.remotePort} />
          <button type="submit" class="btn fwd-go" disabled={!newFwd.remotePort} title="Open tunnel">{@render iPlus()}</button>
        </form>
        <div class="files-list">
          {#each forwards as f (f.id)}
            <div class="fwd-row">
              <span class="fwd-label" title="localhost:{f.localPort} → {f.remoteName}:{f.remotePort}">
                <b>:{f.localPort}</b> <span class="fwd-arrow">→</span> {f.remoteName}:{f.remotePort}
              </span>
              <span class="fwd-btns">
                <button class="icon-btn" title="Built-in preview" onclick={() => openPreview(f)}>{@render iGlobe()}</button>
                <button class="icon-btn" title="Open in browser" onclick={() => openInBrowser(`http://localhost:${f.localPort}`)}>{@render iExternal()}</button>
                <button class="icon-btn" title="Stop tunnel" onclick={() => stopForward(f)}>{@render iClose()}</button>
              </span>
            </div>
          {:else}
            <p class="sb-empty">No tunnels — enter a remote port ↑</p>
          {/each}
        </div>
        <div class="files-hint">The remote port becomes reachable on localhost</div>
      </aside>
    {/if}

    {#if preview}
      <aside class="preview">
        <div class="preview-bar">
          <button class="icon-btn" title="Reload" onclick={reloadPreview}>{@render iRefresh()}</button>
          <span class="preview-url">{preview.url}</span>
          <button class="icon-btn" title="Open in browser" onclick={() => openInBrowser(preview!.url)}>{@render iExternal()}</button>
          <button class="icon-btn" title="Close preview" onclick={() => (preview = null)}>{@render iClose()}</button>
        </div>
        <iframe class="preview-frame" title="Preview" src={preview.url} bind:this={previewFrame}></iframe>
      </aside>
    {/if}
    </div>
  </section>
</main>

<!-- ─── toasts ─────────────────────────────────────────────────────────── -->
{#if toasts.length}
  <div class="toasts">
    {#each toasts as t (t.id)}
      <div class="toast {t.kind}" transition:fly={{ y: 8, duration: 150 }} animate:flip={{ duration: 150 }}>{t.msg}</div>
    {/each}
  </div>
{/if}

<!-- ─── attentions ─────────────────────────────────────────────────────── -->
{#if attentions.length}
  <div class="attentions">
    {#each attentions.slice(-4) as a (a.id)}
      <div class="att-card" class:stop={a.kind === "stop"} class:hasopts={!!a.options?.length} transition:fly={{ y: 8, duration: 150 }} animate:flip={{ duration: 150 }}>
        <div class="att-head">
          <span class="att-icon {a.kind}">{#if a.kind === "stop"}{@render iCheck()}{:else}{@render iAlert()}{/if}</span>
          <button class="att-msg" onclick={() => { gotoAttention(a); dismissAttention(a); }} title="Go to pane">
            {a.message}
          </button>
          {#if a.kind === "notif" && !a.options?.length}
            <button class="att-btn yes" title="Allow (sends 1)" onclick={() => answerAttention(a, "1")}>✓</button>
            <button class="att-btn no" title="Deny (sends Esc)" onclick={() => answerAttention(a, "\x1b")}>✗</button>
          {/if}
          <button class="icon-btn" onclick={() => dismissAttention(a)}>{@render iClose()}</button>
        </div>
        {#if a.options?.length}{@render choiceBtns(a)}{/if}
      </div>
    {/each}
    {#if attentions.length > 4}<div class="att-more">+{attentions.length - 4} more</div>{/if}
  </div>
{/if}

<!-- ─── modales ────────────────────────────────────────────────────────── -->
{#if modal}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div class="overlay" onclick={() => (modal = null)} transition:fade={{ duration: 120 }}>
    <div class="sheet" class:wide={modal.type === "settings"} class:diff={modal.type === "diff"} onclick={(e) => e.stopPropagation()} transition:scale={{ start: 0.96, opacity: 0, duration: 160 }}>
      {#if modal.type === "remote"}
        <h2>{remotes.some((r) => r.id === (modal as any).data.id) ? "Edit remote" : "New remote"}</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveRemote(); }}>
          <label>{@render field("Name")}<input bind:value={modal.data.name} placeholder="my-vps (auto if empty)" use:autofocus /></label>
          <div class="f-pair">
            <label class="grow">{@render field("Host")}<input bind:value={modal.data.host} placeholder="vps.example.com" required /></label>
            <label class="f-port">{@render field("Port")}<input type="number" bind:value={modal.data.port} /></label>
          </div>
          <label>{@render field("User")}<input bind:value={modal.data.user} required /></label>
          <label>{@render field("Authentication")}
            <select bind:value={modal.data.auth}>
              <option value="key">Private key</option>
              <option value="password">Password</option>
              <option value="agent">ssh-agent</option>
            </select>
          </label>
          {#if (modal.data.auth ?? "key") === "key"}
            <label>{@render field("Identity")}
              <select bind:value={modal.data.identityId} required>
                {#each identities as i (i.id)}<option value={i.id}>{i.name}</option>{/each}
              </select>
            </label>
            {#if !identities.length}<p class="f-hint">No identity — add one, or choose ssh-agent / password.</p>{/if}
          {:else if modal.data.auth === "password"}
            <label>{@render field("Password")}
              <input type="password" bind:value={modal.password} placeholder={remotes.some((x) => x.id === (modal as any).data.id) ? "(unchanged)" : ""} />
            </label>
            <p class="f-hint">Stored encrypted in {secretStore}, on this machine only.</p>
          {:else}
            <p class="f-hint">Uses the keys loaded in your ssh-agent (<code>ssh-add</code>).</p>
          {/if}
          <label>{@render field("Working directory (optional)")}
            <input bind:value={modal.data.dir} placeholder="~/code/my-project — otherwise home directory" />
          </label>
          <div class="mgr-head"><span>Session</span></div>
          <div class="group">
            <label class="f-check">
              <input type="checkbox" bind:checked={modal.data.claude} />
              <span>Claude Code — sync config and <b>track the agent</b> (the <code>claude</code> you launch is tracked)</span>
            </label>
            {#if modal.data.claude}
              <label class="f-check sub-check">
                <input type="checkbox" checked={!!modal.data.autoLaunch} onchange={(e) => { if (modal?.type === "remote") modal.data.autoLaunch = e.currentTarget.checked; }} />
                <span>Launch <code>claude</code> automatically on connect (otherwise: shell, launch it whenever you want)</span>
              </label>
            {/if}
            <label class="f-check">
              <input type="checkbox" checked={modal.data.tmux !== false} onchange={(e) => { if (modal?.type === "remote") modal.data.tmux = e.currentTarget.checked; }} />
              <span>tmux — persistent sessions (survive disconnects)</span>
            </label>
            {#if moshOk && modal.data.auth !== "password" && !modal.data.sysSsh}
              <label class="f-check">
                <input type="checkbox" checked={!!modal.data.mosh} onchange={(e) => { if (modal?.type === "remote") modal.data.mosh = e.currentTarget.checked; }} />
                <span>mosh — near-instant echo &amp; resume after drops (UDP; native tmux splits, not control mode)</span>
              </label>
            {/if}
            <label class="f-check">
              <input type="checkbox" checked={!!modal.data.sysSsh} onchange={(e) => { if (modal?.type === "remote") modal.data.sysSsh = e.currentTarget.checked; }} />
              <span>System ssh — use the OpenSSH binary (any key format, ssh-agent, ~/.ssh/config). No SFTP / port-forwarding / metrics.</span>
            </label>
          </div>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Cancel</button>
            <button type="submit" class="btn">Save</button>
          </div>
        </form>
      {:else if modal.type === "identity"}
        <h2>{identities.some((i) => i.id === (modal as any).data.id) ? "Edit identity" : "New identity"}</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveIdentity(); }}>
          <label>{@render field("Name")}<input bind:value={modal.data.name} placeholder="(auto if empty)" use:autofocus /></label>
          <label>{@render field("Private key")}<input bind:value={modal.data.keyPath} required /></label>
          <label>{@render field("Passphrase")}
            <input type="password" bind:value={modal.passphrase} placeholder={modal.data.hasPassphrase ? "(unchanged)" : "(none)"} />
          </label>
          <p class="f-hint">Stored in {secretStore}, never on disk.</p>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Cancel</button>
            <button type="submit" class="btn">Save</button>
          </div>
        </form>
      {:else if modal.type === "project"}
        <h2>Edit project</h2>
        <form onsubmit={(e) => { e.preventDefault(); saveProjectEdit(); }}>
          <label>{@render field("Name")}<input bind:value={modal.data.name} required use:autofocus /></label>
          {@render field("Emoji")}
          <div class="emoji-grid">
            <button
              type="button"
              class="emoji-cell"
              class:sel={!modal.data.emoji}
              title="No emoji"
              onclick={() => modal && modal.type === "project" && (modal.data.emoji = undefined)}>{@render iClose()}</button>
            {#each EMOJI as name (name)}
              <button
                type="button"
                class="emoji-cell"
                class:sel={modal.data.emoji === name}
                onmouseenter={() => (hoverEmoji = name)}
                onmouseleave={() => (hoverEmoji = null)}
                onclick={() => modal && modal.type === "project" && (modal.data.emoji = name)}>
                <Tgs {name} size={26} play={hoverEmoji === name || modal.data.emoji === name} />
              </button>
            {/each}
          </div>
          <p class="f-hint">
            “NewsEmoji” pack by its authors on Telegram —
            <button type="button" class="link-btn" onclick={() => openInBrowser("https://t.me/addemoji/NewsEmoji")}>t.me/addemoji/NewsEmoji</button>
          </p>
          <label>
            {@render field("Startup commands")}
            <textarea class="cmd-area" bind:value={modal.data.cmd} rows="3" placeholder="(none)"></textarea>
          </label>
          <p class="f-hint">One command per line, replayed in every terminal opened in this project.</p>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Cancel</button>
            <button type="submit" class="btn">Save</button>
          </div>
        </form>
      {:else if modal.type === "saveProject"}
        <h2>Save project</h2>
        <form onsubmit={(e) => { e.preventDefault(); confirmSaveProject(); }}>
          <label>{@render field("Project name")}<input bind:value={modal.name} required use:autofocus /></label>
          <div class="sheet-actions">
            <button type="button" class="btn ghost" onclick={() => (modal = null)}>Cancel</button>
            <button type="submit" class="btn">Save</button>
          </div>
        </form>
      {:else if modal.type === "picker"}
        <h2>{modal.dir ? "Open in the new pane" : "New connection"}</h2>
        {#if remotes.length > 6}
          <input class="split-filter" bind:value={modal.filter} placeholder="Filter…" use:autofocus />
        {/if}
        <div class="split-list">
          <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
          <div class="row" onclick={doPickBrowser} title="Open an in-app web browser pane">
            <span class="row-icon">{@render iGlobe()}</span>
            <span class="row-label">Web browser</span>
            <span class="row-meta">preview localhost / a site</span>
          </div>
          {#each locals.filter((l) => l.name.toLowerCase().includes((modal as any).filter.toLowerCase())) as l (l.id)}
            {@render remoteRow(l, doPick, false)}
          {/each}
          {#each remotes.filter((r) => r.name.toLowerCase().includes((modal as any).filter.toLowerCase())) as r (r.id)}
            {@render remoteRow(r, doPick, false)}
          {/each}
        </div>
        {#if !modal.dir && !modal.projectId}
          <label class="f-check cc-check">
            <input type="checkbox" bind:checked={modal.cc} />
            <span><b>Native tmux</b> mode — panes mirror tmux splits (experimental)</span>
          </label>
        {/if}
        <div class="sheet-actions spread">
          <button class="btn ghost" onclick={() => (modal = { type: "connections" })}>Manage connections…</button>
          <button class="btn ghost" onclick={() => (modal = null)}>Cancel</button>
        </div>
      {:else if modal.type === "connections"}
        <h2>Connections</h2>
        <div class="mgr-head">
          <span>SSH remotes</span>
          <span class="mgr-btns">
            <button class="btn ghost sm" title="Import from ~/.ssh/config and VS Code" onclick={openSshImport}>{@render iDownload()}Import</button>
            <button class="btn ghost sm" onclick={() => editRemote(undefined, true)}>{@render iPlus()}New remote</button>
          </span>
        </div>
        <div class="split-list carded">
          {#each remotes as r (r.id)}
            {@render remoteRow(r, (x) => editRemote(x, true), true)}
          {:else}
            {#if identities.length}
              {@render emptyState(iTerminal, "No remotes", "Add an SSH remote to connect to a server.", { label: "Add a remote", run: () => editRemote(undefined, true) })}
            {:else}
              {@render emptyState(iKey, "Add an identity first", "You need an SSH key before you can add a remote.", { label: "Add an identity", run: () => editIdentity(undefined, true) })}
            {/if}
          {/each}
        </div>
        <div class="mgr-head">
          <span>SSH identities</span>
          <button class="btn ghost sm" onclick={() => editIdentity(undefined, true)}>{@render iPlus()}New identity</button>
        </div>
        <div class="split-list carded">
          {#each identities as i (i.id)}
            {@const uses = remotes.filter((r) => (r.auth ?? "key") === "key" && r.identityId === i.id).length}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row mgr" onclick={() => editIdentity(i, true)} title={i.keyPath}>
              <span class="row-icon">{@render iKey()}</span>
              <span class="row-main">
                <span class="row-label">{i.name}{#if i.hasPassphrase}<span class="row-lock" title="Passphrase protected"> 🔒</span>{/if}</span>
                <span class="row-sub">{i.keyPath}</span>
              </span>
              <span class="row-meta">{uses ? `used by ${uses}` : "unused"}</span>
              {@render rowActions(i.id, () => editIdentity(i, true), () => deleteIdentity(i))}
            </div>
          {:else}
            {@render emptyState(iKey, "No identities", "Add an SSH key to authenticate your connections.", { label: "Add an identity", run: () => editIdentity(undefined, true) })}
          {/each}
        </div>
        <div class="mgr-head"><span>Backup &amp; sync</span></div>
        <div class="sync-row">
          <button type="button" class="btn ghost" onclick={exportConfig}>{@render iCopy()}Copy config</button>
          <button type="button" class="btn ghost" onclick={() => (modal = { type: "configImport", text: "" })}>{@render iClipboard()}Paste config…</button>
        </div>
        <p class="f-hint">Moves your remotes, projects &amp; settings to another PC — never passwords or keys.</p>
        <div class="sheet-actions">
          <button class="btn" onclick={() => (modal = null)}>Close</button>
        </div>
      {:else if modal.type === "diff"}
        <h2 class="diff-head">
          <span class="diff-path">{modal.path}</span>
          <span class="diff-mode" class:conflict={modal.mode === "conflict"}>{modal.mode}</span>
        </h2>
        <div class="diff-body">
          {#if modal.loading}
            <p class="sb-empty">Loading…</p>
          {:else if !modal.text.trim()}
            <p class="sb-empty">No changes</p>
          {:else}
            {#each parseDiff(modal.text) as l}<div class="dl {l.t}">{l.s || " "}</div>{/each}
          {/if}
        </div>
        <div class="sheet-actions">
          <button class="btn" onclick={() => (modal = null)}>Close</button>
        </div>
      {:else if modal.type === "configImport"}
        <h2>Import config</h2>
        <p class="f-hint">Paste the config you copied from your other PC, then Import. Existing entries with the same id are updated; the rest is added.</p>
        <textarea class="config-paste" bind:value={modal.text} spellcheck="false" placeholder={'{ "v": 1, "remotes": [ … ] }'} use:autofocus></textarea>
        <div class="sheet-actions">
          <button type="button" class="btn ghost" onclick={() => (modal = { type: "connections" })}>Cancel</button>
          <button type="button" class="btn" onclick={() => modal?.type === "configImport" && importConfig(modal.text)}>Import</button>
        </div>
      {:else if modal.type === "sshImport"}
        <h2>Import SSH / VS Code remotes</h2>
        <p class="f-hint">From <code>~/.ssh/config</code> (includes resolved) and VS Code's Remote-SSH config.</p>
        <div class="split-list carded">
          {#each modal.hosts as h (h.host)}
            <div class="row import-row">
              <span class="row-icon">{@render iTerminal()}</span>
              <span class="row-label">{h.host}<span class="row-meta"> · {h.user || "?"}@{h.hostName}:{h.port}</span></span>
              <button class="btn ghost import-btn" onclick={() => importHost(h)}>Import</button>
            </div>
          {:else}
            <p class="sb-empty">Everything imported ✓</p>
          {/each}
        </div>
        <div class="sheet-actions">
          <button class="btn" onclick={() => (modal = { type: "connections" })}>Close</button>
        </div>
      {:else if modal.type === "palette"}
        {@const items = filteredPalette(modal.filter)}
        <input
          class="palette-input"
          bind:value={modal.filter}
          placeholder="Go to… or run a command"
          use:autofocus
          oninput={() => (paletteSel = 0)}
          onkeydown={(e) => paletteNav(e, (modal as { filter: string }).filter)} />
        <div class="split-list palette-list">
          {#each items as it, i (i)}
            <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
            <div class="row" class:sel={i === paletteSel} onmousemove={() => (paletteSel = i)} onclick={() => runPalette(it)}>
              <span class="row-icon">
                {#if it.icon === "local"}{@render iLaptop()}{:else if it.icon === "project"}{@render iBookmark()}{:else if it.icon === "action"}{@render iBolt()}{:else}{@render iTerminal()}{/if}
              </span>
              <span class="row-label">{it.label}</span>
              {#if it.kbd && it.kbd !== "—"}<span class="kbd">{it.kbd}</span>{:else}<span class="row-meta">{it.sub}</span>{/if}
            </div>
          {:else}
            <p class="sb-empty">No results</p>
          {/each}
        </div>
      {:else if modal.type === "settings"}
        <h2>Settings</h2>
        <div class="seg">
          <button type="button" class:on={settingsTab === "appearance"} onclick={() => (settingsTab = "appearance")}>Appearance</button>
          <button type="button" class:on={settingsTab === "terminal"} onclick={() => (settingsTab = "terminal")}>Terminal</button>
          <button type="button" class:on={settingsTab === "shortcuts"} onclick={() => (settingsTab = "shortcuts")}>Shortcuts</button>
        </div>

        {#if settingsTab === "appearance"}
          <form onsubmit={(e) => { e.preventDefault(); modal = null; }}>
            <div class="f-pair">
              <label class="grow">{@render field("Terminal font")}
                <div class="font-field">
                  <input
                    value={fontOpen ? fontQuery : settings.fontFamily}
                    onfocus={() => { fontQuery = ""; fontOpen = true; }}
                    oninput={(e) => { fontQuery = e.currentTarget.value; fontOpen = true; }}
                    onkeydown={(e) => { if (e.key === "Enter") { e.preventDefault(); if (fontQuery.trim()) pickFont(fontQuery.trim()); else fontOpen = false; } else if (e.key === "Escape") { fontOpen = false; e.currentTarget.blur(); } }}
                    onblur={() => setTimeout(() => (fontOpen = false), 160)}
                    placeholder="Search for a font…" />
                  <button type="button" class="icon-btn font-reset" title="Default font" onmousedown={(e) => { e.preventDefault(); pickFont(DEFAULT_FONT); }}>{@render iRefresh()}</button>
                </div>
              </label>
              <label class="f-port">{@render field("Size")}<input type="number" min="9" max="32" bind:value={settings.fontSize} onchange={applySettings} /></label>
            </div>
            <button type="button" class="link-btn" onclick={importVscodeFont}>{@render iDownload()}<span>Import VS Code's font</span></button>
            {#if fontOpen}
              {@const q = fontQuery.toLowerCase().trim()}
              {@const matches = fontList.filter((f) => f.toLowerCase().includes(q)).slice(0, 80)}
              <div class="font-list">
                {#each matches as f (f)}
                  <button type="button" class="font-opt" class:cur={f === settings.fontFamily} style="font-family:'{f.replace(/'/g, '')}', monospace" onmousedown={(e) => { e.preventDefault(); pickFont(f); }}>{f}</button>
                {:else}
                  <div class="font-empty">{fontList.length ? "No font — type a name then Enter" : "Loading…"}</div>
                {/each}
              </div>
            {/if}
            <label>{@render field("Terminal theme")}
              <select bind:value={settings.theme} onchange={applySettings}>
                {#each Object.keys(THEMES) as th}<option value={th}>{th}</option>{/each}
                <option value="Custom">Custom</option>
              </select>
            </label>
            {#if settings.theme === "Custom"}
              <div class="theme-editor">
                <div class="color-row">
                  {@render colorField("Background", "background")}
                  {@render colorField("Text", "foreground")}
                  {@render colorField("Cursor", "cursor")}
                  {@render colorField("Selection", "selectionBackground")}
                </div>
                <div class="ansi-line">
                  <span class="f-label">ANSI palette</span>
                  <div class="ansi-grid">
                    {#each ANSI_KEYS as k (k)}
                      <input type="color" class="ansi-dot" title={k} value={(settings.customTheme[k] as string) ?? '#000000'} oninput={(e) => setCustom(k, e.currentTarget.value)} />
                    {/each}
                  </div>
                </div>
                <label class="seed">
                  <span class="f-label">Start from a preset</span>
                  <select onchange={(e) => { if (e.currentTarget.value) { settings.customTheme = { ...THEMES[e.currentTarget.value] }; applySettings(); } e.currentTarget.selectedIndex = 0; }}>
                    <option value="">Choose…</option>
                    {#each Object.keys(THEMES) as th}<option value={th}>{th}</option>{/each}
                  </select>
                </label>
              </div>
            {/if}
            <div class="mgr-head"><span>Projects</span></div>
            <div class="group">
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.projects} onchange={() => save()} />
                <span>Projects (saved terminal layouts) — off keeps terminals only</span>
              </label>
              {#if settings.projects}
                <label class="f-check">
                  <input type="checkbox" bind:checked={settings.emojiAnim} onchange={() => save()} />
                  <span>Animate project emojis continuously</span>
                </label>
              {/if}
            </div>
            <div class="sheet-actions"><button type="submit" class="btn">Close</button></div>
          </form>
        {:else if settingsTab === "terminal"}
          <form onsubmit={(e) => { e.preventDefault(); modal = null; }}>
            <div class="f-pair">
              <label class="grow">{@render field("Cursor style")}
                <select bind:value={settings.cursorStyle} onchange={applySettings}>
                  <option value="bar">Bar</option>
                  <option value="block">Block</option>
                  <option value="underline">Underline</option>
                </select>
              </label>
              <label class="f-port">{@render field("Line height")}<input type="number" min="1" max="2" step="0.05" bind:value={settings.lineHeight} onchange={applySettings} /></label>
            </div>
            <label>{@render field("Scrollback (lines kept in history)")}<input type="number" min="0" max="100000" step="1000" bind:value={settings.scrollback} onchange={applySettings} /></label>
            <div class="mgr-head"><span>Behavior</span></div>
            <div class="group">
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.cursorBlink} onchange={applySettings} />
                <span>Blinking cursor</span>
              </label>
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.copyOnSelect} onchange={applySettings} />
                <span>Copy selection automatically</span>
              </label>
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.tmuxStatus} onchange={() => save()} />
                <span>Show tmux status bar (takes effect on reconnect)</span>
              </label>
            </div>
            <div class="mgr-head"><span>Agent events</span></div>
            <div class="group">
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.sounds} onchange={() => save()} />
                <span>Play a sound on agent events (waiting / finished / denied)</span>
              </label>
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.notifications} onchange={() => save()} />
                <span>Show a system notification on agent events</span>
              </label>
              <p class="f-hint">Both stay quiet while you are looking at the pane that raised the event.</p>
            </div>
            <div class="mgr-head"><span>Agent teams</span></div>
            <div class="group">
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.agentTeams} onchange={() => save()} />
                <span>Let Claude spawn a team of agents (experimental)</span>
              </label>
              <label class="f-check">
                <input type="checkbox" bind:checked={settings.agentTeamPanes} disabled={!settings.agentTeams} onchange={() => save()} />
                <span>Give each teammate its own pane</span>
              </label>
              <p class="f-hint">
                Applied on the next sync to a remote. A team costs far more tokens than a single agent. Own panes need a
                native tmux tab, where each teammate is mirrored as a real pane; unchecked, they all share the lead's pane.
              </p>
            </div>
            <div class="sheet-actions"><button type="submit" class="btn">Close</button></div>
          </form>
        {:else}
          <p class="f-hint">Click a shortcut, then press the new key combination. Esc cancels, and a combo already in use is moved to this action.</p>
          <div class="keys-list">
            {#each KEYBINDINGS as b (b.id)}
              <div class="key-row">
                <span class="key-label">{b.label}</span>
                <button type="button" class="key-combo" class:recording={recordingBind === b.id}
                  onclick={() => (recordingBind = b.id)}
                  onkeydown={(e) => { if (recordingBind === b.id) recordKey(e, b.id); }}
                  onblur={() => { if (recordingBind === b.id) recordingBind = null; }}>
                  {recordingBind === b.id ? "Press keys…" : formatCombo(keyOf(b.id))}
                </button>
                <button type="button" class="icon-btn" title="Reset to default" onclick={() => resetKey(b.id)}>{@render iRefresh()}</button>
              </div>
            {/each}
            <div class="key-row static">
              <span class="key-label">Switch to tab 1–9</span>
              <span class="key-combo fixed">{isMac ? "⌘1–9" : "Ctrl+Shift+1–9"}</span>
              <span class="icon-spacer"></span>
            </div>
          </div>
          <div class="sheet-actions spread">
            <button type="button" class="link-btn" onclick={resetAllKeys}>{@render iRefresh()}<span>Reset all to defaults</span></button>
            <button type="button" class="btn" onclick={() => (modal = null)}>Close</button>
          </div>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  /* ─── design tokens ─────────────────────────────────────────────────── */
  /* Charte GitHub Primer (dark). Les fonds sont des paliers SOLIDES à teinte
     froide : c'est ce qui donne des menus nets là où un gris neutre bave.
     Échelle typo — 5 crans, rien entre les deux :
       10px  micro-label capitales suivies (en-têtes de section, jauges)
       11px  méta, hints, kbd, mono technique
       12px  UI secondaire (options, onglets, libellés de panneau)
       13px  corps (lignes, champs, boutons)
       16px  titre de feuille
       24px  wordmark */
  :global(:root) {
    --bg-app: #0d1117;                           /* canvas.default */
    --surface: #161b22;                          /* canvas.overlay — feuilles, panneaux */
    --surface-raised: #21262d;                   /* contrôle surélevé — btn secondaire, kbd */
    --surface-inset: #010409;                    /* canvas.inset — rail de segmented control */
    /* hover/active restent en alpha : ils se composent par-dessus des fonds
       différents (bg-app, feuille, panneau). Un palier solide en casserait un. */
    --surface-hover: rgba(177, 186, 196, 0.12);  /* neutral.muted */
    --surface-active: rgba(177, 186, 196, 0.2);
    --border: #30363d;                           /* border.default — visible, assumé */
    --border-strong: #3d444d;                    /* border.emphasis */
    --text-primary: #e6edf3;                     /* fg.default */
    --text-secondary: #9198a1;                   /* fg.muted */
    /* Écart assumé avec Primer : leur fg.subtle actuel (#656c76) tombe à
       3.57:1 sur le canvas et échoue AA en petit texte — or ce token porte les
       en-têtes de section (10px) et les hints (11px). #7d8590 (fg.subtle des
       Primer 2022) passe à 5.07 tout en restant nettement plus discret que
       --text-secondary. La hiérarchie tient par la taille et les capitales
       suivies, pas par un gris illisible. */
    --text-tertiary: #7d8590;
    /* Primer sépare deux bleus, et l'écart est un problème de contraste, pas
       de goût : --accent est un bleu de TEXTE (icônes/liens sur fond sombre),
       --selected un bleu de REMPLISSAGE (blanc par-dessus). Du blanc sur
       --accent tombe à 3.1:1 et échoue AA ; sur --selected il passe. */
    --accent: #2f81f7;                           /* accent.fg — sert aussi de survol pour --selected */
    --accent-subtle: rgba(47, 129, 247, 0.15);   /* accent.subtle — fonds de drop/record */
    --selected: #1f6feb;                         /* accent.emphasis */
    --attention: #d29922;                        /* attention.fg */
    --success: #3fb950;                          /* success.fg */
    --success-emphasis: #238636;                 /* btn.primary — la seule action affirmative */
    --success-hover: #2ea043;
    --danger: #f85149;                           /* danger.fg */
    --radius-sm: 5px;
    --radius-md: 6px;
    --radius-lg: 10px;
    --focus-ring: 0 0 0 3px rgba(47, 129, 247, 0.4); /* net, pas un halo */
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
    background: rgba(47, 129, 247, 0.4);
  }
  :global(*:focus-visible) {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  :global(::-webkit-scrollbar) { width: 8px; height: 8px; }
  :global(::-webkit-scrollbar-thumb) { background: rgba(110, 118, 129, 0.4); border-radius: 4px; }
  :global(::-webkit-scrollbar-thumb:hover) { background: rgba(110, 118, 129, 0.6); }
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
    height: 38px; /* calé sur la titlebar du contenu ; dégage les feux macOS (y=20) */
    flex: none;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding: 0 10px;
  }
  /* marque de l'app : logo + nom, en pied de sidebar (au-dessus de Settings) —
     loin des feux macOS, identique sur Windows/Linux */
  .sb-foot { flex: none; }
  /* marque discrète (« fantôme ») sous Settings, avec la version */
  .sb-brand {
    display: flex;
    align-items: center;
    gap: 7px;
    margin: 6px 8px 8px;
    padding: 2px 10px;
  }
  .sb-logo { display: flex; flex: none; width: 15px; }
  .sb-logo :global(svg) { width: 15px; height: 15px; }
  .sb-wordmark {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sb-ver { font-size: 11px; color: var(--text-tertiary); font-variant-numeric: tabular-nums; }
  .sb-toggle { opacity: 0; transition: opacity 120ms; }
  .sidebar:hover .sb-toggle { opacity: 1; }
  .sb-scroll { flex: 1; overflow-y: auto; min-height: 0; padding-bottom: 8px; }
  .sb-section { margin-top: 10px; }
  .sb-scroll .sb-section:first-child { margin-top: 2px; } /* colle « Terminals » près du haut */
  /* En-tête de section : capitales suivies, 10px/600. Le gras lourd à faible
     contraste (11px/700 à 35% d'opacité) bavait ; les capitales portent la
     hiérarchie, le poids n'a plus à la porter tout seul. */
  .sb-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px 4px 16px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }
  .sb-add { opacity: 0; }
  .sb-section:hover .sb-add { opacity: 1; }
  .sb-empty {
    margin: 2px 16px;
    font-size: 12px;
    font-style: italic;
    color: var(--text-tertiary);
  }
  /* état vide : icône + titre + hint + action (sidebar comme modales) */
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 5px;
    padding: 20px 12px;
  }
  .empty-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    margin-bottom: 3px;
    border-radius: 9px;
    background: var(--surface-hover);
    color: var(--text-tertiary);
  }
  .empty-title { margin: 0; font-size: 12px; font-weight: 500; color: var(--text-secondary); }
  .empty-hint { margin: 0; font-size: 11px; line-height: 1.4; color: var(--text-tertiary); max-width: 230px; }
  .empty-btn { margin-top: 7px; height: 26px; padding: 0 12px; font-size: 12px; }
  .sb-settings {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    /* bas à 2px : Connections + Settings forment une paire, elles se lisent
       comme un menu, pas comme deux blocs séparés (.sb-brand rattrape l'écart) */
    margin: 8px 8px 2px;
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
  .sb-settings + .sb-settings { margin-top: 0; }

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
  /* champ de renommage inline (sidebar + barre de pane) */
  .row-rename, .pane-rename {
    flex: 1; min-width: 0; width: auto; height: 20px; padding: 0 6px;
    font-size: 13px; border-radius: 4px; border: 1px solid var(--accent);
    background: var(--bg-app); color: var(--text-primary);
  }
  .pane-rename { flex: none; width: 170px; height: 18px; font-size: 11px; }
  .row-rename:focus, .pane-rename:focus { box-shadow: none; border-color: var(--accent); }
  .row-meta { font-size: 11px; color: var(--text-tertiary); flex: none; }
  /* badge Primer : 10px capitales suivies. La bordure était en rgba(94,124,226)
     — un bleu-violet qui ne correspondait à aucun accent du thème. */
  .row-tag {
    flex: none;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--accent);
    background: var(--accent-subtle);
    border: 1px solid rgba(47, 129, 247, 0.35);
    border-radius: 4px;
    padding: 0 5px;
  }
  .row-actions { display: none; align-items: center; gap: 2px; flex: none; }
  .row:hover .row-actions { display: flex; }
  /* le survol sort les boutons : on rend au nom la place de la pastille (6px +
     gap), sinon un nom de 11 caractères est déjà rogné. */
  .row:hover .row-tag, .row:hover .row-meta, .row:hover .dot { display: none; }
  /* variante « manager » (modale Connections) : 2 lignes, détails + actions toujours visibles */
  .row.mgr { height: auto; min-height: 44px; padding-top: 5px; padding-bottom: 5px; }
  .row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; overflow: hidden; }
  .row-main .row-label { flex: none; }
  .row-lock { font-size: 10px; }
  .row-sub { font-size: 11px; color: var(--text-tertiary); font-family: ui-monospace, Menlo, monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row.mgr .row-actions { display: flex; }
  .row.mgr:hover .row-tag, .row.mgr:hover .row-meta { display: inline-flex; }
  .row.mgr:hover .dot { display: block; } /* le manager est sur 2 lignes : il a la place */
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
  .row.drop { box-shadow: inset 0 0 0 1.5px var(--accent); background: var(--accent-subtle); }
  .row.drop-before { box-shadow: inset 0 2px 0 var(--accent); }
  .row.drop-after { box-shadow: inset 0 -2px 0 var(--accent); }
  .row.drop-merge { box-shadow: inset 0 0 0 1.5px var(--accent); background: var(--accent-subtle); }
  .sb-section.drop { box-shadow: inset 0 0 0 1.5px var(--accent); border-radius: var(--radius-md); }
  .row .row-plus { display: none; flex: none; }
  .row:hover .row-plus { display: inline-flex; }
  .row.dim { cursor: pointer; }
  .row.dim .row-label { color: var(--text-tertiary); }
  .row.dim .row-icon { color: var(--text-tertiary); }
  .row.dim:hover .row-icon { color: var(--accent); } /* survol : « cliquer pour rouvrir » */
  /* sélection : rempli en --selected (accent.emphasis), pas en --accent —
     du blanc sur --accent tombe à 3.1:1 et échoue AA. */
  .row.current { background: var(--selected); }
  .row.current .row-label, .row.current .row-icon, .row.current .row-spin { color: #fff; }
  .row.current .dot.attention { background: #fff; }
  .row.current .row-actions .icon-btn { color: rgba(255, 255, 255, 0.8); }
  .row-label.strong { font-weight: 500; }
  .row-spin { color: var(--text-tertiary); display: flex; flex: none; }
  .chev { width: 16px; height: 16px; flex: none; transition: transform 150ms var(--ease); }
  .chev.open { transform: rotate(90deg); }

  /* question de choix (menu agent) — utilisée dans les notifs */
  .agent-q { font-size: 11px; color: var(--text-secondary); line-height: 1.3; }
  .choice-row { display: flex; gap: 4px; }
  .choice-row.wrap { flex-wrap: wrap; }
  .choice {
    display: inline-flex; align-items: center; max-width: 100%;
    padding: 3px 7px; border-radius: 6px; cursor: pointer;
    font-size: 11px; line-height: 1.2; text-align: left;
    background: var(--surface-active); /* --surface-2 n'a jamais existé : c'était le fallback qui peignait */
    border: 1px solid transparent; color: var(--text-primary);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .choice:hover { border-color: var(--accent); }
  .choice b { color: var(--accent); margin-right: 3px; font-weight: 700; }
  .choice.yes:hover { border-color: var(--success); }
  .choice.no:hover { border-color: var(--danger, #ff453a); }

  /* même traitement que .sb-head : les sections des modales et de la sidebar
     doivent se lire comme un seul système */
  .mgr-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: 18px 0 6px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-tertiary);
  }
  .mgr-head:first-of-type { margin-top: 0; }
  .mgr-btns { display: flex; gap: 2px; }
  .import-row { margin: 0; }
  .sync-row { display: flex; gap: 8px; }
  .sync-row .btn { display: inline-flex; align-items: center; gap: 6px; flex: 1; justify-content: center; }
  .config-paste {
    width: 100%; height: 160px; box-sizing: border-box; resize: vertical;
    background: var(--bg-app); border: 1px solid var(--border-strong); border-radius: var(--radius-md);
    padding: 8px 10px; color: var(--text-primary);
    font-family: ui-monospace, Menlo, monospace; font-size: 12px; line-height: 1.4;
    transition: box-shadow 120ms, border-color 120ms;
  }
  .config-paste:focus { border-color: var(--accent); box-shadow: var(--focus-ring); outline: none; }
  .config-paste::placeholder { color: var(--text-tertiary); }
  .cmd-area {
    width: 100%; box-sizing: border-box; resize: vertical;
    background: var(--bg-app); border: 1px solid var(--border-strong); border-radius: var(--radius-md);
    padding: 6px 10px; color: var(--text-primary);
    font-family: ui-monospace, Menlo, monospace; font-size: 12px; line-height: 1.5;
    transition: box-shadow 120ms, border-color 120ms;
  }
  .cmd-area:hover { border-color: rgba(255, 255, 255, 0.22); }
  .cmd-area:focus { border-color: var(--accent); box-shadow: var(--focus-ring); outline: none; }
  .cmd-area::placeholder { color: var(--text-tertiary); }
  .import-btn { height: 22px; padding: 0 10px; font-size: 12px; flex: none; }
  .sheet .row { margin: 0; }
  .sheet-actions.spread { justify-content: space-between; align-items: center; }
  .sheet.wide { width: 560px; }
  .sheet.diff { width: min(860px, 92vw); }

  /* vue de diff */
  .diff-head { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  .diff-path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .diff-mode { flex: none; font-size: 11px; font-weight: 500; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-strong); border-radius: var(--radius-sm); padding: 1px 7px; }
  .diff-mode.conflict { color: var(--danger); border-color: var(--danger); }
  .diff-body {
    margin: 12px 0; max-height: 62vh; overflow: auto;
    background: var(--bg-app); border: 1px solid var(--border); border-radius: var(--radius-md); padding: 6px 0;
    font-family: ui-monospace, Menlo, monospace; font-size: 12px; line-height: 1.5;
  }
  /* pre-wrap : une ligne longue s'enroule au lieu d'imposer un scroll horizontal */
  .dl { padding: 0 10px; white-space: pre-wrap; word-break: break-word; color: var(--text-primary); }
  .dl.add { background: rgba(46, 160, 67, 0.15); }
  .dl.del { background: rgba(248, 81, 73, 0.15); }
  .dl.hunk { color: var(--text-tertiary); background: var(--surface-hover); margin: 4px 0; }
  .dl.meta { color: var(--text-tertiary); }

  /* segmented control (onglets de réglages) — rail en creux, onglet actif surélevé */
  .seg { display: flex; gap: 2px; padding: 3px; margin-bottom: 16px; background: var(--surface-inset); border: 1px solid var(--border); border-radius: var(--radius-md); }
  .seg button { flex: 1; padding: 5px 10px; border: 1px solid transparent; background: transparent; color: var(--text-secondary); font: inherit; font-size: 12px; font-weight: 500; border-radius: var(--radius-sm); cursor: pointer; transition: background 120ms var(--ease), color 120ms var(--ease); }
  .seg button:hover { color: var(--text-primary); }
  .seg button.on { background: var(--surface-raised); border-color: var(--border-strong); color: var(--text-primary); }

  /* liste des raccourcis */
  .keys-list { display: flex; flex-direction: column; gap: 1px; max-height: 56vh; overflow-y: auto; margin: 0 -4px; }
  .key-row { display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: var(--radius-sm); }
  .key-row:hover { background: var(--surface-hover); }
  .key-row.static:hover { background: transparent; }
  .key-label { flex: 1; font-size: 12px; color: var(--text-primary); }
  .key-combo { min-width: 92px; padding: 3px 10px; text-align: center; font-family: ui-monospace, Menlo, monospace; font-size: 12px; color: var(--text-primary); background: var(--surface-raised); border: 1px solid var(--border-strong); border-radius: var(--radius-sm); cursor: pointer; transition: border-color 120ms var(--ease); }
  .key-combo:hover { border-color: var(--accent); }
  .key-combo.recording { border-color: var(--accent); color: var(--accent); background: var(--accent-subtle); }
  .key-combo.fixed { cursor: default; color: var(--text-tertiary); border-style: dashed; }
  .key-combo.fixed:hover { border-color: var(--border-strong); }
  .icon-spacer { width: 24px; }

  /* ─── titlebar + tabs ────────────────────────────────────────────────── */
  .content { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--bg-app); }
  .titlebar {
    height: 38px; /* toolbar unifiée compacte */
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
  }
  .traffic-pad { width: 68px; flex: none; }
  :global(body.win) .traffic-pad { width: 0; }
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
    font-size: 11px;
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
  .icon-btn:active:not(:disabled) { background: var(--surface-active); }
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

  /* panneau source control (git) */
  .git-panel { width: 300px; }
  .git-branch-wrap { position: relative; }
  .git-branch {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 3px 8px; border-radius: 999px;
    border: 1px solid var(--border); background: var(--surface-hover);
    color: var(--text-primary); font-size: 12px; font-weight: 600; cursor: pointer;
  }
  .git-branch:hover { border-color: var(--accent); }
  .git-branch-name { max-width: 130px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .git-count {
    min-width: 15px; height: 15px; padding: 0 4px; border-radius: 8px;
    background: color-mix(in srgb, var(--text-primary) 12%, transparent);
    color: var(--text-tertiary); font-size: 10px; font-weight: 600;
    display: inline-flex; align-items: center; justify-content: center;
  }
  .git-branches {
    position: absolute; top: calc(100% + 4px); left: 0; z-index: 5;
    min-width: 180px; max-height: 240px; overflow-y: auto;
    background: var(--bg-app); border: 1px solid var(--border);
    border-radius: var(--radius-sm); box-shadow: 0 8px 24px rgba(0,0,0,.3); padding: 4px;
  }
  .git-branch-item {
    display: block; width: 100%; text-align: left; padding: 5px 8px; border: none;
    background: transparent; border-radius: var(--radius-sm); font-size: 12px; color: var(--text-primary); cursor: pointer;
  }
  .git-branch-item:hover { background: var(--surface-hover); }
  .git-branch-item.current { color: var(--accent); font-weight: 600; }
  .git-graph-row {
    display: flex; align-items: center; gap: 6px; width: 100%; background: transparent;
    padding: 8px 12px; font-size: 12px; font-weight: 600; color: var(--text-primary);
    border: none; border-top: 1px solid var(--border); border-bottom: 1px solid var(--border); cursor: pointer;
  }
  .git-graph-row:hover { background: var(--surface-hover); }
  .git-graph-chev { display: inline-flex; transition: transform .12s; color: var(--text-tertiary); }
  .git-graph-chev.open { transform: rotate(90deg); }
  .git-log { max-height: 180px; overflow-y: auto; padding: 4px 12px; border-bottom: 1px solid var(--border); }
  .git-log-line { font-size: 11px; font-family: ui-monospace, SFMono-Regular, monospace; color: var(--text-tertiary); padding: 2px 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .git-commit-box { padding: 10px 12px 6px; }
  .git-msg {
    width: 100%; min-height: 56px; resize: vertical; padding: 8px 10px;
    background: var(--surface-hover); border: 1px solid var(--border);
    border-radius: var(--radius-sm); color: var(--text-primary); font: inherit; font-size: 12px;
  }
  .git-msg:focus { outline: none; border-color: var(--accent); }
  .git-commit-hint { font-size: 11px; color: var(--text-tertiary); padding: 4px 2px 0; }
  .git-status-line { display: flex; align-items: center; gap: 6px; padding: 4px 12px; font-size: 11px; color: var(--text-tertiary); }
  .git-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-tertiary); flex: none; }
  .git-dot.staged { background: var(--accent); }
  .git-upstream { margin-left: auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 150px; }
  .git-actions { display: flex; gap: 8px; padding: 6px 12px 10px; border-bottom: 1px solid var(--border); }
  .git-actions .btn { flex: 1; min-width: 0; }
  /* Le vert Primer, réservé à la SEULE action affirmative de l'app : Commit.
     C'est la signature GitHub. L'étendre à Save/Close la dissoudrait — et un
     « Close » vert ne veut rien dire.
     `:not(:disabled)` porte la spécificité, ne pas le retirer : `.btn` est
     déclaré PLUS BAS dans ce fichier ; à égalité (0,2,0 une fois la classe de
     scope Svelte ajoutée) il gagnerait à l'ordre source et le vert ne
     sortirait jamais. Le pseudo-classe passe la règle à (0,3,0). */
  .git-commit-btn:not(:disabled) { background: var(--success-emphasis); }
  .git-commit-btn:not(:disabled):hover { background: var(--success-hover); }
  /* fetch en cours : seule l'icône tourne (.spin global ferait tourner tout le bouton) */
  .icon-btn.fetching :global(svg) { animation: rot 800ms linear infinite; }
  .git-changes-head {
    display: flex; align-items: center; justify-content: space-between;
    padding: 8px 12px 4px; font-size: 10px; font-weight: 600; letter-spacing: .06em;
    color: var(--text-tertiary); text-transform: uppercase;
  }
  .git-all { font-size: 11px; color: var(--accent); background: transparent; border: none; cursor: pointer; text-transform: none; letter-spacing: 0; }
  .git-all:hover { text-decoration: underline; }
  .git-row { display: flex; align-items: center; gap: 8px; }
  .git-file { flex: 1; min-width: 0; display: flex; align-items: baseline; gap: 6px; border: none; background: transparent; padding: 0; font: inherit; text-align: left; cursor: pointer; }
  .git-file .row-icon { align-self: center; }
  .git-file:hover .git-name { color: var(--accent); }
  .git-name { font-size: 12px; color: var(--text-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .git-sub { font-size: 11px; color: var(--text-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: none; max-width: 90px; }
  .git-name.conflict { color: var(--danger); }
  .git-confl { flex: none; color: var(--danger); font-size: 12px; font-weight: 600; padding: 0 4px; }
  .git-check {
    width: 18px; height: 18px; flex: none; border-radius: 4px;
    border: 1.5px solid var(--border); background: transparent;
    display: inline-flex; align-items: center; justify-content: center; cursor: pointer; color: #fff;
  }
  .git-check:hover { border-color: var(--accent); }
  .git-check.on { background: var(--selected); border-color: var(--selected); }
  .git-dim { color: var(--text-tertiary); font-size: 11px; }

  /* redirections de ports + aperçu */
  .fwd-count {
    flex: none;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: 8px;
    background: var(--selected);
    color: #fff;
    font-size: 10px;
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
    /* suit le thème : le padding autour de xterm doit être du fond terminal, pas du gris fixe */
    background: var(--term-bg, var(--bg-app));
  }
  .pane.active { box-shadow: inset 0 0 0 1px var(--border-strong); }
  .pane-bar {
    height: 26px;
    flex: none;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 8px;
    font-size: 11px;
    color: var(--text-tertiary);
    /* liseré latéral : marque le panneau focalisé sans toucher au contenu */
    border-left: 2px solid transparent;
    transition: background 100ms, border-color 100ms;
  }
  /* panneau focalisé : barre teintée accent + texte plein (façon iTerm) */
  .pane.active .pane-bar {
    color: var(--text-primary);
    background: color-mix(in srgb, var(--accent) 9%, transparent);
    border-left-color: var(--accent);
  }
  .pane-left { display: flex; align-items: center; gap: 6px; min-width: 0; flex: 1; }
  .pane-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* statut vivant du pane (working / waiting / done) */
  .pstat { display: inline-flex; align-items: center; gap: 4px; flex: none; max-width: 55%; }
  .pstat :global(svg) { flex: none; }
  .pstat-label {
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-family: var(--mono, ui-monospace, monospace); font-size: 11px;
  }
  .pstat.working { color: var(--accent); }
  .pstat.waiting { color: var(--attention); }
  .pstat.done { color: var(--success); animation: stat-fade 4s var(--ease) forwards; }
  @keyframes stat-fade { 0%, 40% { opacity: 1; } 100% { opacity: 0.45; } }
  /* statut compact sur les lignes de session */
  .sstat { display: inline-flex; flex: none; }
  .sstat.working { color: var(--accent); }
  .sstat.waiting { color: var(--attention); }
  .sstat.done { color: var(--success); }
  .sstat.error { color: var(--danger); }
  .pane-btns { display: flex; align-items: center; gap: 1px; opacity: 0; transition: opacity 120ms; }
  .pane:hover .pane-btns { opacity: 1; }
  .pane-btns .icon-btn { width: 20px; height: 20px; }
  .pane-term { flex: 1; min-height: 0; padding: 0 10px 8px 10px; position: relative; }

  /* ─── panneau navigateur ─────────────────────────────────────────────── */
  .browser-ico { display: flex; flex: none; color: var(--text-tertiary); }
  .url-input {
    flex: 1; min-width: 0; height: 18px; padding: 0 8px;
    font-size: 11px; border-radius: 4px;
    border: 1px solid var(--border); background: var(--bg-app);
    color: var(--text-primary); font-family: inherit;
  }
  .url-input:focus { border-color: var(--accent); box-shadow: none; }
  .browser-body {
    flex: 1; min-height: 0; position: relative;
    margin: 0 8px 8px; border-radius: 6px; overflow: hidden; background: #fff;
  }
  .browser-frame { width: 100%; height: 100%; border: none; background: #fff; }
  .browser-empty {
    position: absolute; inset: 0;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 8px; background: var(--bg-app); color: var(--text-tertiary); font-size: 12px;
  }
  .browser-empty :global(svg) { width: 22px; height: 22px; opacity: 0.5; }
  .browser-empty code { font-size: 11px; background: var(--surface); padding: 1px 5px; border-radius: 3px; }

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
  .veil-target { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text-tertiary); user-select: text; }
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
  .wordmark { font-size: 24px; font-weight: 700; color: var(--text-secondary); letter-spacing: -0.3px; } /* Large Title */
  .welcome .hint { margin: 0 0 12px; font-size: 13px; color: var(--text-tertiary); }
  .welcome-list { width: 300px; display: flex; flex-direction: column; gap: 1px; }

  /* ─── boutons / formulaires ──────────────────────────────────────────── */
  /* Boutons Primer. Le rempli est en --selected (accent.emphasis) et non en
     --accent : voir la note contraste des tokens. */
  .btn {
    height: 30px;
    padding: 0 16px;
    background: var(--selected);
    color: #fff;
    border: 1px solid rgba(240, 246, 252, 0.1);
    border-radius: var(--radius-md);
    font-size: 13px;
    font-weight: 500;
    font-family: inherit;
    transition: background 100ms;
  }
  .btn:hover { background: var(--accent); }
  .btn:active { background: #1a5fcc; }
  .btn:disabled { opacity: 0.5; }
  /* bouton secondaire Primer : palier solide + bordure, pas un voile blanc */
  .btn.ghost {
    background: var(--surface-raised);
    border: 1px solid var(--border-strong);
    color: var(--text-primary);
  }
  .btn.ghost:hover { background: var(--border); border-color: #656c76; }
  .btn.sm { height: 24px; padding: 0 10px; font-size: 11px; border-radius: var(--radius-sm); display: inline-flex; align-items: center; gap: 5px; }

  input, select {
    height: 30px;
    box-sizing: border-box;
    width: 100%;
    background: var(--bg-app); /* champ en creux sur la feuille */
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    padding: 0 10px;
    color: var(--text-primary);
    font-size: 13px;
    font-family: inherit;
    transition: box-shadow 120ms, border-color 120ms;
  }
  input:hover, select:hover { border-color: #656c76; }
  input::placeholder { color: var(--text-tertiary); }
  input:focus, select:focus { border-color: var(--accent); box-shadow: var(--focus-ring); outline: none; }
  input[type="checkbox"] { width: auto; height: auto; accent-color: var(--accent); }
  input[type="number"] { appearance: textfield; }

  /* ─── modales ────────────────────────────────────────────────────────── */
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(1, 4, 9, 0.65);
    backdrop-filter: blur(8px) saturate(120%);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 30;
  }
  /* La feuille est en --surface (canvas.overlay) et non en --surface-raised :
     --surface-raised devient le palier des contrôles POSÉS dessus (btn ghost,
     seg actif, kbd). Deux paliers distincts = des contrôles qui se détachent. */
  .sheet {
    width: 440px;
    max-height: 82vh;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: 0 16px 32px rgba(1, 4, 9, 0.85);
    padding: 22px 24px;
  }
  .sheet h2 { margin: 0 0 18px; font-size: 16px; font-weight: 600; letter-spacing: -0.01em; }
  .sheet form { display: flex; flex-direction: column; gap: 14px; }
  .sheet label { display: flex; flex-direction: column; gap: 5px; }
  .f-label { font-size: 11px; font-weight: 500; letter-spacing: 0.01em; color: var(--text-secondary); }
  .f-pair { display: flex; gap: 10px; }
  .f-pair .grow { flex: 1; }
  .f-port { width: 80px; flex: none; }
  .f-check { flex-direction: row !important; align-items: center; gap: 12px !important; font-size: 12px; color: var(--text-secondary); }
  .f-check span { flex: 1; line-height: 1.35; } /* texte à gauche, interrupteur poussé à droite */
  .f-check.sub-check { margin-left: 22px; margin-top: -6px; font-size: 12px; color: var(--text-tertiary); }
  .f-check code { font-size: 11px; background: var(--surface-active); border-radius: 3px; padding: 0 4px; }
  /* interrupteur iOS pour les options des modales (remplace la case native) */
  .f-check input[type="checkbox"] {
    appearance: none; -webkit-appearance: none;
    order: 2; flex: none;
    width: 34px; height: 20px; border-radius: 20px; border: none;
    background: var(--surface-active);
    position: relative; cursor: pointer; padding: 0;
    transition: background 160ms var(--ease);
  }
  .f-check input[type="checkbox"]::after {
    content: ""; position: absolute; top: 2px; left: 2px;
    width: 16px; height: 16px; border-radius: 50%;
    background: #fff; box-shadow: 0 1px 2px rgba(0, 0, 0, 0.35);
    transition: transform 160ms var(--ease);
  }
  .f-check input[type="checkbox"]:checked { background: var(--selected); }
  .f-check input[type="checkbox"]:checked::after { transform: translateX(14px); }
  .f-check input[type="checkbox"]:hover { border: none; }
  .f-check input[type="checkbox"]:focus { box-shadow: none; }
  .f-check input[type="checkbox"]:focus-visible { box-shadow: var(--focus-ring); }
  .f-hint { margin: -4px 0 0; font-size: 11px; line-height: 1.4; color: var(--text-tertiary); }
  /* Groupe de réglages : carte bordée, lignes séparées par un filet. Remplace
     la liste d'interrupteurs à plat où chaque option flottait au même niveau
     que les champs — rien ne disait ce qui allait avec quoi. */
  .group {
    display: flex;
    flex-direction: column;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .group .f-check { margin: 0; padding: 10px 12px; }
  .group .f-check + .f-check { border-top: 1px solid var(--border); }
  /* sous-option : pas de filet, elle doit rester collée à son parent */
  .group .f-check.sub-check { margin: 0; padding-left: 34px; border-top: none; }
  .group .f-check:hover { background: var(--surface-hover); }
  .group .f-hint { margin: 0; padding: 8px 12px; border-top: 1px solid var(--border); background: var(--surface); }
  /* dans une feuille, le form pose déjà un gap de 14px : le sous-titre se cale
     dessus au lieu d'empiler ses propres marges */
  .sheet form .mgr-head { margin: 2px 0 -6px; }
  .emoji-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(38px, 1fr));
    gap: 2px;
    max-height: 180px;
    overflow-y: auto;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-app);
  }
  .emoji-cell {
    display: grid;
    place-items: center;
    aspect-ratio: 1;
    padding: 0;
    border: 1px solid transparent;
    border-radius: 6px;
    background: none;
    color: var(--text-tertiary);
    cursor: pointer;
  }
  .emoji-cell:hover { background: var(--surface-hover); }
  .emoji-cell.sel { border-color: var(--accent); background: var(--surface-active); }
  .sheet-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
  .split-list { display: flex; flex-direction: column; gap: 1px; margin: 0 -8px; }
  /* Listes de la modale Connections : mêmes cartes bordées que .group, pour que
     « remotes » et « identities » se lisent comme des sections et non comme des
     lignes lâchées sur le fond de la feuille. */
  .split-list.carded {
    margin: 0;
    gap: 0;
    background: var(--bg-app);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .split-list.carded .row { border-radius: 0; margin: 0; padding: 0 10px; }
  .split-list.carded .row + .row { border-top: 1px solid var(--border); }
  .split-list.carded .empty { padding: 18px 12px; }
  .split-filter { margin-bottom: 10px; }
  .palette-input { margin-bottom: 10px; }
  .palette-list { max-height: 52vh; overflow-y: auto; }
  .palette-list .row { cursor: pointer; }
  /* ligne active (clavier ↑/↓ ou survol) — même bleu de remplissage que .row.current */
  .palette-list .row.sel { background: var(--selected); }
  .palette-list .row.sel .row-label,
  .palette-list .row.sel .row-icon,
  .palette-list .row.sel .row-meta { color: #fff; }
  .palette-list .row:hover { background: transparent; } /* la sélection suit la souris, pas le hover natif */
  .kbd {
    flex: none;
    font-family: ui-monospace, Menlo, monospace;
    font-size: 11px;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px; /* touche physique — le seul relief assumé de l'UI */
    border-radius: 4px;
    padding: 1px 6px;
  }
  .palette-list .row.sel .kbd { color: #fff; background: rgba(255, 255, 255, 0.16); border-color: rgba(255, 255, 255, 0.3); }

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
  .font-opt.cur { color: var(--accent); }
  .font-opt.cur:hover { color: #fff; }
  .font-empty { padding: 8px 10px; font-size: 12px; color: var(--text-tertiary); }
  .font-field { display: flex; gap: 4px; align-items: center; }
  .font-field input { flex: 1; }
  .font-reset { flex: none; }
  .link-btn {
    display: inline-flex; align-items: center; gap: 5px; align-self: flex-start;
    margin: -4px 0 0; padding: 2px 0; background: none; border: none;
    color: var(--accent); font-size: 12px; font-family: inherit;
  }
  .link-btn:hover { text-decoration: underline; }

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
    background: rgba(33, 38, 45, 0.9);
    backdrop-filter: blur(30px) saturate(180%);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg); /* notification macOS */
    padding: 9px 14px;
    font-size: 13px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    max-width: 480px;
  }
  .toast.error { border-left: 2px solid var(--danger); }
  .toast.success { border-left: 2px solid var(--success); }

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
  /* `att-card` et pas `attention` : `.dot.attention` (pastille de la sidebar)
     matchait aussi `.attention`, à égalité de spécificité — la carte gagnait à
     l'ordre source et repeignait la pastille en carte de 24px. */
  .att-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: rgba(33, 38, 45, 0.9);
    backdrop-filter: blur(30px) saturate(180%);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    padding: 7px 8px;
    font-size: 12px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  }
  .att-card.hasopts { min-width: 260px; }
  .att-head { display: flex; align-items: center; gap: 6px; }
  .att-icon { display: inline-flex; flex: none; color: var(--attention); }
  .att-icon.stop { color: var(--success); }
  .att-card .choice { background: var(--surface-active); color: var(--text-primary); }
  .att-card .agent-q { color: var(--text-secondary); padding: 0 4px; }
  .att-msg {
    flex: 1;
    background: none;
    border: none;
    color: var(--text-primary);
    text-align: left;
    font-size: 12px;
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
  }
  .search-bar input { width: 170px; height: 24px; }

  /* terminal : sélection de texte autorisée */
  .pane-term :global(.xterm) { user-select: text; }

  /* respecte « réduire les animations » du système (accessibilité) */
  @media (prefers-reduced-motion: reduce) {
    :global(*), :global(*::before), :global(*::after) {
      animation-duration: 0.01ms !important;
      transition-duration: 0.01ms !important;
    }
  }
</style>
