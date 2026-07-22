use crate::ssh::{Cmd, SshState};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

/// Métriques de la machine locale (load, CPU, RAM, disque) — même forme que
/// la ligne "M …" des métriques distantes, mais en JSON direct.
#[tauri::command]
pub fn local_metrics() -> serde_json::Value {
    use sysinfo::{Disks, System};
    let mut sys = System::new();
    sys.refresh_memory();
    // ponytail: la load average n'existe pas sur Windows (sysinfo renvoie 0) —
    // concept Unix, on l'accepte tel quel plutôt que d'échantillonner le CPU.
    let load = System::load_average().one;
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let disks = Disks::new_with_refreshed_list();
    // disque contenant le home : racine "/" sur Unix, "C:\" (ou autre) sur Windows.
    // On prend le point de montage le plus spécifique qui préfixe le home.
    let root = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
    let (disk_total, disk_used) = disks
        .list()
        .iter()
        .filter(|d| root.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| (d.total_space(), d.total_space() - d.available_space()))
        .unwrap_or((0, 0));
    serde_json::json!({
        "load": load,
        "cpus": cpus,
        "memTotal": sys.total_memory(),
        "memUsed": sys.used_memory(),
        "diskTotal": disk_total,
        "diskUsed": disk_used,
    })
}

/// pid du process shell de chaque session PTY locale, pour lire son cwd (panneau
/// git d'un terminal local). Rempli par `spawn_in_pty`, purgé à la fermeture.
#[derive(Default)]
pub struct PtyPids(pub std::sync::Mutex<std::collections::HashMap<String, u32>>);

fn size(cols: u32, rows: u32) -> PtySize {
    PtySize {
        rows: rows as u16,
        cols: cols as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Spawne une commande dans un PTY et la câble aux mêmes events/commandes que
/// SSH (`ssh-output-*` / `ssh-closed-*`, SshState). Partagé par local + mosh.
async fn spawn_in_pty(
    app: AppHandle,
    state: &SshState,
    session_id: String,
    cols: u32,
    rows: u32,
    cmd: CommandBuilder,
) -> Result<(), String> {
    let pty = native_pty_system();
    let pair = pty.openpty(size(cols, rows)).map_err(|e| e.to_string())?;
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    // pid du shell → cwd lisible plus tard (local_cwd, panneau git)
    if let Some(pid) = child.process_id() {
        app.state::<PtyPids>().0.lock().unwrap().insert(session_id.clone(), pid);
    }

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let master = pair.master;

    let (tx, mut rx) = mpsc::channel::<Cmd>(64);
    state.0.lock().await.insert(session_id.clone(), tx);

    // lecture bloquante → thread dédié
    let out_event = format!("ssh-output-{session_id}");
    let closed_event = format!("ssh-closed-{session_id}");
    let app_r = app.clone();
    let sid = session_id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = app_r.emit(&out_event, B64.encode(&buf[..n]));
                }
            }
        }
        let _ = app_r.emit(&closed_event, ());
        app_r.state::<PtyPids>().0.lock().unwrap().remove(&sid);
        tauri::async_runtime::block_on(async {
            app_r.state::<SshState>().0.lock().await.remove(&sid);
        });
    });

    // contrôle write/resize/close
    tauri::async_runtime::spawn(async move {
        while let Some(c) = rx.recv().await {
            match c {
                Cmd::Write(bytes) => {
                    let _ = writer.write_all(&bytes);
                }
                Cmd::Resize(cols, rows) => {
                    let _ = master.resize(size(cols, rows));
                }
                Cmd::Close => break,
            }
        }
        let _ = child.kill(); // le reader voit EOF et émet ssh-closed
    });

    Ok(())
}

/// Décode la sortie de `wsl.exe -l -q` : de l'UTF-16LE, une distro par ligne.
/// Testable hors Windows — c'est la seule vraie logique de `wsl_distros`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_distros(out: &[u8]) -> Vec<String> {
    let u16s: Vec<u16> = out.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u16s)
        .lines()
        // wsl.exe termine chaque nom par un \r ; trim() l'enlève avec le BOM éventuel
        .map(|l| l.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Distributions WSL installées (vide hors Windows / si WSL absent).
#[tauri::command]
pub fn wsl_distros() -> Vec<String> {
    #[cfg(not(windows))]
    return Vec::new();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // -l -q : noms seuls, sans la mention "(par défaut)".
        // CREATE_NO_WINDOW (0x0800_0000) évite un flash de console.
        match std::process::Command::new("wsl.exe")
            .args(["-l", "-q"])
            .creation_flags(0x0800_0000)
            .output()
        {
            Ok(o) if o.status.success() => parse_distros(&o.stdout),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_distros;

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    // Le cœur physique de local_cwd : sysinfo lit-il le cwd d'un AUTRE process du
    // même utilisateur ? (macOS : proc_pidinfo sans root ; Linux : /proc/pid/cwd)
    #[cfg(not(windows))]
    #[test]
    fn reads_foreign_process_cwd() {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .current_dir("/tmp")
            .spawn()
            .expect("spawn sleep");
        let pid = Pid::from_u32(child.id());
        let mut sys = System::new();
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
        );
        let cwd = sys.process(pid).and_then(|p| p.cwd()).map(|c| c.display().to_string());
        let _ = child.kill();
        // macOS résout /tmp → /private/tmp : on tolère le suffixe.
        assert!(cwd.as_deref().is_some_and(|c| c.ends_with("/tmp")), "cwd lu = {cwd:?}");
    }

    #[test]
    fn parses_wsl_output() {
        // ce que `wsl.exe -l -q` écrit réellement : UTF-16LE, lignes en \r\n
        assert_eq!(
            parse_distros(&utf16le("Ubuntu\r\nDebian\r\n")),
            vec!["Ubuntu".to_string(), "Debian".to_string()]
        );
        // BOM en tête + ligne vide finale
        assert_eq!(parse_distros(&utf16le("\u{feff}Ubuntu-22.04\r\n\r\n")), vec!["Ubuntu-22.04".to_string()]);
        // WSL absent → sortie vide, et jamais de panique sur un octet orphelin
        assert!(parse_distros(b"").is_empty());
        assert!(parse_distros(b"\x00").is_empty());
    }
}

/// Terminal local : shell de l'utilisateur dans un PTY, mêmes events et mêmes
/// commandes write/resize/disconnect que les sessions SSH (via SshState).
/// `distro` non vide → shell de login de cette distribution WSL (Windows only).
#[tauri::command]
pub async fn local_connect(
    app: AppHandle,
    state: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
    distro: Option<String>,
) -> Result<(), String> {
    #[cfg(not(windows))]
    let _ = distro; // WSL n'existe que sous Windows
    // Windows n'a pas de $SHELL ni de flag `-l` : PowerShell, présent partout.
    #[cfg(windows)]
    let mut cmd = {
        let mut c = match distro.filter(|d| !d.is_empty()) {
            // `wsl.exe -d <distro>` lance le shell de login de la distro. Le cwd
            // Windows plus bas ne s'y applique pas : wsl traduirait le chemin en
            // /mnt/c/… alors qu'on veut le $HOME Linux → il gère seul son cwd.
            Some(d) => {
                let mut c = CommandBuilder::new("wsl.exe");
                c.args(["-d", d.as_str(), "--cd", "~"]);
                c
            }
            None => CommandBuilder::new("powershell.exe"),
        };
        // portable-pty démarre avec un environnement VIDE. Sans PATH, le chemin
        // relatif "powershell.exe" est introuvable → le terminal local ne
        // démarrait pas sous Windows. On hérite de l'env du process parent
        // (comme un vrai terminal), ce qui fournit aussi USERPROFILE, APPDATA,
        // PSModulePath… nécessaires au bon fonctionnement de PowerShell.
        for (k, v) in std::env::vars_os() {
            c.env(k, v);
        }
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let mut c = CommandBuilder::new(&shell);
        c.arg("-l"); // shell de login
        c
    };
    cmd.env("TERM", "xterm-256color");
    if let Some(home) = dirs::home_dir() {
        cmd.cwd(home);
    }
    spawn_in_pty(app, &state, session_id, cols, rows, cmd).await
}

/// Cwd du shell d'une session locale, pour ouvrir le panneau git là où le terminal
/// se trouve (et non au home). Lit le cwd du pid via sysinfo — même utilisateur que
/// l'app, donc autorisé sans root sur macOS/Linux. None si le pid est inconnu ou
/// illisible → l'appelant retombe sur le home.
#[tauri::command]
pub fn local_cwd(pids: State<'_, PtyPids>, session_id: String) -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let pid = Pid::from_u32(*pids.0.lock().unwrap().get(&session_id)?);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );
    sys.process(pid).and_then(|p| p.cwd()).map(|c| c.display().to_string())
}

/// `git -C cwd args` sur la machine locale : pendant de `sftp::git_run` (SSH) pour
/// le panneau source control d'un terminal local. cwd vide/`~` → home.
// ponytail: une app GUI macOS démarre avec un PATH minimal — on ajoute les
// préfixes Homebrew (comme mosh_connect) pour trouver git s'il n'est pas dans /usr/bin.
#[tauri::command]
pub async fn git_run_local(cwd: String, args: Vec<String>) -> Result<(u32, String), String> {
    let cwd = if cwd.is_empty() || cwd == "~" {
        crate::home_dir().unwrap_or_else(|_| ".".into())
    } else {
        cwd
    };
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&cwd)
        .args(&args)
        .env(
            "PATH",
            format!("/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{}", std::env::var("PATH").unwrap_or_default()),
        )
        .output()
        .await
        .map_err(|e| e.to_string())?;
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(1) as u32, s))
}

/// Transport « ssh système » : lance le binaire `ssh` d'OpenSSH dans un PTY, au
/// lieu de la pile russh interne. On délègue TOUT à OpenSSH → compatibilité
/// parfaite (tous formats de clé, ~/.ssh/config, ssh-agent, ProxyJump,
/// known_hosts, passphrase saisie dans le terminal). En échange : pas de SFTP /
/// forwards / métriques / tmux -CC (qui vivent sur le canal de contrôle russh).
#[tauri::command]
pub async fn ssh_pty_connect(
    app: AppHandle,
    state: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    auth: Option<String>,
    exec_cmd: Option<String>,
) -> Result<(), String> {
    let mut cmd = CommandBuilder::new("ssh");
    // hériter l'env : PATH (trouver ssh), SSH_AUTH_SOCK (agent), HOME (~/.ssh/config)
    for (k, v) in std::env::vars_os() {
        cmd.env(k, v);
    }
    cmd.arg("-p");
    cmd.arg(port.to_string());
    cmd.arg("-t"); // force un PTY distant (nécessaire pour tmux / TUI même avec une commande)
    // TOFU auto (comme russh) + keepalive pour détecter les coupures
    for o in [
        "StrictHostKeyChecking=accept-new",
        "ServerAliveInterval=15",
        "ServerAliveCountMax=3",
    ] {
        cmd.arg("-o");
        cmd.arg(o);
    }
    if auth.as_deref() == Some("key") && !key_path.is_empty() {
        cmd.arg("-i");
        cmd.arg(crate::ssh::expand_tilde(&key_path));
        cmd.arg("-o");
        cmd.arg("IdentitiesOnly=yes");
    }
    cmd.arg(format!("{user}@{host}"));
    if let Some(c) = exec_cmd.filter(|c| !c.is_empty()) {
        cmd.arg(c); // commande distante (snippet tmux) exécutée via le shell distant
    }
    cmd.env("TERM", "xterm-256color");
    spawn_in_pty(app, &state, session_id, cols, rows, cmd).await
}

/// Localise le binaire `mosh` (une app GUI macOS n'a pas Homebrew dans son PATH).
fn find_mosh() -> Option<std::path::PathBuf> {
    let mut dirs = vec!["/opt/homebrew/bin".to_string(), "/usr/local/bin".to_string()];
    if let Ok(p) = std::env::var("PATH") {
        dirs.extend(p.split(':').map(str::to_string));
    }
    dirs.into_iter()
        .map(|d| std::path::Path::new(&d).join("mosh"))
        .find(|p| p.exists())
}

/// Indique si le transport mosh est utilisable (binaire présent, hors Windows).
#[tauri::command]
pub fn mosh_available() -> bool {
    !cfg!(windows) && find_mosh().is_some()
}

/// Transport mosh : lance `mosh` en local dans un PTY. mosh gère le bootstrap
/// SSH, l'UDP, l'écho prédictif et la reprise de session après coupure ; son
/// rendu (séquences d'échappement) est simplement affiché par xterm. Unix only.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn mosh_connect(
    app: AppHandle,
    state: State<'_, SshState>,
    session_id: String,
    cols: u32,
    rows: u32,
    host: String,
    port: u16,
    user: String,
    key_path: String,
    auth: Option<String>,
    exec_cmd: Option<String>,
) -> Result<(), String> {
    if auth.as_deref() == Some("password") {
        return Err("mosh doesn't support password auth (the bootstrap ssh can't enter it) — use a key, ssh-agent, or the SSH transport".into());
    }
    let mosh = find_mosh().ok_or("mosh introuvable localement (brew install mosh)")?;
    let bin_dir = mosh.parent().map(|p| p.display().to_string()).unwrap_or_default();

    // ssh de bootstrap : port + TOFU (accept-new, comme russh) + clé si fournie
    let mut ssh = format!("ssh -p {port} -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10");
    if auth.as_deref() != Some("agent") && !key_path.is_empty() {
        ssh.push_str(&format!(" -i {key_path}"));
    }

    let mut cmd = CommandBuilder::new(mosh.as_os_str());
    cmd.arg(format!("--ssh={ssh}"));
    cmd.arg(format!("{user}@{host}"));
    if let Some(c) = exec_cmd.filter(|c| !c.is_empty()) {
        // le snippet tmux est du shell : on l'exécute via un shell de login distant.
        // mosh transmet l'argv structurellement (pas de ré-échappement à gérer).
        cmd.arg("--");
        cmd.arg("sh");
        cmd.arg("-lc");
        cmd.arg(c);
    }
    cmd.env("TERM", "xterm-256color");
    // le wrapper mosh doit trouver mosh-client (à côté de lui) + ssh
    cmd.env(
        "PATH",
        format!("{bin_dir}:/usr/bin:/bin:{}", std::env::var("PATH").unwrap_or_default()),
    );
    // mosh exige un locale UTF-8 ; une app GUI macOS hérite souvent d'un env sans
    // LANG → fallback UTF-8 uniquement si l'env courant n'en fournit pas déjà un
    let has_utf8 = ["LC_ALL", "LANG", "LC_CTYPE"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .any(|v| v.to_uppercase().replace('-', "").contains("UTF8"));
    if !has_utf8 {
        cmd.env("LANG", "en_US.UTF-8");
    }
    spawn_in_pty(app, &state, session_id, cols, rows, cmd).await
}
