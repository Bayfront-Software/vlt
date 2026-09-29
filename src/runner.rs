//! `vlt run` の子プロセス実行。出力を伏せ字にしながら中継する。
//!
//! 出力を中継する間は vlt が親として残るので、終了コードとシグナルを子に揃える:
//! - 子の終了コードをそのまま返す（シグナルで死んだら 128+番号、シェルの慣習）。
//! - SIGTERM / SIGHUP は子へ転送する（監視ツールが vlt だけに送っても子が止まるように）。
//! - SIGINT は無視する（端末の Ctrl-C は同じプロセスグループの子にも届くので、二重に送らない）。

use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};

use crate::mask::Masker;

static CHILD_PID: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        // kill は async-signal-safe
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

/// 各出力を伏せ字にするか。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskStreams {
    pub stdout: bool,
    pub stderr: bool,
}

/// 伏せ字にする出力先を決める。既定（force=None）は「端末でない出力だけ」。
/// 端末へ出す対話型プログラム（`vlt run -- claude` など）をパイプで壊さないため。
pub fn mask_streams(force: Option<bool>, stdout_is_tty: bool, stderr_is_tty: bool) -> MaskStreams {
    match force {
        Some(on) => MaskStreams { stdout: on, stderr: on },
        None => MaskStreams { stdout: !stdout_is_tty, stderr: !stderr_is_tty },
    }
}

pub fn is_tty(fd: libc::c_int) -> bool {
    unsafe { libc::isatty(fd) == 1 }
}

fn pump(mut from: impl Read, mut to: impl Write, secrets: &[String]) -> std::io::Result<()> {
    let mut masker = Masker::new(secrets.iter().map(String::as_bytes));
    let mut buf = [0u8; 8192];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        let out = masker.push(&buf[..n]);
        if !out.is_empty() {
            to.write_all(&out)?;
            to.flush()?;
        }
    }
    to.write_all(&masker.finish())?;
    to.flush()
}

/// 子プロセスを起動し、指定の出力を伏せ字にして中継する。終了コードを返す。
pub fn run_masked(
    cmd: &[String],
    env: &[(String, String)],
    secrets: &[String],
    streams: MaskStreams,
    out: Box<dyn Write + Send>,
    err: Box<dyn Write + Send>,
) -> Result<i32, String> {
    let (program, args) = cmd.split_first().ok_or("実行するコマンドがありません")?;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::inherit())
        .stdout(if streams.stdout { Stdio::piped() } else { Stdio::inherit() })
        .stderr(if streams.stderr { Stdio::piped() } else { Stdio::inherit() });
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to run {program}: {e}"))?;

    CHILD_PID.store(child.id() as i32, Ordering::SeqCst);
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, forward_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, forward_signal as *const () as libc::sighandler_t);
    }

    let mut threads = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let secrets = secrets.to_vec();
        threads.push(std::thread::spawn(move || pump(stdout, out, &secrets)));
    }
    if let Some(stderr) = child.stderr.take() {
        let secrets = secrets.to_vec();
        threads.push(std::thread::spawn(move || pump(stderr, err, &secrets)));
    }
    let status = child.wait().map_err(|e| format!("Failed to wait for {program}: {e}"))?;
    for t in threads {
        // 読み手が先に閉じた（`| head` など）ときの書き込み失敗は無視してよい
        let _ = t.join();
    }
    CHILD_PID.store(0, Ordering::SeqCst);
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
        libc::signal(libc::SIGHUP, libc::SIG_DFL);
    }
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Sink {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn sh(script: &str, env: &[(&str, &str)], secrets: &[&str]) -> (i32, String, String) {
        let (out, err) = (Sink::default(), Sink::default());
        let env: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let secrets: Vec<String> = secrets.iter().map(|s| s.to_string()).collect();
        let code = run_masked(
            &["/bin/sh".into(), "-c".into(), script.into()],
            &env,
            &secrets,
            MaskStreams { stdout: true, stderr: true },
            Box::new(out.clone()),
            Box::new(err.clone()),
        )
        .unwrap();
        (code, out.text(), err.text())
    }

    #[test]
    fn masks_stdout_and_stderr_and_keeps_exit_code() {
        let (code, out, err) = sh(r#"echo "token=$TOKEN"; echo "oops $TOKEN" >&2; exit 3"#, &[("TOKEN", "s3cret-value")], &["s3cret-value"]);
        assert_eq!(code, 3);
        assert_eq!(out, "token=<concealed by vlt>\n");
        assert_eq!(err, "oops <concealed by vlt>\n");
    }

    #[test]
    fn env_is_exactly_what_was_given() {
        let (_, out, _) = sh("echo \"[$A][$HOME_SHOULD_BE_UNSET]\"", &[("A", "1")], &[]);
        assert_eq!(out, "[1][]\n");
    }

    #[test]
    fn signal_death_becomes_128_plus_signal() {
        let (code, _, _) = sh("kill -TERM $$", &[], &[]);
        assert_eq!(code, 128 + libc::SIGTERM);
    }

    #[test]
    fn missing_program_is_an_error() {
        let result = run_masked(
            &["/nonexistent/program".into()],
            &[],
            &[],
            MaskStreams { stdout: true, stderr: true },
            Box::new(Sink::default()),
            Box::new(Sink::default()),
        );
        assert!(result.unwrap_err().contains("/nonexistent/program"));
    }

    #[test]
    fn default_masks_only_non_terminal_streams() {
        assert_eq!(mask_streams(None, false, false), MaskStreams { stdout: true, stderr: true });
        assert_eq!(mask_streams(None, true, false), MaskStreams { stdout: false, stderr: true });
        assert_eq!(mask_streams(Some(true), true, true), MaskStreams { stdout: true, stderr: true });
        assert_eq!(mask_streams(Some(false), false, false), MaskStreams { stdout: false, stderr: false });
    }
}
