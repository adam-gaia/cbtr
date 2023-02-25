use anyhow::Result;
use lib::run;

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}
