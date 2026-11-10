#![allow(
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::needless_borrow,
    clippy::collapsible_if,
    clippy::format_in_format_args,
    clippy::manual_is_multiple_of,
    clippy::needless_borrows_for_generic_args,
    clippy::suspicious_open_options,
    clippy::type_complexity,
    clippy::while_let_on_iterator
)]

mod api;
mod auth;
mod cli;
mod db;
mod deploy;
mod proxy;
mod server;
mod state;
mod term;
mod ui;

use clap::Parser;

fn main() -> std::process::ExitCode {
    // Secrets live in SQLite (including WAL/SHM) and backups; make every file
    // this process creates private regardless of the caller's umask.
    #[cfg(unix)]
    unsafe {
        libc::umask(0o077);
    }
    let parsed = cli::Cli::parse();
    let json = parsed.json;
    let result = match parsed.cmd {
        cli::Cmd::Serve(args) => {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    term::error(&format!("{err:#}"));
                    return std::process::ExitCode::FAILURE;
                }
            };
            rt.block_on(server::run(args))
        }
        cli::Cmd::DeployGit { app, data, r#ref } => {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    term::error(&format!("{err:#}"));
                    return std::process::ExitCode::FAILURE;
                }
            };
            rt.block_on(cli::deploy_git(app, data, r#ref))
        }
        cmd => cli::run(cmd, json),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            term::error(&format!("{err:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

#[allow(dead_code)]
fn _assert_send()
where
    std::sync::Arc<crate::auth::AuthState>: Send + Sync,
{
}
