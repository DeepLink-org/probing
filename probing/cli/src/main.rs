use anyhow::Result;
use probing_cli::cli_main;

#[tokio::main]
pub async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    cli_main(args).await
}
