#![forbid(unsafe_code)]
//! Prototype CLI for a lower-bound Bitcoin proof-of-reserve.
//! SECURITY:  demo code only.  Do **not** use with real funds until
//! every TODO is implemented and the code is audited.

use std::{fs::File, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Result, Context};
use base58::ToBase58;
use base64::{engine::general_purpose as b64, Engine as _};
use bitcoin::{Address, Txid};
use bitcoin::address::NetworkChecked;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use clap::{Parser, Subcommand};
use curve25519_dalek_ng::{ristretto::CompressedRistretto, scalar::Scalar};
use merlin::Transcript;
use qrcode::{QrCode, EcLevel, render::unicode};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

/// ─────────────────────────── CLI ────────────────────────────
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// One-shot: generate proof JSON (still asks for private key)
    Generate {
        #[arg(long)] rpc_url: String,
        #[arg(long)] rpc_user: String,
        #[arg(long)] rpc_pass: String,
        #[arg(long)] address: String,
        #[arg(long)] min: u64,
        #[arg(long)] height: u64,
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
    /// Verify proof JSON against a full node
    Verify {
        #[arg(long)] proof: PathBuf,
        #[arg(long)] rpc_url: String,
        #[arg(long)] rpc_user: String,
        #[arg(long)] rpc_pass: String,
    },
    /// Step 1 of PSBT flow: build unsigned PSBT (+ draft Proof)
    BuildPsbt {
        #[arg(long)] rpc_url: String,
        #[arg(long)] rpc_user: String,
        #[arg(long)] rpc_pass: String,
        #[arg(long)] address: String,
        #[arg(long)] min: u64,
        #[arg(long)] height: u64,
        #[arg(long, default_value_t = false)]
        qr: bool,
        #[arg(long, default_value = "unsigned.psbt")]
        psbt_out: PathBuf,
        #[arg(long, default_value = "draft.json")]
        draft_out: PathBuf,
    },
    /// Step 2 of PSBT flow: merge signed PSBT, finish proof
    AttachSigs {
        #[arg(long)] draft: PathBuf,
        #[arg(long)] signed_psbt: PathBuf,
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
}

/// ──────────────────── Proof JSON schema ─────────────────────
#[derive(Debug, Serialize, Deserialize)]
struct Proof {
    block_height: u64,
    utxo_root: String,
    commitments: Vec<String>,
    range_proof: String,
    diff_commitment: String,
    ownership_proofs: Vec<String>,
    min_amount: u64,
    // new metadata
    psbt_hash: Option<String>,
    signing_type: Option<String>,
}

/// ────────────────────────── main ────────────────────────────
fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Generate { rpc_url, rpc_user, rpc_pass, address, min, height, out } => {
            let proof = generate_proof(&rpc_url, &rpc_user, &rpc_pass, &address, min, height)?;
            serde_json::to_writer_pretty(File::create(out)?, &proof)?;
            println!("✅ proof generated ({} commitments)", proof.commitments.len());
        }
        Cmd::Verify { proof, rpc_url, rpc_user, rpc_pass } => {
            let pf: Proof = serde_json::from_reader(File::open(&proof)?)?;
            verify_proof(&pf, &rpc_url, &rpc_user, &rpc_pass)?;
            println!("✅ proof verified");
        }
        Cmd::BuildPsbt { rpc_url, rpc_user, rpc_pass, address, min, height,
                         qr, psbt_out, draft_out } => {
            let (psbt, draft) = build_psbt(&rpc_url, &rpc_user, &rpc_pass, &address, min, height)?;
            std::fs::write(&psbt_out, &psbt)?;
            serde_json::to_writer_pretty(File::create(&draft_out)?, &draft)?;
            println!("✅ PSBT and draft saved");
            if qr { display_qr_frames(&psbt, 5)?; }
        }
        Cmd::AttachSigs { draft, signed_psbt, out } => {
            let mut pf: Proof = serde_json::from_reader(File::open(&draft)?)?;
            let psbt = std::fs::read_to_string(&signed_psbt)?;
            attach_sigs(&mut pf, &psbt)?;
            serde_json::to_writer_pretty(File::create(out)?, &pf)?;
            println!("✅ proof completed");
        }
    }
    Ok(())
}

/// ── UTXO helpers ────────────────────────────────────────────
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
    let mut total = 0u64;
    let commits = utxos.iter().map(|u| {
        total += u.value;
        let blinder = Scalar::random(&mut rng);
        pg.commit(Scalar::from(u.value), blinder).compress().to_bytes()
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

/// ── generate (single-shot) ──────────────────────────────────
fn generate_proof(
    rpc_url: &str, rpc_user: &str, rpc_pass: &str,
    address: &str, min: u64, height: u64
) -> Result<Proof> {

    let rpc = Client::new(rpc_url, Auth::UserPass(rpc_user.into(), rpc_pass.into()))?;

    let addr_raw = Address::from_str(address)?;
    let addr_chk = addr_raw.clone().require_network(*addr_raw.network())?;

    let utxos = fetch_utxos(&rpc, &addr_chk)?;
    anyhow::ensure!(!utxos.is_empty(), "address has no UTXOs");

    let (commitments, total) = commit_utxos(&utxos);
    anyhow::ensure!(total >= min, "balance below threshold");

    let gens  = BulletproofGens::new(64, 1);
    let blinder = Scalar::random(&mut OsRng);
    let mut t = Transcript::new(b"por");
    let (bp, diff_commit) =
        RangeProof::prove_single(&gens, &PedersenGens::default(), &mut t, total - min, &blinder, 64)?;

    let root = merkle_root(commitments.clone());

    Ok(Proof {
        block_height: height,
        utxo_root: hex::encode(root),
        commitments: commitments.iter().map(hex::encode).collect(),
        range_proof: b64::STANDARD.encode(bp.to_bytes()),
        diff_commitment: hex::encode(diff_commit.to_bytes()),
        ownership_proofs: Vec::new(),
        min_amount: min,
        psbt_hash: None,
        signing_type: None,
    })
}

/// ── verify ──────────────────────────────────────────────────
fn verify_proof(pf: &Proof, rpc_url: &str, rpc_user: &str, rpc_pass: &str) -> Result<()> {
    let rpc = Client::new(rpc_url, Auth::UserPass(rpc_user.into(), rpc_pass.into()))?;
    anyhow::ensure!(rpc.get_block_count()? as u64 >= pf.block_height, "node behind");

    // Merkle root
    let mut leaves: Vec<[u8; 32]> = pf.commitments.iter().map(|h| {
        let mut b = [0u8; 32];
        b.copy_from_slice(&hex::decode(h).unwrap());
        b
    }).collect();
    anyhow::ensure!(hex::encode(merkle_root(leaves)) == pf.utxo_root, "root mismatch");

    // Range-proof
    let bp_bytes = b64::STANDARD.decode(&pf.range_proof)?;
    let bp = RangeProof::from_bytes(&bp_bytes)?;
    let commitment = CompressedRistretto::from_slice(
        &hex::decode(&pf.diff_commitment)?);
    let mut t = Transcript::new(b"por");
    bp.verify_single(&BulletproofGens::new(64, 1), &PedersenGens::default(),
                     &mut t, &commitment, 64)
      .context("range-proof failed")?;

    // ownership_proofs verification TODO

    Ok(())
}

/// ── PSBT flow – build unsigned PSBT ─────────────────────────
fn build_psbt(
    rpc_url: &str, rpc_user: &str, rpc_pass: &str,
    address: &str, min: u64, height: u64
) -> Result<(String, Proof)> {

    let rpc = Client::new(rpc_url, Auth::UserPass(rpc_user.into(), rpc_pass.into()))?;

    let addr_raw = Address::from_str(address)?;
    let addr_chk = addr_raw.clone().require_network(*addr_raw.network())?;

    let utxos = fetch_utxos(&rpc, &addr_chk)?;
    anyhow::ensure!(!utxos.is_empty(), "address has no UTXOs");

    let (commitments, total) = commit_utxos(&utxos);
    anyhow::ensure!(total >= min, "balance below threshold");

    let gens  = BulletproofGens::new(64, 1);
    let blinder = Scalar::random(&mut OsRng);
    let mut t = Transcript::new(b"por");
    let (bp, diff_commit) =
        RangeProof::prove_single(&gens, &PedersenGens::default(), &mut t, total - min, &blinder, 64)?;

    // Build dummy PSBT
    let root_hex = hex::encode(merkle_root(commitments.clone()));
    let inputs: Vec<_> = utxos.iter()
        .map(|u| json!({"txid": u.txid, "vout": u.vout}))
        .collect();
    let options = json!({"feeRate": 0, "changePosition": -1, "lockUnspents": true});
    let res: serde_json::Value = rpc.call(
        "walletcreatefundedpsbt",
        &[inputs.into(),                     // inputs
          json!({"data": root_hex.clone()}).into(), // outputs
          serde_json::Value::Null,           // locktime
          options.into(), true.into()])?;    // includeWatching

    let psbt_str = res["psbt"].as_str().context("rpc malformed")?.to_string();
    let psbt_hash = hex::encode(Sha256::digest(&b64::STANDARD.decode(&psbt_str)?));

    let proof = Proof {
        block_height: height,
        utxo_root: root_hex,
        commitments: commitments.iter().map(hex::encode).collect(),
        range_proof: b64::STANDARD.encode(bp.to_bytes()),
        diff_commitment: hex::encode(diff_commit.to_bytes()),
        ownership_proofs: Vec::new(),
        min_amount: min,
        psbt_hash: Some(psbt_hash),
        signing_type: Some("psbt-opreturn-v1".into()),
    };

    Ok((psbt_str, proof))
}

/// Show PSBT as animated QR in the terminal
fn display_qr_frames(data: &str, fps: u64) -> Result<()> {
    let raw = data.as_bytes();
    let chunks: Vec<&[u8]> = raw.chunks(400).collect();
    println!("Building PSBT QR ({} frames, Ctrl-C to abort)…", chunks.len());
    for chunk in chunks {
        let frame_txt = chunk.to_base58();
        let qr = QrCode::with_error_correction_level(frame_txt, EcLevel::L)?;
        let art = qr.render::<unicode::Dense1x2>()
                     .quiet_zone(false)
                     .module_dimensions(2, 1)
                     .build();
        println!("{art}");
        std::thread::sleep(Duration::from_millis(1000 / fps));
    }
    Ok(())
}

/// Merge signed PSBT → fill ownership_proofs
fn attach_sigs(proof: &mut Proof, psbt_str: &str) -> Result<()> {
    use std::str::FromStr;
    let psbt: bitcoin::psbt::Psbt = bitcoin::psbt::Psbt::from_str(psbt_str)?;

    // hash-tamper check
    if let Some(expect) = &proof.psbt_hash {
        let digest = Sha256::digest(&b64::STANDARD.decode(psbt_str)?);
        anyhow::ensure!(hex::encode(digest) == *expect, "PSBT hash mismatch");
    }

    anyhow::ensure!(
        psbt.inputs.len() == proof.commitments.len(),
        "Signed PSBT structure altered"
    );

    proof.ownership_proofs.clear();
    for (idx, input) in psbt.inputs.iter().enumerate() {
        let sig_hex = if let Some((_, sig)) = input.partial_sigs.iter().next() {
            hex::encode(sig.to_vec())
        } else if let Some(sig) = input.tap_key_sig {
            hex::encode(sig.to_vec())
        } else {
            anyhow::bail!("Input {idx} lacks signature");
        };
        proof.ownership_proofs.push(sig_hex);
    }
    Ok(())
}

/// ── mini-tests ──────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merkle_root_single() {
        let leaf = [42u8; 32];
        assert_eq!(merkle_root(vec![leaf]), leaf);
    }

    #[test]
    fn merkle_root_two() {
        let a = [1u8; 32]; let b = [2u8; 32];
        let mut h = Sha256::new(); h.update(a); h.update(b);
        let exp: [u8; 32] = h.finalize().into();
        assert_eq!(merkle_root(vec![a, b]), exp);
    }

    #[test]
    fn range_ok() {
        let gens = BulletproofGens::new(64, 1);
        let pg   = PedersenGens::default();
        let mut t = Transcript::new(b"por");
        let (p, c) = RangeProof::prove_single(&gens, &pg, &mut t, 42u64, &Scalar::from(7u64), 64).unwrap();
        let mut vt = Transcript::new(b"por");
        assert!(p.verify_single(&gens, &pg, &mut vt, &c, 64).is_ok());
    }
}
