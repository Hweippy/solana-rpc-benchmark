use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct BenchmarkConfig {
    pub payer_keypair_path: String,
    pub rpc_url: String,
    pub tx_count: usize,
    pub delay_ms: u64,
    pub cu_price: u64,
    pub tip: Option<u64>,
    #[serde(default = "default_jupiter_url")]
    pub jupiter_url: String,
}

fn default_jupiter_url() -> String {
    "https://lite-api.jup.ag/swap/v1".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct SenderConfig {
    pub name: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub tip_addresses: Vec<String>,
    pub api_key: Option<String>,
    pub header: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub benchmark: BenchmarkConfig,
    pub nonces: Vec<String>,
    pub senders: Vec<SenderConfig>,
}

pub fn load_config(path: &str) -> Result<Config> {
    let contents = fs::read_to_string(path).context("Failed to read config file")?;
    let config: Config = toml::from_str(&contents).context("Failed to parse config file")?;
    Ok(config)
}
