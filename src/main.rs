mod api;
mod cli;
mod db;
mod deploy;
mod proxy;
mod server;
mod state;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let parsed = cli::Cli::parse();
    match parsed.cmd {
        cli::Cmd::Serve(args) => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(server::run(args))
        }
        cmd => cli::run(cmd),
    }
}
