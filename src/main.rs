use std::time::Duration;

use crawler::NtnuCrawler;
use tokio::time::sleep;

mod config;
mod crawler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut crawler = NtnuCrawler::new(
        "https://cos4s.ntnu.edu.tw".to_owned(),
        "http://localhost:8080".to_owned(),
        "40947030S".to_owned(),
        "C*2q5L5Ps1U1".to_owned(),
    );
    println!("Start login");
    crawler.login().await?;
    println!("Start landing");
    crawler.landing_page().await?;
    println!("Start query");
    println!("{:?}", crawler.query("1234").await?);
    Ok(())
}
