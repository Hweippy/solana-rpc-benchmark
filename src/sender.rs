use crate::config::SenderConfig;
use anyhow::{Result, anyhow};
use base64::Engine as _;
use reqwest::{
    Client,
    header::{HeaderName, HeaderValue},
};
use serde_json::json;
use solana_sdk::transaction::VersionedTransaction;
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct SenderClient {
    client: Client,
}

impl SenderClient {
    pub fn new(send_timeout: Duration) -> Self {
        Self {
            client: Client::builder().timeout(send_timeout).build().unwrap(),
        }
    }

    pub async fn send_transaction(&self, tx: &VersionedTransaction, config: &SenderConfig) -> Result<()> {
        let serialized = bincode::serialize(&tx).map_err(|e| anyhow!("Failed to serialize tx: {}", e))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(serialized);

        let req_body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [
                encoded,
                {
                    "encoding": "base64",
                    "skipPreflight": true,
                    "maxRetries": 0,
                }
            ]
        });

        // Send to all URLs for this sender simultaneously
        let mut futures = Vec::new();
        for url_str in config.urls.clone() {
            let mut url = url_str;
            let client = self.client.clone();
            let body = req_body.clone();
            let sender_name = config.name.clone();
            let signature = tx.signatures[0];

            let mut request = client.post(&url);

            if let Some(api_key) = &config.api_key {
                if let Some(header_name_str) = &config.header {
                    // Header-based authentication
                    let header_name = HeaderName::from_str(header_name_str)
                        .map_err(|e| anyhow!("Invalid header name {}: {}", header_name_str, e))?;
                    let header_value = HeaderValue::from_str(api_key)
                        .map_err(|e| anyhow!("Invalid API key for header {}: {}", header_name_str, e))?;
                    request = request.header(header_name, header_value);
                } else {
                    // URL-based authentication (default)
                    if url.contains('?') {
                        url = format!("{}&api-key={}", url, api_key);
                    } else {
                        url = format!("{}?api-key={}", url, api_key);
                    }
                    request = client.post(&url);
                }
            }

            futures.push(tokio::spawn(async move {
                let res = request.json(&body).send().await;

                match res {
                    Ok(response) => {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        if !status.is_success() {
                            log::warn!("Sender {} URL {} returned error {}: {}", sender_name, url, status, text);
                        } else {
                            log::debug!(
                                "Sender {} Sig {} URL {} response: {}",
                                sender_name,
                                signature,
                                url,
                                text
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!("Sender {} URL {} request failed: {:?}", sender_name, url, e);
                    }
                }
            }));
        }

        // Wait for all requests to finish for this sender for this round
        for f in futures {
            let _ = f.await;
        }

        Ok(())
    }
}
