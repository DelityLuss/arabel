//! Smoke test de la plomberie SSH (même séquence que src/ssh.rs).
//! Usage : cargo run --example ssh_smoke -- <host> <port> <user> <key_path>

use russh::keys::{load_secret_key, PrivateKeyWithHashAlg};
use russh::{client, ChannelMsg};
use std::sync::Arc;

struct Handler;
impl client::Handler for Handler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        _key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, key_path) = (&args[1], args[2].parse::<u16>().unwrap(), &args[3], &args[4]);

    let key = load_secret_key(key_path, None).expect("clé");
    let mut handle = client::connect(Arc::new(client::Config::default()), (host.as_str(), port), Handler)
        .await
        .expect("connexion");
    let hash = handle.best_supported_rsa_hash().await.unwrap().flatten();
    let auth = handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .expect("auth");
    assert!(auth.success(), "authentification refusée");

    let mut channel = handle.channel_open_session().await.expect("channel");
    channel
        .request_pty(false, "xterm-256color", 80, 24, 0, 0, &[])
        .await
        .expect("pty");
    channel.request_shell(true).await.expect("shell");

    channel.data(&b"stty size; echo arabel-ok; exit\n"[..]).await.expect("write");
    channel.window_change(120, 40, 0, 0).await.expect("resize");

    let mut output = String::new();
    while let Some(msg) = channel.wait().await {
        match msg {
            ChannelMsg::Data { ref data } => output.push_str(&String::from_utf8_lossy(data)),
            ChannelMsg::ExitStatus { .. } | ChannelMsg::Close => break,
            _ => {}
        }
    }
    // le window_change part avant que le shell n'exécute stty → on doit voir 40 120
    assert!(output.contains("40 120"), "resize non appliqué:\n{output}");
    assert!(output.contains("arabel-ok"), "echo manquant:\n{output}");
    println!("SMOKE OK — connexion, auth, pty, shell, echo et resize passés");
}
