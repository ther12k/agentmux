use agentmux::cli::Cli;
use clap::Parser;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    cli.run()
}
