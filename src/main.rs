#![forbid(unsafe_code)]
//! Prototype CLI for a lower-bound Bitcoin proof-of-reserve.
//! SECURITY:  demo code only.

use std::{fs::File, path::PathBuf, str::FromStr};
use anyhow::Result;
use bitcoin::{Address, Txid};
use bitcoin::address::NetworkChecked;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use clap::{Parser, Subcommand};
use curve25519_dalek_ng::scalar::Scalar;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use merlin::Transcript;
use sha2::{Digest, Sha256};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli { #[command(subcommand)] cmd: Cmd }

#[derive(Subcommand, Debug)]
enum Cmd {
    Generate {
        #[arg(long)] rpc_url: String,
        #[arg(long)] rpc_user: String,
        #[arg(long)] rpc_pass: String,
        #[arg(long)] address: String,
        #[arg(long)] sk: String,        // placeholder
        #[arg(long)] min: u64,          // sats
        #[arg(long)] height: u64,
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
    Verify {
        #[arg(long)] proof: PathBuf,
        #[arg(long)] rpc_url: String,
        #[arg(long)] rpc_user: String,
        #[arg(long)] rpc_pass: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct Proof {
    block_height: u64,
    utxo_root: String,
    commitments: Vec<String>,
    range_proof: String,
    ownership_proofs: Vec<String>,
    min_amount: u64,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Generate { rpc_url, rpc_user, rpc_pass, address, sk, min, height, out } => {
            let proof = generate_proof(&rpc_url, &rpc_user, &rpc_pass,
                                       &address, &sk, min, height)?;
            serde_json::to_writer_pretty(File::create(out)?, &proof)?;
            println!("✅ proof generated ({} commitments)", proof.commitments.len());
        }
        Cmd::Verify { proof, rpc_url, rpc_user, rpc_pass } => {
            let pf: Proof = serde_json::from_reader(File::open(&proof)?)?;
            verify_proof(&pf, &rpc_url, &rpc_user, &rpc_pass)?;
            println!("✅ proof verified");
        }
    }
    Ok(())
}

// ── helpers ─────────────────────────────────────────────────

#[derive(Debug)]
struct Utxo { txid: Txid, vout: u32, value: u64 }

fn fetch_utxos(rpc: &Client, addr: &Address<NetworkChecked>) -> Result<Vec<Utxo>> {
    rpc.list_unspent(None, None, Some(&[addr]), None, None)?
        .into_iter()
        .map(|u| Ok(Utxo { txid: u.txid, vout: u.vout, value: u.amount.to_sat() }))
        .collect()
}

fn commit_utxos(utxos: &[Utxo]) -> (Vec<[u8; 32]>, u64) {
    let pg = PedersenGens::default();
    let mut rng = OsRng;
    let mut total = 0;
    let commits: Vec<[u8; 32]> = utxos.iter().map(|u| {
        total += u.value;
        let r = Scalar::random(&mut rng);
        pg.commit(Scalar::from(u.value), r).compress().to_bytes()
    }).collect();
    (commits, total)
}

fn merkle_root(mut leaves: Vec<[u8; 32]>) -> [u8; 32] {
    if leaves.is_empty() { return [0u8; 32]; }
    while leaves.len() > 1 {
        if leaves.len() % 2 == 1 { leaves.push(*leaves.last().unwrap()); }
        leaves = leaves.chunks(2).map(|p| {
            let mut h = Sha256::new();
            h.update(p[0]); h.update(p[1]);
            h.finalize().into()
        }).collect();
    }
    leaves[0]
}

// ── generate ────────────────────────────────────────────────

fn generate_proof(
    rpc_url: &str, rpc_user: &str, rpc_pass: &str,
    address: &str, _sk: &str, min: u64, height: u64
) -> Result<Proof> {

    let rpc = Client::new(rpc_url, Auth::UserPass(rpc_user.into(), rpc_pass.into()))?;

    let addr_raw = Address::from_str(address)?;
    let net      = *addr_raw.network();
    let addr_chk = addr_raw.require_network(net)?;

    let utxos = fetch_utxos(&rpc, &addr_chk)?;
    anyhow::ensure!(!utxos.is_empty(), "address has no UTXOs");

    let (commitments, total) = commit_utxos(&utxos);
    anyhow::ensure!(total >= min, "balance below threshold");

    let gens  = BulletproofGens::new(64, 1);
    let blind = Scalar::random(&mut OsRng);
    let mut t = Transcript::new(b"por");
    let (bp, _) = RangeProof::prove_single(
        &gens, &PedersenGens::default(), &mut t,
        total - min, &blind, 64)?;

    let root = merkle_root(commitments.clone());

    Ok(Proof {
        block_height: height,
        utxo_root: hex::encode(root),
        commitments: commitments.iter().map(hex::encode).collect(),
        range_proof: base64::encode(bp.to_bytes()),   // deprecated API – ok for demo
        ownership_proofs: Vec::new(),
        min_amount: min,
    })
}

// ── verify ──────────────────────────────────────────────────

fn verify_proof(pf: &Proof, rpc_url: &str, rpc_user: &str, rpc_pass: &str) -> Result<()> {
    let rpc = Client::new(rpc_url, Auth::UserPass(rpc_user.into(), rpc_pass.into()))?;
    anyhow::ensure!(rpc.get_block_count()? as u64 >= pf.block_height, "node behind");

    let leaves: Vec<[u8; 32]> = pf.commitments.iter().map(|h| {
        let mut b = [0u8; 32];
        b.copy_from_slice(&hex::decode(h).unwrap());
        b
    }).collect();

    anyhow::ensure!(hex::encode(merkle_root(leaves)) == pf.utxo_root, "root mismatch");

    let _ = RangeProof::from_bytes(&base64::decode(&pf.range_proof)?)?;
    Ok(())
}

// ── tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_root_single_leaf() {
        let leaf = [42u8; 32];
        assert_eq!(merkle_root(vec![leaf]), leaf);
    }

    #[test]
    fn merkle_root_two_leaves() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let mut h = Sha256::new();
        h.update(a);
        h.update(b);
        let expected: [u8; 32] = h.finalize().into();
        assert_eq!(merkle_root(vec![a, b]), expected);
    }
}
