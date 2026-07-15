// « Préviens-moi » — point d'entrée unique pour attirer l'attention de l'utilisateur.
//
// Deux moitiés d'un même sujet :
//  1. LIRE l'état d'un tour Claude dans le buffer du terminal (`claudeTurn`) ;
//  2. le SIGNALER : son, notification système, badge du Dock (`notify`).
//
// Les appelants disent CE QUI arrive (`notify("done", …)`), pas COMMENT le
// signaler : préférences et permission système sont gérées une seule fois, ici.
// `claudeTurn` est pure → auto-testée dans `demo()`.

import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Événement à signaler. Détermine le ton joué et l'urgence perçue. */
export type NotifyKind = "waiting" | "done" | "error";
export type Live = "working" | "waiting" | "done" | null;

// ─── lecture de l'état dans le buffer ──────────────────────────────────────
// Claude Code imprime des marqueurs fiables dans son TUI : « esc to interrupt »
// tant qu'il génère, un menu « ❯ 1. … » quand il attend une réponse. C'est la
// seule source qui marche sans les hooks (non installés en local), donc la
// seule qui donne le check vert sur un pane local.

const WORKING = /esc to interrupt/i;
const MENU = [/❯\s*[1-9][.)]/, /\besc to cancel\b/i, /Do you want to proceed/i];

/** Fin de tour : délai sans marqueur avant de conclure (le footer clignote entre deux rendus). */
export const GRACE_MS = 1200;
/** Après un envoi clavier, Claude repart : on ne conclut pas « done » dans cette fenêtre. */
export const SUBMIT_MS = 2000;

/**
 * Statut d'un tour Claude d'après les dernières lignes du buffer.
 * `seen` = dernier instant où « esc to interrupt » était affiché (0 = jamais vu
 * travailler → le pane n'exécute pas Claude, on ne conclut rien).
 * Retourne le `seen` à mémoriser pour le prochain appel.
 */
export function claudeTurn(tail: string, seen: number, submit: number, now: number): { status: Live; seen: number } {
  if (WORKING.test(tail)) return { status: "working", seen: now }; // Claude génère (footer live)
  // menu de permission / choix : on garde la fenêtre de grâce ouverte, car à la
  // réponse le tour reprend — sans ça on conclurait « done » entre les deux.
  if (MENU.some((re) => re.test(tail))) return { status: "waiting", seen: seen ? now : 0 };
  if (!seen) return { status: null, seen: 0 }; // jamais vu travailler → pas notre affaire
  if (now - submit < SUBMIT_MS) return { status: "working", seen }; // tu viens d'envoyer → il repart
  if (now - seen < GRACE_MS) return { status: "working", seen };
  return { status: "done", seen }; // marqueur disparu → tour fini (collant jusqu'au suivant)
}

/** Préférences utilisateur : synchronisées depuis les settings de l'app. */
export const prefs = { sounds: true, notifications: true };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// permission demandée au démarrage (et pas au premier événement : sinon la
// toute première notif est avalée par la boîte de dialogue macOS).
let granted = false;
if (inTauri)
  (async () => {
    granted = (await isPermissionGranted()) || (await requestPermission()) === "granted";
  })();

// ─── sons : tons synthétisés (aucun fichier à bundler) ─────────────────────
let audioCtx: AudioContext | null = null;
// séquence de notes [fréquence Hz, décalage s] par événement
const SEQ: Record<NotifyKind, number[][]> = {
  waiting: [[660, 0], [880, 0.11]], // montant — « à toi de jouer »
  done: [[784, 0], [1047, 0.1]],    // quinte — terminé
  error: [[330, 0], [220, 0.13]],   // descendant — refus / erreur
};

export function playSound(kind: NotifyKind) {
  if (!prefs.sounds || typeof AudioContext === "undefined") return;
  try {
    audioCtx ??= new AudioContext();
    const ctx = audioCtx;
    if (ctx.state === "suspended") ctx.resume();
    for (const [freq, at] of SEQ[kind]) {
      const osc = ctx.createOscillator(), gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.value = freq;
      const t0 = ctx.currentTime + at;
      gain.gain.setValueAtTime(0, t0);
      gain.gain.linearRampToValueAtTime(0.16, t0 + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.001, t0 + 0.18);
      osc.connect(gain).connect(ctx.destination);
      osc.start(t0);
      osc.stop(t0 + 0.2);
    }
  } catch { /* audio indispo : silencieux */ }
}

/** Badge de l'icône du Dock (0 → aucun badge). */
export function setBadge(n: number) {
  if (inTauri) getCurrentWindow().setBadgeCount(n || undefined).catch(() => {});
}

/** Signale un événement : son + notification système, selon les préférences. */
export function notify(kind: NotifyKind, title: string, body: string) {
  if (granted && prefs.notifications) sendNotification({ title, body });
  playSound(kind);
}

/** Auto-test des fonctions pures (appelé en mode démo navigateur). */
export function demo() {
  const ok = (c: boolean, m: string) => { if (!c) throw new Error("notify: " + m); };
  const T = 100_000; // « maintenant » arbitraire

  // pane ordinaire : jamais vu Claude → aucun statut, et surtout jamais « done »
  ok(claudeTurn("$ ls\nREADME.md\n", 0, 0, T).status === null, "shell inerte");
  ok(claudeTurn("$ git push\n", 0, T, T).status === null, "un Entrée dans un shell ne crée pas d'état");

  // Claude travaille → arme le marqueur
  const w = claudeTurn("… (esc to interrupt)\n", 0, 0, T);
  ok(w.status === "working" && w.seen === T, "esc to interrupt → working");

  // marqueur disparu : grâce, puis fin de tour
  ok(claudeTurn("bla\n", T - 500, 0, T).status === "working", "clignotement du footer ≠ fin");
  ok(claudeTurn("bla\n", T - GRACE_MS - 1, 0, T).status === "done", "marqueur parti → done");
  // …mais pas si tu viens d'appuyer sur Entrée (le tour repart)
  ok(claudeTurn("bla\n", T - 9999, T - 100, T).status === "working", "envoi récent ≠ done");

  // menu de choix → waiting, et la grâce est repoussée pour ne pas conclure
  // « done » entre la réponse et la reprise du footer.
  const m = claudeTurn("❯ 1. Yes\n  2. No\n", T - 9999, 0, T);
  ok(m.status === "waiting" && m.seen === T, "menu → waiting + grâce repoussée");
  ok(claudeTurn("Do you want to proceed?\n", 0, 0, T).status === "waiting", "question → waiting");
  // un menu sur un pane qui n'a jamais exécuté Claude n'arme pas la fin de tour
  ok(claudeTurn("❯ 1. Yes\n  2. No\n", 0, 0, T).seen === 0, "menu seul n'arme pas done");
}
