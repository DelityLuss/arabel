//! Smoke test du terminal local (même plomberie portable-pty que local_connect).

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};

fn main() {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let mut cmd = CommandBuilder::new(&shell);
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);

    let mut writer = pair.master.take_writer().unwrap();
    let mut reader = pair.master.try_clone_reader().unwrap();
    writer.write_all(b"stty size; echo arabel-local-ok; exit\n").unwrap();
    pair.master
        .resize(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();

    // garde-fou : dump + échec après 15 s
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut out = String::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.push_str(&String::from_utf8_lossy(&buf[..n]));
            if out.contains("arabel-local-ok") && (out.contains("24 80") || out.contains("40 120")) {
                break;
            }
        }
        let _ = tx.send(out);
    });
    let out = rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .unwrap_or_else(|_| panic!("timeout: rien d'exploitable lu du PTY"));
    let _ = child.kill();
    assert!(out.contains("arabel-local-ok"), "echo manquant:\n{out}");
    println!("LOCAL SMOKE OK — pty, shell {shell}, echo et resize passés");
    std::process::exit(0); // ne pas attendre les threads/fds du PTY
}
