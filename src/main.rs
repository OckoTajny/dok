mod api;
mod auth;
mod cli;
mod db;
mod deploy;
mod proxy;
mod server;
mod state;
mod ui;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    // Secrets live in SQLite (including WAL/SHM) and backups; make every file
    // this process creates private regardless of the caller's umask.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }
    let parsed = cli::Cli::parse();
    match parsed.cmd {
        cli::Cmd::Serve(args) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(server::run(args))
        }
        cli::Cmd::DeployGit { app, data, r#ref } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(cli::deploy_git(app, data, r#ref))
        }
        cmd => cli::run(cmd),
    }
}

#[allow(dead_code)]
fn _assert_send()
where
    std::sync::Arc<crate::auth::AuthState>: Send + Sync,
{
}
