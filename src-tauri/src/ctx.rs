//! Bandeau « contexte & quotas » : d'où viennent les chiffres.
//!
//! Claude Code n'expose ni le contexte consommé ni ses fenêtres de quota par un
//! fichier ou une commande. Le seul endroit où les trois existent ensemble est
//! le JSON qu'il passe sur stdin à la **status line** configurée, à chaque
//! rendu :
//!
//! ```json
//! { "context_window": { "used_percentage": 38, "context_window_size": 200000, … },
//!   "rate_limits": { "five_hour":  { "used_percentage": 12.4, "resets_at": 1755630000 },
//!                    "seven_day":  { "used_percentage": 31.0, "resets_at": 1756000000 } } }
//! ```
//!
//! On s'y branche donc : `~/.arabel/ctx.sh` devient la status line, dépose une
//! copie du JSON dans `~/.arabel/ctx/<pane>.json`, puis **rend la main à celle
//! que tu avais déjà** (mise de côté dans `~/.arabel/statusline.inner`). Le
//! terminal ne perd rien, et l'app n'a plus qu'à relire un petit fichier.
//!
//! Ce module tient la moitié locale (ta machine) ; la moitié distante est dans
//! `ssh.rs`, qui pousse le même script par SSH.
//!
//! `context_window` et `rate_limits` sont apparus dans ce JSON au fil des
//! versions de Claude Code : si celle qui tourne ne les envoie pas, le relais
//! écrit quand même son fichier, et le bandeau reste simplement masqué.
//! `rate_limits` manque aussi tant que la session n'a pas reçu sa première
//! réponse d'API — les quotas viennent des en-têtes de réponse.

use std::path::{Path, PathBuf};

/// Commande inscrite dans `settings.json`. `$HOME` est développé par le shell
/// qui lance la status line — même convention que le hook de `ssh.rs`.
pub const STATUSLINE_CMD: &str = "$HOME/.arabel/ctx.sh";

/// Le relais. Il doit rester en sh POSIX : il tourne aussi bien sur ton Mac que
/// sur un VPS qui n'a que dash.
pub const CTX_SH: &str = r#"#!/bin/sh
# arabel — relais de status line Claude Code.
# Garde une copie du JSON de status line pour l'app (contexte + quotas 5 h / 7 j),
# puis rend la main à la status line que tu avais déjà, si tu en avais une.
in=$(cat)
d="$HOME/.arabel/ctx"
p=${ARABEL_PANE:-unknown}
mkdir -p "$d" 2>/dev/null
# Une ligne par fichier : `tr` aplatit un JSON éventuellement indenté. Sans
# risque — une chaîne JSON ne contient jamais de saut de ligne littéral.
# Écriture puis `mv` : l'app ne peut pas lire un fichier à moitié écrit.
printf '{"pane":"%s","at":%s,"ctx":%s}' "$p" "$(date +%s)" "$in" | tr -d '\n' > "$d/.$p.tmp" 2>/dev/null &&
  mv "$d/.$p.tmp" "$d/$p.json" 2>/dev/null
inner="$HOME/.arabel/statusline.inner"
if [ -s "$inner" ]; then
  printf '%s' "$in" | sh -c "$(cat "$inner")"
else
  # Pas de status line auparavant : on en imprime une minimale plutôt que rien
  # (la configurer la remplace, on ne veut pas te laisser une ligne vide). Les
  # compteurs, eux, sont dans le bandeau d'arabel — inutile de les redire ici.
  m=$(printf '%s' "$in" | sed -n 's/.*"display_name":"\([^"]*\)".*/\1/p')
  c=$(printf '%s' "$in" | sed -n 's/.*"current_dir":"\([^"]*\)".*/\1/p')
  printf '\033[2m%s  %s\033[0m' "$m" "${c##*/}"
fi
"#;

/// Script python qui branche (ou débranche) le relais dans `~/.claude/settings.json`.
/// Partagé avec le côté SSH, qui l'exécute sur le VPS via `python3 -`.
///
/// Le prélude doit définir `ENABLE` (bool) et `OURS` (str).
pub const MERGE_PY: &str = r#"
import json, os
p = os.path.expanduser("~/.claude/settings.json")
try:
    cfg = json.load(open(p))
except Exception:
    cfg = {}
inner = os.path.expanduser("~/.arabel/statusline.inner")
sl = cfg.get("statusLine") or {}
cur = sl.get("command")
changed = False
if ENABLE:
    # ta status line à toi est mise de côté, pas écrasée : ctx.sh la rappelle.
    if cur and cur != OURS:
        os.makedirs(os.path.dirname(inner), exist_ok=True)
        open(inner, "w").write(cur)
    if cur != OURS:
        sl["type"] = "command"; sl["command"] = OURS
        cfg["statusLine"] = sl; changed = True
elif cur == OURS:
    prev = ""
    try:
        prev = open(inner).read().strip()
    except Exception:
        pass
    if prev:
        sl["command"] = prev; cfg["statusLine"] = sl
    else:
        cfg.pop("statusLine", None)
    try:
        os.remove(inner)
    except Exception:
        pass
    changed = True
os.makedirs(os.path.dirname(p), exist_ok=True)
json.dump(cfg, open(p, "w"), indent=2)
print("changed" if changed else "unchanged")
"#;

fn arabel_dir() -> Result<PathBuf, String> {
    Ok(Path::new(&crate::home_dir()?).join(".arabel"))
}

/// Écrit `~/.arabel/ctx.sh` et le rend exécutable (Unix).
fn write_script(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("~/.arabel: {e}"))?;
    let p = dir.join("ctx.sh");
    std::fs::write(&p, CTX_SH).map_err(|e| format!("ctx.sh: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod ctx.sh: {e}"))?;
    }
    Ok(())
}

/// Branche (ou débranche) le relais dans le `~/.claude/settings.json` LOCAL.
/// Idempotent : rappelable à chaque démarrage sans effet de bord.
///
/// Symétrie exacte avec le python distant, en Rust pour ne rien exiger de la
/// machine de l'utilisateur (un Mac n'a pas forcément python3).
#[tauri::command]
pub fn ctx_setup(enable: bool) -> Result<String, String> {
    setup_in(Path::new(&crate::home_dir()?), enable)
}

/// Le corps de `ctx_setup`, paramétré par le home — testable sans toucher à
/// l'environnement du process.
fn setup_in(home: &Path, enable: bool) -> Result<String, String> {
    let dir = home.join(".arabel");
    let inner = dir.join("statusline.inner");
    let settings = home.join(".claude/settings.json");
    let mut cfg: serde_json::Value = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !cfg.is_object() {
        cfg = serde_json::json!({});
    }
    let cur = cfg
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .map(str::to_string);

    let changed = if enable {
        write_script(&dir)?;
        if let Some(prev) = cur.as_deref().filter(|c| *c != STATUSLINE_CMD) {
            std::fs::write(&inner, prev).map_err(|e| format!("statusline.inner: {e}"))?;
        }
        if cur.as_deref() != Some(STATUSLINE_CMD) {
            let sl = cfg
                .as_object_mut()
                .unwrap()
                .entry("statusLine")
                .or_insert_with(|| serde_json::json!({}));
            if !sl.is_object() {
                *sl = serde_json::json!({});
            }
            sl["type"] = serde_json::json!("command");
            sl["command"] = serde_json::json!(STATUSLINE_CMD);
            true
        } else {
            false
        }
    } else if cur.as_deref() == Some(STATUSLINE_CMD) {
        // on ne retire QUE le nôtre, et on rend la sienne à l'utilisateur
        let prev = std::fs::read_to_string(&inner)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        if prev.is_empty() {
            cfg.as_object_mut().unwrap().remove("statusLine");
        } else {
            cfg["statusLine"]["command"] = serde_json::json!(prev);
        }
        let _ = std::fs::remove_file(&inner);
        true
    } else {
        false
    };

    if changed {
        if let Some(parent) = settings.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("~/.claude: {e}"))?;
        }
        std::fs::write(
            &settings,
            serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("settings.json: {e}"))?;
    }
    Ok(if changed {
        if enable { "status line relay installed" } else { "status line relay removed" }.into()
    } else {
        "already up to date".to_string()
    })
}

/// Les derniers JSON déposés par le relais sur CETTE machine, un par pane.
/// Best-effort : un répertoire absent = personne n'a encore lancé claude ici.
///
/// Les fichiers plus vieux qu'un jour sont supprimés au passage : ils portent
/// l'id d'un pane d'une session d'app révolue, plus rien ne les relira.
#[tauri::command]
pub fn ctx_read() -> Vec<String> {
    let dir = match arabel_dir() {
        Ok(d) => d.join("ctx"),
        Err(_) => return vec![],
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };
    let day = std::time::Duration::from_secs(86_400);
    let mut out = vec![];
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|d| d > day).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        if let Ok(txt) = std::fs::read_to_string(&path) {
            if !txt.trim().is_empty() {
                out.push(txt);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("arabel-ctx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".claude")).unwrap();
        d
    }
    fn settings(home: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap()).unwrap()
    }
    fn cmd(home: &Path) -> Option<String> {
        settings(home)["statusLine"]["command"].as_str().map(str::to_string)
    }

    /// Le cas qui fait mal : l'utilisateur avait DÉJÀ une status line. On doit la
    /// mettre de côté, pas la perdre — et la lui rendre quand il décoche.
    #[test]
    fn keeps_and_gives_back_an_existing_status_line() {
        let home = tmp("chain");
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"bun ~/line.ts","padding":0},"model":"opus"}"#,
        )
        .unwrap();

        setup_in(&home, true).unwrap();
        assert_eq!(cmd(&home).as_deref(), Some(STATUSLINE_CMD));
        assert_eq!(
            std::fs::read_to_string(home.join(".arabel/statusline.inner")).unwrap(),
            "bun ~/line.ts"
        );
        // le reste du fichier de l'utilisateur est intact
        assert_eq!(settings(&home)["model"], "opus");
        assert_eq!(settings(&home)["statusLine"]["padding"], 0);
        // idempotent : réinstaller ne doit pas écraser la sienne par la nôtre
        setup_in(&home, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(".arabel/statusline.inner")).unwrap(),
            "bun ~/line.ts"
        );

        setup_in(&home, false).unwrap();
        assert_eq!(cmd(&home).as_deref(), Some("bun ~/line.ts"));
        assert!(!home.join(".arabel/statusline.inner").exists());
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Sans status line au départ : on pose la nôtre, et décocher ne laisse rien.
    #[test]
    fn adds_then_removes_cleanly() {
        let home = tmp("clean");
        setup_in(&home, true).unwrap();
        assert_eq!(cmd(&home).as_deref(), Some(STATUSLINE_CMD));
        assert!(home.join(".arabel/ctx.sh").exists());

        setup_in(&home, false).unwrap();
        assert!(settings(&home).get("statusLine").is_none());
        // décocher deux fois de suite ne doit rien casser
        setup_in(&home, false).unwrap();
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Une status line posée par quelqu'un d'autre APRÈS coup ne doit pas être
    /// balayée : on ne retire que la nôtre.
    #[test]
    fn leaves_a_foreign_status_line_alone() {
        let home = tmp("foreign");
        std::fs::write(
            home.join(".claude/settings.json"),
            r#"{"statusLine":{"type":"command","command":"my-line"}}"#,
        )
        .unwrap();
        setup_in(&home, false).unwrap();
        assert_eq!(cmd(&home).as_deref(), Some("my-line"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
