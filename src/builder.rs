use crate::config::SenderConfig;
use crate::nonce::NonceInfo;
use anyhow::{Context, Result, anyhow};
use base64::{Engine as _, engine::general_purpose};
use rand::seq::SliceRandom;
use serde_json::Value;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    message::{AddressLookupTableAccount, VersionedMessage, v0},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use solana_system_interface::instruction as system_instruction;
use std::str::FromStr;

pub struct BuildParams<'a> {
    pub cu_price: u64,
    pub tip: Option<u64>,
    pub jup_ixs: &'a [Value],
    pub cu_limit: u32,
    pub lookup_tables: Vec<AddressLookupTableAccount>,
}

pub fn build_transaction(
    payer: &Keypair,
    nonce_info: &NonceInfo,
    sender_config: &SenderConfig,
    params: BuildParams,
) -> Result<VersionedTransaction> {
    let advance_nonce_ix = system_instruction::advance_nonce_account(
        &nonce_info.pubkey,
        &payer.pubkey(), // The payer is the authorized authority
    );

    let cu_limit_ix = ComputeBudgetInstruction::set_compute_unit_limit(params.cu_limit);
    let cu_price_ix = ComputeBudgetInstruction::set_compute_unit_price(params.cu_price);

    let mut instructions = vec![advance_nonce_ix.clone(), cu_limit_ix, cu_price_ix];

    // Add Jupiter instructions
    for jup_ix in params.jup_ixs {
        instructions.push(parse_jupiter_instruction(jup_ix)?);
    }

    // Add tip instruction if addresses are available
    if !sender_config.tip_addresses.is_empty() {
        let mut rng = rand::thread_rng();
        let tip_address_str = sender_config.tip_addresses.choose(&mut rng).unwrap();

        let tip_address = Pubkey::from_str(tip_address_str).map_err(|e| {
            anyhow!(
                "Invalid tip address {} for sender {}: {}",
                tip_address_str,
                sender_config.name,
                e
            )
        })?;

        let transfer_ix = system_instruction::transfer(&payer.pubkey(), &tip_address, params.tip.unwrap());
        instructions.push(transfer_ix);
    }

    // advance_nonce_ix is added twice, to the transaction is built to fail, which is expected and what we want
    instructions.push(advance_nonce_ix);

    // Build v0 message
    let message = v0::Message::try_compile(
        &payer.pubkey(),
        &instructions,
        &params.lookup_tables,
        nonce_info.blockhash,
    )
    .map_err(|e| anyhow!("Failed to compile v0 message: {}", e))?;

    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[payer])
        .map_err(|e| anyhow!("Failed to create VersionedTransaction: {}", e))?;

    Ok(tx)
}

fn parse_jupiter_instruction(ix_val: &Value) -> Result<Instruction> {
    let program_id = Pubkey::from_str(
        ix_val
            .get("programId")
            .and_then(|v| v.as_str())
            .context("No programId")?,
    )?;
    let accounts_val = ix_val
        .get("accounts")
        .and_then(|v| v.as_array())
        .context("No accounts")?;
    let mut accounts = Vec::new();
    for acc in accounts_val {
        accounts.push(AccountMeta {
            pubkey: Pubkey::from_str(acc.get("pubkey").and_then(|v| v.as_str()).context("No pubkey")?)?,
            is_signer: acc.get("isSigner").and_then(|v| v.as_bool()).unwrap_or(false),
            is_writable: acc.get("isWritable").and_then(|v| v.as_bool()).unwrap_or(false),
        });
    }
    let data = general_purpose::STANDARD
        .decode(ix_val.get("data").and_then(|v| v.as_str()).context("No data")?)
        .map_err(|e| anyhow!("Failed to decode Jupiter ix data: {}", e))?;

    Ok(Instruction {
        program_id,
        accounts,
        data,
    })
}
