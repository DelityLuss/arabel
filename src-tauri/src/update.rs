//! Détection de nouvelle version.
//!
//! Pas d'auto-update ici, et c'est délibéré : remplacer le binaire en place
//! demande une paire de clés de signature (minisign) côté `tauri-plugin-updater`,
//! donc un secret dans la CI et un `pubkey` dans `tauri.conf.json`. Tant que ça
//! n'existe pas, l'app se contente de LIRE la dernière release publiée sur
//! GitHub et de te le dire ; le téléchargement reste un clic vers la page de
//! release, où tu prends le paquet de ton OS. Rien n'est installé sans toi.
//!
//! Le sondage se fait côté Rust plutôt qu'en `fetch` depuis la webview : pas de
//! CORS ni de CSP à négocier, un vrai `User-Agent` (l'API GitHub refuse les
//! requêtes qui n'en ont pas), et un timeout court qu'on maîtrise.

use std::time::Duration;

/// Dépôt qui publie les releases. Même valeur que le `origin` du projet.
const REPO: &str = "DelityLuss/arabel";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Version de la release, sans le `v` du tag (`0.6.3`).
    pub version: String,
    /// Version qui tourne ici — pratique pour l'afficher côte à côte.
    pub current: String,
    /// Page de la release : c'est là qu'on envoie l'utilisateur.
    pub url: String,
    /// Corps de la release (markdown brut), tronqué : le bandeau n'en montre
    /// que les premières lignes.
    pub notes: String,
    /// Date de publication ISO 8601, telle que GitHub la donne.
    pub published_at: String,
}

/// Version en triplet + « est-ce une pré-version ». `v0.6.3-beta.1` →
/// `([0,6,3], true)`. Un composant illisible vaut 0 : on ne veut pas qu'un tag
/// exotique fasse échouer la comparaison, juste qu'il ne se fasse pas passer
/// pour plus récent.
fn parse(v: &str) -> ([u32; 3], bool) {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut out = [0u32; 3];
    for (i, part) in core.split('.').take(3).enumerate() {
        out[i] = part.parse().unwrap_or(0);
    }
    (out, pre)
}

/// `remote` est-il strictement plus récent que `local` ? À triplet égal, une
/// pré-version perd contre la version finale (0.7.0-rc1 < 0.7.0).
fn is_newer(remote: &str, local: &str) -> bool {
    let (r, r_pre) = parse(remote);
    let (l, l_pre) = parse(local);
    match r.cmp(&l) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => l_pre && !r_pre,
    }
}

/// Interroge la dernière release publiée et renvoie `Some` seulement si elle
/// est plus récente que le binaire en cours. `None` = déjà à jour.
///
/// L'API `releases/latest` ignore d'elle-même les brouillons et les
/// pré-releases : le workflow de release crée un brouillon, donc rien ne sort
/// d'ici tant que tu ne l'as pas publié à la main.
#[tauri::command]
pub async fn update_check(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let client = reqwest::Client::builder()
        // court : c'est un sondage d'arrière-plan, il n'a le droit de gêner personne.
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", format!("arabel/{current}"))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        // 404 = aucune release publiée (que des brouillons) : ce n'est pas une
        // panne, on répond « rien de neuf ». 403 = quota anonyme de l'API épuisé.
        let code = res.status();
        if code == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        return Err(format!("GitHub answered {code}"));
    }
    // `res.json()` demanderait la feature `json` de reqwest, qu'on n'active pas
    // pour un seul appel : le texte + serde_json fait exactement pareil.
    let text = res.text().await.map_err(|e| e.to_string())?;
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = body
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("release without a tag")?;
    if !is_newer(tag, &current) {
        return Ok(None);
    }
    let notes = body.get("body").and_then(|v| v.as_str()).unwrap_or("");
    Ok(Some(UpdateInfo {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        current,
        url: body
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://github.com/{REPO}/releases/latest")),
        notes: notes.chars().take(600).collect(),
        published_at: body
            .get("published_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compare_versions() {
        assert!(is_newer("v0.6.3", "0.6.2"));
        assert!(is_newer("0.7.0", "0.6.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.6.2", "0.6.2"));
        assert!(!is_newer("0.6.1", "0.6.2"));
        // une pré-version ne remplace pas la finale du même numéro, mais la
        // finale remplace bien la pré-version
        assert!(!is_newer("0.7.0-rc.1", "0.7.0"));
        assert!(is_newer("0.7.0", "0.7.0-rc.1"));
        // tag illisible : jamais « plus récent »
        assert!(!is_newer("nightly", "0.6.2"));
    }
}
