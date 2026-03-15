# RPC-Bench

`rpc-bench` is a high-performance Solana RPC benchmarking tool designed to measure the landing performance and latency of various RPC providers under load. It integrates with Jupiter to build realistic swap transactions and uses Nonce accounts to track landing rates precisely.

## Key Features

- **Multi-Sender Benchmarking**: Compare multiple RPC providers/endpoints simultaneously.
- **Jupiter Integration**: Pulls live quotes and swap instructions to build realistic transaction payloads.
- **Nonce-Based Tracking**: Uses durable nonces to ensure transactions are tracked accurately even if they fail to land.
- **SWQoS Support**: Benchmark endpoints specifically tuned for Stake-Weighted Quality of Service.
- **Customizable Fees**: Configure Compute Unit (CU) price and optional Jito-style tips.
- **Detailed Analytics**: Generates a summary table with landed counts, percentage rates, and estimated execution costs.

## Prerequisites

- **Rust**: Latest stable version.
- **Solana Keypair**: A funded payer account for transaction fees.
- **Nonce Accounts**: At least one initialized Nonce account is required for the benchmarking loop. **Multiple nonces are preferable** (3-4 is usually enough) to avoid bottlenecks and ensure smooth concurrent broadcasting.

## Installation

```bash
git clone <repository-url>
cd rpc-bench
cargo build --release
```

## Configuration

The tool uses a TOML configuration file. See [config-example.toml](config-example.toml) for a fully documented example.

### Key Sections:

- **`nonces`**: A list of Nonce account public keys.
- **`[benchmark]`**:
    - `rpc_url`: The primary RPC used for fetching data (ALTs, quotes, statuses).
    - `payer_keypair_path`: Path to your Solana keypair JSON.
    - `tx_count`: Number of rounds to run.
    - `delay_ms`: Time to wait between rounds.
    - `cu_price`: Priority fee in micro-lamports.
    - `tip`: (Optional) Jito tip in lamports.
- **`[[senders]]`**:
    - `name`: Identifying tag for the provider.
    - `urls`: List of endpoints to broadcast to.
    - `api_key` & `header`: (Optional) For authenticated RPC access.

## Usage

To run a benchmark:

```bash
./target/release/rpc-bench <path-to-your-config.toml>
```

## How It Works

1. **Setup**: The tool fetches a Jupiter quote and builds a set of swap instructions.
2. **Loop**: For each round (`tx_count`):
    - Fetches the next available Nonce from the configured accounts.
    - Builds a versioned transaction containing the swap logic, priority fees, and an optional tip.
    - **Note**: The transaction is intentionally built with a trailing "Advance Nonce" instruction to ensure it doesn't actually swap funds but still measures the path to the leader.
    - Broadcasts the transaction to all configured `senders` simultaneously.
3. **Analysis**: After the loop, it waits for a few seconds and then queries the `rpc_url` for the signature statuses of all sent transactions.
4. **Report**: Outputs a comparative table of performance metrics.

## Interpreting Results

The output summary shows:
- **Landed**: Number of transactions that successfully reached the `confirmed` status.
- **Rate (%)**: Success rate relative to the total rounds.
- **Execution Cost**: Estimated SOL spent on priority fees for the landed transactions.

## Example Results

When a benchmark completes, it prints a summary table similar to this:

```text
================ BENCHMARK SUMMARY ================
Total Transactions Sent: 80
Total Landed:            80
Delay:                   1000 ms
CU Price:                100 micro-lamports
Tip:                     0.001 SOL
---------------------------------------------------
sender1              | Landed: 60    | 75.00%
sender2              | Landed: 30    | 37.50%
sender3              | Landed: 10    | 12.50%
---------------------------------------------------
Total Execution Cost: 0.000410 SOL
===================================================
```

---

*This tool is intended for performance testing and optimization. Use responsibly.*
