use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("tempcheck=info".parse()?))
        .with_writer(std::io::stderr)
        .init();

    tempcheck::app::run(tempcheck::config::Cli::parse()).await
}
