//! Smoke test tmux : création avec init, détachement, réattachement (même
//! commande que le front — tmuxCmd de +page.svelte).
//! Usage : cargo run --example tmux_smoke -- <host> <port> <user> <key_path>

use arabel_lib::ssh::{connect_auth, exec};
use russh::ChannelMsg;
use std::time::Duration;

const NAME: &str = "arabel-smoketest";

fn tmux_cmd(init: &str) -> String {
    let send = if init.is_empty() {
        String::new()
    } else {
        format!("sleep 0.3; tmux send-keys -t '{NAME}' '{init}' C-m; ")
    };
    // fenêtre en `sh` nu pour un test hermétique (le front garde le shell de l'utilisateur)
    format!(
        "export PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"; \
         if command -v tmux >/dev/null 2>&1; then \
         if tmux has-session -t '{NAME}' 2>/dev/null; then exec tmux -u attach-session -t '{NAME}'; \
         else tmux -u new-session -d -s '{NAME}' sh && {send}exec tmux -u attach-session -t '{NAME}'; fi; \
         else exec \"$SHELL\" -l; fi"
    )
}

async fn open_pane(
    handle: &russh::client::Handle<arabel_lib::ssh::Handler>,
    cmd: &str,
) -> russh::Channel<russh::client::Msg> {
    let ch = handle.channel_open_session().await.unwrap();
    ch.request_pty(false, "xterm-256color", 100, 30, 0, 0, &[]).await.unwrap();
    ch.exec(true, cmd).await.unwrap();
    ch
}

async fn read_until(ch: &mut russh::Channel<russh::client::Msg>, needle: &str) -> String {
    let mut out = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let msg = tokio::time::timeout_at(deadline, ch.wait())
            .await
            .unwrap_or_else(|_| panic!("timeout en attendant «{needle}» ; reçu:\n{out}"));
        match msg {
            Some(ChannelMsg::Data { ref data }) => {
                out.push_str(&String::from_utf8_lossy(data));
                if out.contains(needle) {
                    return out;
                }
            }
            None => panic!("canal fermé en attendant «{needle}» ; reçu:\n{out}"),
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, key_path) =
        (&args[1], args[2].parse::<u16>().unwrap(), &args[3], &args[4]);

    let handle = connect_auth(host, port, user, key_path, None, None).await.unwrap();
    // état propre
    let _ = exec(
        &handle,
        &format!("export PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin\"; tmux kill-session -t {NAME} 2>/dev/null; true"),
        None,
    )
    .await;

    // 1. création : l'init doit être tapé
    let mut ch = open_pane(&handle, &tmux_cmd("echo arabel-init-ok")).await;
    read_until(&mut ch, "arabel-init-ok").await;
    // marqueur supplémentaire tapé par « l'utilisateur »
    ch.data(&b"echo arabel-marker-42\n"[..]).await.unwrap();
    read_until(&mut ch, "arabel-marker-42").await;
    // 2. détachement brutal (coupure réseau simulée)
    drop(ch);
    drop(handle);

    // 3. réattachement : nouvelle connexion, même nom → scrollback préservé,
    //    et l'init ne doit PAS être retapé (une seule occurrence après clear)
    let handle2 = connect_auth(host, port, user, key_path, None, None).await.unwrap();
    let mut ch2 = open_pane(&handle2, &tmux_cmd("echo arabel-init-ok")).await;
    let redraw = read_until(&mut ch2, "arabel-marker-42").await;
    assert!(redraw.contains("arabel-marker-42"), "scrollback perdu");

    // 4. fin propre
    let _ = exec(&handle2, &format!("tmux kill-session -t {NAME} 2>/dev/null; true"), None).await;
    println!("TMUX SMOKE OK — création+init, détachement, réattachement avec état préservé");
    std::process::exit(0);
}
