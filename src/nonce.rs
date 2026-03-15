use anyhow::{Result, anyhow};
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::CommitmentConfig};
use solana_nonce::state::State as NonceState;
use solana_nonce::versions::Versions as NonceVersions;
use solana_pubkey::Pubkey;
use solana_sdk::hash::Hash;
use std::sync::Arc;
use tokio::time::Duration;

#[derive(Debug, Clone)]
pub struct NonceInfo {
    pub pubkey: Pubkey,
    pub blockhash: Hash,
}

pub struct NonceManager {
    client: Arc<RpcClient>,
    nonces: Vec<Pubkey>,
    current_index: usize,
}

impl NonceManager {
    pub fn new(rpc_url: &str, nonces_b58: &[String]) -> Result<Self> {
        let client = Arc::new(RpcClient::new_with_timeout_and_commitment(
            rpc_url.to_string(),
            Duration::from_secs(3),
            CommitmentConfig::confirmed(),
        ));

        let mut nonces = Vec::new();
        for b58 in nonces_b58 {
            let pubkey = b58
                .parse::<Pubkey>()
                .map_err(|e| anyhow!("Invalid public key for nonce {}: {}", b58, e))?;
            nonces.push(pubkey);
        }

        if nonces.is_empty() {
            return Err(anyhow!("Nonce list cannot be empty"));
        }

        Ok(Self {
            client,
            nonces,
            current_index: 0,
        })
    }

    pub async fn get_next_nonce(&mut self) -> Result<NonceInfo> {
        let pubkey = self.nonces[self.current_index];
        self.current_index = (self.current_index + 1) % self.nonces.len();

        let hash = self.fetch_nonce_hash(&pubkey).await?;

        Ok(NonceInfo {
            pubkey,
            blockhash: hash,
        })
    }

    async fn fetch_nonce_hash(&self, pubkey: &Pubkey) -> Result<Hash> {
        let mut retries = 3;
        while retries > 0 {
            match self.client.get_account(pubkey).await {
                Ok(account) => {
                    if let Ok(state) = bincode::deserialize::<NonceVersions>(&account.data) {
                        match state {
                            NonceVersions::Current(state) => match *state {
                                NonceState::Uninitialized => {
                                    return Err(anyhow!("Nonce account {} is uninitialized", pubkey));
                                }
                                NonceState::Initialized(data) => {
                                    return Ok(data.blockhash());
                                }
                            },
                            NonceVersions::Legacy(state) => match *state {
                                NonceState::Uninitialized => {
                                    return Err(anyhow!("Nonce account {} is uninitialized (Legacy)", pubkey));
                                }
                                NonceState::Initialized(data) => {
                                    return Ok(data.blockhash());
                                }
                            },
                        }
                    } else {
                        return Err(anyhow!("Failed to deserialize nonce account data for {}", pubkey));
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Failed to fetch nonce account {}: {}. Retries left: {}",
                        pubkey,
                        e,
                        retries - 1
                    );
                    retries -= 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        Err(anyhow!("Failed to fetch nonce hash after retries"))
    }
}
