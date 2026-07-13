//! Smoke test du transfert de config (exec + tar via stdin, même plomberie que claude_sync).
//! Usage : cargo run --example sync_smoke -- <host> <port> <user> <key_path> <scratch_dir>

use arabel_lib::ssh::{connect_auth, exec};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (host, port, user, key_path, scratch) =
        (&args[1], args[2].parse::<u16>().unwrap(), &args[3], &args[4], &args[5]);

    // tarball local d'un fichier de test
    let src = format!("{scratch}/src");
    std::fs::create_dir_all(format!("{src}/.claude")).unwrap();
    std::fs::write(format!("{src}/.claude/CLAUDE.md"), "arabel-sync-ok\n").unwrap();
    let tar = std::process::Command::new("tar")
        .args(["czf", "-", "-C", &src, ".claude"])
        .output()
        .unwrap();
    assert!(tar.status.success());

    let handle = connect_auth(host, port, user, key_path, None, None).await.unwrap();
    let dst = format!("{scratch}/dst");
    let (st, out) = exec(&handle, &format!("mkdir -p {dst}"), None).await.unwrap();
    assert_eq!(st, 0, "mkdir: {out}");
    let (st, out) = exec(&handle, &format!("tar xzf - -C {dst}"), Some(&tar.stdout)).await.unwrap();
    assert_eq!(st, 0, "tar distant: {out}");

    // hôte de test = localhost, donc on vérifie directement sur le disque
    let content = std::fs::read_to_string(format!("{dst}/.claude/CLAUDE.md")).unwrap();
    assert_eq!(content, "arabel-sync-ok\n");
    println!("SYNC SMOKE OK — exec, stdin streamé et extraction tar distants passés");
}
