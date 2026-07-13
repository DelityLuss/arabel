//! Smoke test SFTP : subsystem, canonicalize, upload, listing, download (même
//! plomberie que src/sftp.rs).
//! Usage : cargo run --example sftp_smoke -- <host> <port> <user> <key_path> <scratch_dir>

use arabel_lib::ssh::connect_auth;
use russh_sftp::client::SftpSession;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, key_path, scratch) =
        (&args[1], args[2].parse::<u16>().unwrap(), &args[3], &args[4], &args[5]);
    std::fs::create_dir_all(scratch).unwrap();

    let handle = connect_auth(host, port, user, key_path, None, None).await.unwrap();
    let ch = handle.channel_open_session().await.unwrap();
    ch.request_subsystem(true, "sftp").await.unwrap();
    let sftp = SftpSession::new(ch.into_stream()).await.unwrap();

    let home = sftp.canonicalize(".").await.unwrap();
    assert!(home.starts_with('/'), "home non absolu: {home}");

    // upload
    let remote_path = format!("{scratch}/sftp-up.bin");
    let payload = b"arabel-sftp-0123456789".repeat(1000);
    let mut f = sftp.create(&remote_path).await.unwrap();
    f.write_all(&payload).await.unwrap();
    f.shutdown().await.unwrap();

    // listing
    let dir = sftp.read_dir(scratch.as_str()).await.unwrap();
    let entry = dir
        .into_iter()
        .find(|e| e.file_name() == "sftp-up.bin")
        .expect("fichier absent du listing");
    assert_eq!(entry.metadata().size.unwrap_or(0), payload.len() as u64);
    assert!(!entry.file_type().is_dir());

    // download + comparaison
    let mut rf = sftp.open(&remote_path).await.unwrap();
    let mut back = Vec::new();
    rf.read_to_end(&mut back).await.unwrap();
    assert_eq!(back, payload, "contenu téléchargé différent");

    println!("SFTP SMOKE OK — home={home}, upload {} octets, listing et download vérifiés", payload.len());
    std::process::exit(0);
}
