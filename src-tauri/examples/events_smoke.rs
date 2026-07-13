//! Smoke test du watcher d'événements (tail -F via canal SSH, même plomberie qu'events_watch).
//! Usage : cargo run --example events_smoke -- <host> <port> <user> <key_path> <scratch_dir>

use russh::ChannelMsg;
use arabel_lib::ssh::{connect_auth, exec};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, key_path, scratch) =
        (&args[1], args[2].parse::<u16>().unwrap(), &args[3], &args[4], &args[5]);
    let log = format!("{scratch}/events.jsonl");

    let handle = connect_auth(host, port, user, key_path, None, None).await.unwrap();
    let mut ch = handle.channel_open_session().await.unwrap();
    ch.exec(true, format!("mkdir -p {scratch} && touch {log} && exec tail -F -n0 {log}"))
        .await
        .unwrap();

    // laisse tail démarrer puis ajoute une ligne via une 2e connexion exec
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let line = r#"{"pane":"p1","event":{"hook_event_name":"Notification","message":"test"}}"#;
    let (st, out) = exec(&handle, &format!("echo '{line}' >> {log}"), None).await.unwrap();
    assert_eq!(st, 0, "append: {out}");

    let mut buf = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let msg = tokio::time::timeout_at(deadline, ch.wait()).await.expect("timeout: rien reçu du tail");
        match msg {
            Some(ChannelMsg::Data { ref data }) => {
                buf.push_str(&String::from_utf8_lossy(data));
                if buf.contains("hook_event_name") {
                    break;
                }
            }
            None => panic!("canal fermé avant réception"),
            _ => {}
        }
    }
    assert!(buf.contains(r#""pane":"p1""#), "ligne inattendue: {buf}");
    println!("EVENTS SMOKE OK — tail -F streamé et ligne de hook reçue");
}
