use anyhow::Result;
use probing_cli::cli_main;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // cli_main already uses #[tokio::main], so it handles async execution internally
    cli_main(args).await
}
