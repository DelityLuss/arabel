// Moteur du mode contrôle tmux (`tmux -CC`).
//
// tmux -CC n'affiche pas un TUI : il émet sur stdout un protocole ligne-à-ligne
// (`%output`, `%layout-change`, …) et lit des commandes tmux sur stdin. Ce
// module parse ce flux et expose des callbacks ; le rendu (xterm par panneau)
// vit dans le composant. Les fonctions pures ont un self-check dans `demo()`.

/** Arbre de layout tmux : feuille (paneId) ou groupe (dir h = côte à côte, v = empilé). */
export type Lay = { w: number; h: number; paneId?: string; dir?: "h" | "v"; children?: Lay[] };

/** Désescape la donnée d'un `%output` (octets non imprimables encodés `\ooo`). */
export function unescapeOutput(s: string): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (c === "\\" && /[0-7]/.test(s[i + 1] || "") && /[0-7]/.test(s[i + 2] || "") && /[0-7]/.test(s[i + 3] || "")) {
      out.push(parseInt(s.substr(i + 1, 3), 8));
      i += 3;
    } else {
      out.push(c.charCodeAt(0) & 0xff);
    }
  }
  return Uint8Array.from(out);
}

/** Parse une chaîne de layout tmux (`checksum,WxH,X,Y{...}`). */
export function parseLayout(input: string): Lay {
  const s = input.slice(input.indexOf(",") + 1); // ôte le checksum
  let i = 0;
  const num = () => { let j = i; while (i < s.length && s[i] >= "0" && s[i] <= "9") i++; return parseInt(s.slice(j, i), 10); };
  const expect = (c: string) => { if (s[i] !== c) throw new Error(`layout: attendu '${c}' pos ${i} dans '${s}'`); i++; };
  function cell(): Lay {
    const w = num(); expect("x"); const h = num(); expect(","); num(); expect(","); num();
    if (s[i] === "{" || s[i] === "[") {
      const dir = s[i] === "{" ? "h" : "v"; const close = s[i] === "{" ? "}" : "]"; i++;
      const children = [cell()];
      while (s[i] === ",") { i++; children.push(cell()); }
      expect(close);
      return { w, h, dir, children };
    }
    expect(","); return { w, h, paneId: String(num()) };
  }
  return cell();
}

/** Convertit un layout tmux (n-aire) en arbre binaire (droite-penché) type arabel. */
export function layToTree<T>(lay: Lay, leafFor: (paneId: string) => T): T | { dir: "h" | "v"; ratio: number; a: any; b: any } {
  if (lay.paneId) return leafFor(lay.paneId);
  const kids = lay.children!;
  const size = (c: Lay) => (lay.dir === "h" ? c.w : c.h);
  const build = (arr: Lay[]): any => {
    if (arr.length === 1) return layToTree(arr[0], leafFor);
    const total = arr.reduce((s, c) => s + size(c), 0);
    return { dir: lay.dir!, ratio: size(arr[0]) / total, a: layToTree(arr[0], leafFor), b: build(arr.slice(1)) };
  };
  return build(kids);
}

/** Liste les paneIds présents dans un layout. */
export function layPanes(lay: Lay): string[] {
  return lay.paneId ? [lay.paneId] : lay.children!.flatMap(layPanes);
}

/** Taille (colonnes×lignes) de chaque panneau, telle que tmux la découpe. La
 *  grille xterm doit s'y caler EXACTEMENT, sinon le TUI se réécrit par-dessus. */
export function layPaneSizes(lay: Lay): Record<string, { w: number; h: number }> {
  const out: Record<string, { w: number; h: number }> = {};
  const walk = (l: Lay) => (l.paneId != null ? (out[l.paneId] = { w: l.w, h: l.h }) : l.children!.forEach(walk));
  walk(lay);
  return out;
}

/** Encode une saisie (string xterm) en octets hex pour `send-keys -H`. */
export function toHexKeys(data: string): string {
  return Array.from(new TextEncoder().encode(data), (b) => b.toString(16).padStart(2, "0")).join(" ");
}

export type CcEvents = {
  output: (paneId: string, bytes: Uint8Array) => void;
  layout: (windowId: string, tree: Lay) => void;
  windowClose: (windowId: string) => void;
  windowRenamed: (windowId: string, name: string) => void;
  paneActive: (windowId: string, paneId: string) => void;
  reply: (lines: string[], error: boolean) => void; // réponse à une commande %begin…%end
  exit: (reason: string) => void;
};

/** Parseur incrémental du flux control-mode. `feed()` reçoit les octets (chaîne latin1). */
export class TmuxControl {
  private buf = "";
  private inReply = false;
  private isCmdReply = false;
  private replyLines: string[] = [];
  constructor(private ev: CcEvents) {}

  feed(chunk: string) {
    this.buf += chunk;
    let nl: number;
    while ((nl = this.buf.indexOf("\n")) >= 0) {
      let line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      if (line.endsWith("\r")) line = line.slice(0, -1);
      this.handle(line);
    }
  }

  private handle(line: string) {
    // tmux -CC ouvre le flux par un DCS (`\033P1000p`) COLLÉ au premier %begin,
    // et le referme par ST. Sans ce nettoyage, `startsWith("%begin")` rate le
    // bloc initial et ses lignes sont prises pour des notifications.
    // (Les données `%output` sont échappées en `\ooo` : jamais d'ESC brut ici.)
    line = line.replace(/^\x1bP1000p/, "").replace(/\x1b\\$/, "");
    if (this.inReply) {
      if (line.startsWith("%end") || line.startsWith("%error")) {
        this.inReply = false;
        // seul un bloc de RÉPONSE consomme un handler : livrer le bloc
        // d'ouverture de tmux décalerait toute la file d'un cran.
        if (this.isCmdReply) this.ev.reply(this.replyLines, line.startsWith("%error"));
      } else this.replyLines.push(line);
      return;
    }
    if (line.startsWith("%begin")) {
      this.inReply = true;
      this.replyLines = [];
      // `%begin <time> <num> <flags>` : bit 0 = « réponse à une commande du
      // client ». tmux ouvre le control-mode par un bloc à flags=0 qui ne répond
      // à aucune commande — c'est le seul moyen de le distinguer.
      // ponytail: le man tmux dit « flags: currently not used », mais tmux émet
      // 0 (ouverture) vs 1 (réponse) de façon constante — vérifié en 3.7b, y
      // compris sur %error. Le `& 1` survit à l'ajout de bits. Si un tmux futur
      // cassait ça, le symptôme serait un onglet -CC vide : voir demo().
      this.isCmdReply = (parseInt(line.split(" ")[3], 10) & 1) === 1;
      return;
    }

    const seg = line.split(" ");
    switch (seg[0]) {
      case "%output": {
        // %output %<pane> <data> — data peut contenir des espaces
        const rest = line.slice("%output ".length);
        const sp = rest.indexOf(" ");
        if (sp < 0) return;
        this.ev.output(rest.slice(0, sp).replace(/^%/, ""), unescapeOutput(rest.slice(sp + 1)));
        return;
      }
      case "%layout-change":
        // %layout-change @win layout visible-layout flags
        try { this.ev.layout(seg[1].replace(/^@/, ""), parseLayout(seg[2])); } catch { /* layout illisible : ignoré */ }
        return;
      case "%window-close":
      case "%unlinked-window-close":
        this.ev.windowClose(seg[1].replace(/^@/, ""));
        return;
      case "%window-renamed":
        this.ev.windowRenamed(seg[1].replace(/^@/, ""), seg.slice(2).join(" "));
        return;
      case "%window-pane-changed":
        this.ev.paneActive(seg[1].replace(/^@/, ""), seg[2].replace(/^%/, ""));
        return;
      case "%exit":
        this.ev.exit(line.slice("%exit".length).trim());
        return;
      // %session-changed, %sessions-changed, %client-*, %pane-mode-changed,
      // %window-add, %continue, %pause… : non nécessaires au MVP.
    }
  }
}

/** Self-check des fonctions pures (lancé en dev). */
export function demo() {
  const eq = (a: unknown, b: unknown, m: string) => { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`tmuxcc demo: ${m}`); };
  eq([...unescapeOutput("a\\033[31mb")], [97, 27, 91, 51, 49, 109, 98], "unescape esc");
  eq([...unescapeOutput("\\303\\251")], [195, 169], "unescape utf8");
  eq(parseLayout("bc62,80x24,0,0,0").paneId, "0", "layout leaf");
  const lh = parseLayout("e9b4,80x24,0,0{40x24,0,0,1,39x24,41,0,2}");
  eq(lh.dir, "h", "layout h");
  eq(layPanes(lh), ["1", "2"], "layout panes");
  eq(layPaneSizes(lh), { "1": { w: 40, h: 24 }, "2": { w: 39, h: 24 } }, "layout pane sizes");
  eq(layToTree(lh, (p) => ({ leaf: p })), { dir: "h", ratio: 40 / 79, a: { leaf: "1" }, b: { leaf: "2" } }, "toTree");
  eq(toHexKeys("hi"), "68 69", "hex");

  // parseur de flux
  const got: Record<string, unknown> = {};
  const ctrl = new TmuxControl({
    output: (p, b) => (got.out = [p, [...b]]),
    layout: (w, t) => (got.lay = [w, layPanes(t)]),
    windowClose: (w) => (got.close = w),
    windowRenamed: () => {},
    paneActive: (w, p) => (got.active = [w, p]),
    reply: (lines, err) => (got.reply = [lines, err]),
    exit: () => (got.exit = true),
  });
  ctrl.feed("%output %2 hi\\033x\n");
  eq(got.out, ["2", [104, 105, 27, 120]], "cc output");
  ctrl.feed("%layout-change @1 e9b4,80x24,0,0{40x24,0,0,1,39x24,41,0,2} vis *\n");
  eq(got.lay, ["1", ["1", "2"]], "cc layout");
  ctrl.feed("%begin 1 1 1\nligne\n%end 1 1 1\n");
  eq(got.reply, [["ligne"], false], "cc reply");
  // Flux RÉEL de tmux -CC (capturé en 3.7b) : le tout premier %begin est préfixé
  // d'un DCS \033P1000p ET porte flags=0 — c'est le bloc d'ouverture de tmux, il
  // ne répond à AUCUNE commande. S'il était livré comme une réponse, il mangerait
  // le handler du bootstrap et l'onglet -CC resterait vide.
  got.reply = null;
  ctrl.feed("\x1bP1000p%begin 2 279 0\n%end 2 279 0\n");
  eq(got.reply, null, "bloc d'ouverture (flags=0) ne consomme pas de handler");
  ctrl.feed("%begin 3 289 1\n@0|1|b25d,80x24,0,0,0\n%end 3 289 1\n");
  eq(got.reply, [["@0|1|b25d,80x24,0,0,0"], false], "réponse (flags=1) livrée");
  ctrl.feed("%begin 4 295 1\n%error 4 295 1\n");
  eq(got.reply, [[], true], "erreur (flags=1) livrée");
  ctrl.feed("%window-close @1\n");
  eq(got.close, "1", "cc close");
  return "ok";
}
