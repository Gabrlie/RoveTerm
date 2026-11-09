//! `devsshd` command-line entry point.

use std::path::PathBuf;

use devsshd::{DevConfig, DevServer, generate_key};

const USAGE: &str = "usage: devsshd [--addr 127.0.0.1:2222] [--user demo] [--password demo] \
[--host-key PATH] [--authorized-key PATH]... [--shell PATH] [--sftp-root DIR]";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut addr = "127.0.0.1:2222".to_owned();
    let mut config = DevConfig::demo();
    config.shell = None;
    let mut host_key_path = PathBuf::from("target/devsshd_host_key");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || {
            args.next()
                .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))
        };
        match flag.as_str() {
            "--addr" => addr = value()?,
            "--user" => config.user = value()?,
            "--password" => config.password = Some(value()?),
            "--host-key" => host_key_path = PathBuf::from(value()?),
            "--authorized-key" => config
                .authorized_keys
                .push(russh::keys::load_public_key(value()?)?),
            "--shell" => config.shell = Some(value()?),
            "--sftp-root" => config.sftp_root = Some(PathBuf::from(value()?)),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}").into()),
        }
    }

    config.host_key = if host_key_path.exists() {
        russh::keys::load_secret_key(&host_key_path, None)?
    } else {
        let key = generate_key();
        if let Some(parent) = host_key_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(
            &host_key_path,
            key.to_openssh(russh::keys::ssh_key::LineEnding::LF)?
                .as_bytes(),
        )?;
        key
    };
    let fingerprint = config.host_key.public_key().fingerprint(Default::default());

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async move {
        let server = DevServer::start(&addr, config).await?;
        tracing::info!(addr = %server.addr(), %fingerprint, "devsshd listening");
        tokio::signal::ctrl_c().await?;
        drop(server);
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}
