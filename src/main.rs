use anyhow::{Context, Result, anyhow};
use log::{error, info};
use rand::seq::SliceRandom;
use reqwest::Client;
use serde_json::Value;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::CommitmentConfig};
use solana_sdk::message::AddressLookupTableAccount;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Signature, Signer};
use solana_sdk::signer::keypair::read_keypair_file;
use std::str::FromStr;
use std::time::Duration;

use clap::Parser;

mod builder;
mod config;
mod nonce;
mod sender;
mod tracker;

use crate::{
    builder::build_transaction, config::load_config, nonce::NonceManager, sender::SenderClient, tracker::Tracker,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to the configuration file
    config: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    info!("Starting RPC-Bench");

    let cli = Cli::parse();
    let config = load_config(&cli.config)?;

    let payer = read_keypair_file(&config.benchmark.payer_keypair_path).map_err(|e| {
        anyhow!(
            "Failed to read keypair from '{}': {}",
            config.benchmark.payer_keypair_path,
            e
        )
    })?;
    let payer_pubkey = payer.pubkey().to_string();
    info!("Loaded payer: {}", payer_pubkey);

    let rpc_client = RpcClient::new_with_timeout_and_commitment(
        config.benchmark.rpc_url.clone(),
        Duration::from_secs(30),
        CommitmentConfig::confirmed(),
    );

    // Jupiter integration
    let client = Client::new();

    // 1. Get Quote
    info!("Fetching Jupiter quote...");
    let quote_url = format!(
        "{}/quote?outputMint=EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v&inputMint=So11111111111111111111111111111111111111112&amount=1000000000",
        config.benchmark.jupiter_url.trim_end_matches('/')
    );
    let mut quote: Value = client.get(quote_url).send().await?.json().await?;

    // 2. Modify Quote: outAmount * 10, slippageBps = 0
    if let Some(out_amount_str) = quote.get("outAmount").and_then(|v| v.as_str()) {
        let out_amount: u128 = out_amount_str.parse()?;
        quote["outAmount"] = Value::String((out_amount * 10).to_string());
    }
    quote["slippageBps"] = Value::Number(0.into());

    // 3. Get Swap Instructions
    info!("Fetching Jupiter swap instructions...");
    let swap_url = format!(
        "{}/swap-instructions",
        config.benchmark.jupiter_url.trim_end_matches('/')
    );
    let swap_req_body = serde_json::json!({
        "dynamicComputeUnitLimit": true,
        "userPublicKey": payer_pubkey,
        "quoteResponse": quote
    });

    let swap_data: Value = client.post(swap_url).json(&swap_req_body).send().await?.json().await?;

    // Extract instructions
    let mut jupiter_ixs = Vec::new();
    if let Some(setup_ixs) = swap_data.get("setupInstructions").and_then(|v| v.as_array()) {
        jupiter_ixs.extend(setup_ixs.clone());
    }
    jupiter_ixs.push(
        swap_data
            .get("swapInstruction")
            .context("No swapInstruction in response")?
            .clone(),
    );
    if let Some(cleanup_ix) = swap_data
        .get("cleanupInstruction")
        .and_then(|v| if v.is_null() { None } else { Some(v) })
    {
        jupiter_ixs.push(cleanup_ix.clone());
    }

    let compute_unit_limit = swap_data
        .get("computeUnitLimit")
        .and_then(|v| v.as_u64())
        .map(|limit| (limit as f64 * 1.1) as u32)
        .unwrap_or(200_000);

    // Fetch ALTs
    let mut lookup_tables = Vec::new();
    if let Some(alt_addresses) = swap_data.get("addressLookupTableAddresses").and_then(|v| v.as_array()) {
        for addr_val in alt_addresses {
            let addr_str = addr_val.as_str().context("ALT address not a string")?;
            let pubkey = Pubkey::from_str(addr_str)?;
            info!("Fetching ALT account: {}", addr_str);
            let account = rpc_client.get_account(&pubkey).await?;
            let alt_state =
                solana_address_lookup_table_interface::state::AddressLookupTable::deserialize(&account.data)
                    .map_err(|e| anyhow!("Failed to deserialize ALT {}: {:?}", addr_str, e))?;
            lookup_tables.push(AddressLookupTableAccount {
                key: pubkey,
                addresses: alt_state.addresses.to_vec(),
            });
        }
    }

    info!(
        "Jupiter setup complete. Instructions: {}, ALTs: {}, CU Limit: {}",
        jupiter_ixs.len(),
        lookup_tables.len(),
        compute_unit_limit,
    );

    let mut nonce_manager = NonceManager::new(&config.benchmark.rpc_url, &config.nonces)?;
    let sender_client = SenderClient::new(Duration::from_secs(config.benchmark.send_timeout));
    let mut tracker = Tracker::new();

    // BENCHMARK LOOP
    tokio::select! {
        _ = async {
            for i in 0..config.benchmark.tx_count {
                info!("Starting round {}/{}...", i + 1, config.benchmark.tx_count);
                let nonce_info = match nonce_manager.get_next_nonce().await {
                    Ok(n) => n,
                    Err(e) => {
                        error!("Failed to fetch nonce: {}", e);
                        continue;
                    }
                };

                let mut round_signatures = Vec::new();
                let mut shuffled_senders = config.senders.clone();
                shuffled_senders.shuffle(&mut rand::thread_rng());

                for sender in &shuffled_senders {
                    let params = builder::BuildParams {
                        cu_price: config.benchmark.cu_price,
                        tip: config.benchmark.tip,
                        jup_ixs: &jupiter_ixs,
                        cu_limit: compute_unit_limit,
                        lookup_tables: lookup_tables.clone(),
                    };

                    match build_transaction(&payer, &nonce_info, sender, params) {
                        Ok(tx) => {
                            let sig = tx.signatures[0];
                            tracker.record_signature(sig, sender.name.clone());
                            round_signatures.push((sig, sender.clone(), tx));
                        }
                        Err(e) => {
                            error!("Failed to build tx for sender {}: {}", sender.name, e);
                        }
                    }
                }

                let mut futures = Vec::new();
                for (_sig, sender_config, tx) in round_signatures {
                    let client = sender_client.clone();

                    futures.push(tokio::spawn(async move {
                        if let Err(e) = client.send_transaction(&tx, &sender_config).await {
                            error!("Failed to send tx for {}: {}", sender_config.name, e);
                        }
                    }));
                }

                for f in futures {
                    let _ = f.await;
                }

                if i < config.benchmark.tx_count - 1 {
                    tokio::time::sleep(Duration::from_millis(config.benchmark.delay_ms)).await;
                }
            }
            Ok::<(), anyhow::Error>(())
        } => {
            info!("Finished sending all trades.");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Interrupted! Printing current benchmark info...");
        }
    }

    info!("Waiting 5 seconds for final transactions to process...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    info!("Fetching signature statuses to determine winners...");
    let signatures_to_check: Vec<Signature> = tracker.pending_signatures.keys().cloned().collect();

    let mut landed_count_by_sender: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for sender in &config.senders {
        landed_count_by_sender.insert(sender.name.clone(), 0);
    }

    let chunk_size = 200;
    for chunk in signatures_to_check.chunks(chunk_size) {
        match rpc_client.get_signature_statuses(chunk).await {
            Ok(response) => {
                let statuses = response.value;
                for (_sig, status_opt) in chunk.iter().zip(statuses) {
                    if let Some(_status) = status_opt
                        && let Some(sender_name) = tracker.pending_signatures.get(_sig)
                    {
                        *landed_count_by_sender.entry(sender_name.clone()).or_insert(0) += 1;
                    }
                }
            }
            Err(e) => {
                error!("Failed to fetch signature statuses batch: {}", e);
            }
        }
    }

    println!("\n================ BENCHMARK SUMMARY ================");
    let total_landed: usize = landed_count_by_sender.values().sum();
    let total_sent = tracker.pending_signatures.len();
    println!("Total Transactions Sent: {}", total_sent);
    println!("Total Landed:            {}", total_landed);
    println!("Delay:                   {} ms", config.benchmark.delay_ms);
    println!("CU Price:                {} micro-lamports", config.benchmark.cu_price);
    if let Some(tip) = config.benchmark.tip {
        println!("Tip:                     {} SOL", tip as f64 / 1_000_000_000.0);
    }
    println!("---------------------------------------------------");

    let mut results: Vec<(String, usize)> = landed_count_by_sender.into_iter().collect();
    results.sort_by(|a, b| b.1.cmp(&a.1));

    let priority_fee_lamports = (compute_unit_limit as f64 * config.benchmark.cu_price as f64) / 1_000_000.0;
    let base_fee_lamports = 5000.0;
    let cost_per_landed_sol = (priority_fee_lamports + base_fee_lamports) / 1_000_000_000.0;

    for (name, count) in results {
        let pct = if total_sent > 0 {
            (count as f64 / total_sent as f64) * 100.0
        } else {
            0.0
        };
        println!("{:<20} | Landed: {:<5} | {:.2}%", name, count, pct);
    }
    println!("---------------------------------------------------");
    let total_cost_sol = total_landed as f64 * cost_per_landed_sol;
    println!("Total Execution Cost: {:.6} SOL", total_cost_sol);
    println!("===================================================");

    Ok(())
}
