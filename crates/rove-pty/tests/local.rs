#![cfg(unix)]

use std::time::Duration;

use rove_pty::{LocalRequest, LocalTerminals, PtyConfig};
use rove_session::{TermSize, TerminalOutput};
use tokio::time::timeout;

fn sh() -> LocalRequest {
    LocalRequest {
        shell: Some("/bin/sh".into()),
        cwd: None,
        size: Some(TermSize::new(100, 30).unwrap()),
    }
}

async fn collect(
    output: &mut tokio::sync::mpsc::Receiver<TerminalOutput>,
) -> (Vec<u8>, Option<i32>) {
    let mut data = Vec::new();
    loop {
        match timeout(Duration::from_secs(10), output.recv())
            .await
            .expect("pty output timed out")
        {
            Some(TerminalOutput::Data(chunk)) => data.extend_from_slice(&chunk),
            Some(TerminalOutput::Exit(status)) => return (data, status.code),
            None => return (data, None),
        }
    }
}

#[tokio::test]
async fn runs_commands_and_reports_exit_code() {
    let mut opened = LocalTerminals::new(PtyConfig::default())
        .open(sh())
        .unwrap();
    opened
        .session
        .write(b"echo hel''lo; stty size; exit 7\n")
        .await
        .unwrap();
    let (data, code) = collect(&mut opened.output).await;
    let text = String::from_utf8_lossy(&data);
    assert!(text.contains("hello"), "output: {text}");
    assert!(text.contains("30 100"), "output: {text}");
    assert_eq!(code, Some(7));
}

#[tokio::test]
async fn resize_is_visible_to_the_shell() {
    let mut opened = LocalTerminals::new(PtyConfig::default())
        .open(sh())
        .unwrap();
    opened
        .session
        .resize(TermSize::new(120, 40).unwrap())
        .await
        .unwrap();
    opened.session.write(b"stty size; exit\n").await.unwrap();
    let (data, _) = collect(&mut opened.output).await;
    assert!(String::from_utf8_lossy(&data).contains("40 120"));
}

#[tokio::test]
async fn close_terminates_the_shell_and_is_idempotent() {
    let mut opened = LocalTerminals::new(PtyConfig::default())
        .open(sh())
        .unwrap();
    opened.session.close().await.unwrap();
    opened.session.close().await.unwrap();
    let (_, _) = collect(&mut opened.output).await;
    assert!(opened.session.write(b"x").await.is_err());
}
