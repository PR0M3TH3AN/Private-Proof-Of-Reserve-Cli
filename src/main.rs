#![forbid(unsafe_code)]
//! Prototype CLI for a lower-bound Bitcoin proof-of-reserve.
//! SECURITY:  demo code only.

use std::{fs::File, path::PathBuf, str::FromStr, time::Duration};
use anyhow::Result;
use bitcoin::{Address, Txid};
use bitcoin::address::NetworkChecked;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use clap::{Parser, Subcommand};
use curve25519_dalek_ng::scalar::Scalar;
use curve25519_dalek_ng::ristretto::CompressedRistretto;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use bulletproofs::{BulletproofGens, PedersenGens, RangeProof};
use merlin::Transcript;
use sha2::{Digest, Sha256};
use base64::{engine::general_purpose, Engine as _};
use qrcodegen::{QrCode, QrCodeEcc};
use base58::ToBase58;
use serde_json::json;

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
    AttachSigs {
        #[arg(long)] draft: PathBuf,
        #[arg(long)] signed_psbt: PathBuf,
        #[arg(long, default_value = "proof.json")]
        out: PathBuf,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct Proof {
    block_height: u64,
    utxo_root: String,
    commitments: Vec<String>,
    range_proof: String,
    diff_commitment: String,
    ownership_proofs: Vec<String>,
    min_amount: u64,
    #[serde(default)]
    psbt_hash: Option<String>,
    #[serde(default)]
    signing_type: Option<String>,
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Generate { rpc_url, rpc_user, rpc_pass, address, min, height, out } => {
            let proof = generate_proof(&rpc_url, &rpc_user, &rpc_pass,
                                       &address, min, height)?;
            serde_json::to_writer_pretty(File::create(out)?, &proof)?;
            println!("✅ proof generated ({} commitments)", proof.commitments.len());
        }
        Cmd::Verify { proof, rpc_url, rpc_user, rpc_pass } => {
            let pf: Proof = serde_json::from_reader(File::open(&proof)?)?;
            verify_proof(&pf, &rpc_url, &rpc_user, &rpc_pass)?;
            println!("✅ proof verified");
        }
        Cmd::BuildPsbt { rpc_url, rpc_user, rpc_pass, address, min, height, qr, psbt_out, draft_out } => {
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
    address: &str, min: u64, height: u64
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
    let (bp, diff_commit) = RangeProof::prove_single(
        &gens, &PedersenGens::default(), &mut t,
        total - min, &blind, 64)?;

    let root = merkle_root(commitments.clone());

    Ok(Proof {
        block_height: height,
        utxo_root: hex::encode(root),
        commitments: commitments.iter().map(hex::encode).collect(),
        range_proof: general_purpose::STANDARD.encode(bp.to_bytes()),
        diff_commitment: hex::encode(diff_commit.to_bytes()),
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

    let bp_bytes = general_purpose::STANDARD.decode(&pf.range_proof)?;
    let bp = RangeProof::from_bytes(&bp_bytes)?;
    let v_bytes = hex::decode(&pf.diff_commitment)?;
    let commitment = CompressedRistretto::from_slice(&v_bytes);
    let mut t = Transcript::new(b"por");
    bp.verify_single(&BulletproofGens::new(64, 1), &PedersenGens::default(), &mut t, &commitment, 64)?;
    Ok(())
}

// ── psbt workflow ────────────────────────────────────────────

fn build_psbt(rpc_url: &str, rpc_user: &str, rpc_pass: &str,
              address: &str, min: u64, height: u64) -> Result<(String, Proof)> {
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
    let (bp, diff_commit) = RangeProof::prove_single(
        &gens, &PedersenGens::default(), &mut t,
        total - min, &blind, 64)?;

    let root = merkle_root(commitments.clone());
    let merkle_hex = hex::encode(root);

    let inputs: Vec<serde_json::Value> = utxos.iter()
        .map(|u| json!({"txid": u.txid, "vout": u.vout}))
        .collect();
    let options = json!({"feeRate": 0, "changePosition": -1, "lockUnspents": true});
    let res: serde_json::Value = rpc.call(
        "walletcreatefundedpsbt",
        &[inputs.into(), json!({"data": merkle_hex.clone()}).into(), serde_json::Value::Null,
          options.into(), true.into()])?;
    let psbt_str = res["psbt"].as_str().unwrap().to_string();

    let psbt_bytes = base64::decode(&psbt_str)?;
    let mut hasher = Sha256::new();
    hasher.update(&psbt_bytes);
    let psbt_hash = hex::encode(hasher.finalize());

    let proof = Proof {
        block_height: height,
        utxo_root: merkle_hex,
        commitments: commitments.iter().map(hex::encode).collect(),
        range_proof: general_purpose::STANDARD.encode(bp.to_bytes()),
        diff_commitment: hex::encode(diff_commit.to_bytes()),
        ownership_proofs: Vec::new(),
        min_amount: min,
        psbt_hash: Some(psbt_hash),
        signing_type: Some("psbt-opreturn-v1".to_string()),
    };

    Ok((psbt_str, proof))
}

fn display_qr_frames(data: &str, fps: u64) -> Result<()> {
    let raw = data.as_bytes();
    let chunks: Vec<&[u8]> = raw.chunks(400).collect();
    println!("Building PSBT QR ({} frames, press Ctrl-C to abort) …", chunks.len());
    for chunk in chunks.iter() {
        let mut frame = Vec::new();
        frame.extend_from_slice(chunk);
        let text = frame.to_base58();
        let qr = QrCode::encode_text(&text, QrCodeEcc::Low)?;
        println!("{}", qr.to_string(true, 2));
        std::thread::sleep(Duration::from_millis(1000 / fps));
    }
    Ok(())
}

fn attach_sigs(proof: &mut Proof, psbt_str: &str) -> Result<()> {
    use std::str::FromStr;
    let psbt: bitcoin::psbt::Psbt = bitcoin::psbt::Psbt::from_str(psbt_str)?;

    // verify hash
    if let Some(ref expect) = proof.psbt_hash {
        let bytes = base64::decode(psbt_str)?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hex::encode(hasher.finalize());
        anyhow::ensure!(&digest == expect, "Signed PSBT does not match draft (hash mismatch)");
    }

    anyhow::ensure!(psbt.inputs.len() == proof.commitments.len(), "Signed PSBT structure altered; aborting.");

    proof.ownership_proofs.clear();
    for (idx, input) in psbt.inputs.iter().enumerate() {
        if let Some((_, sig)) = input.partial_sigs.iter().next() {
            proof.ownership_proofs.push(hex::encode(sig.to_vec()));
        } else if let Some(sig) = input.tap_key_sig {
            proof.ownership_proofs.push(hex::encode(sig.to_vec()));
        } else {
            anyhow::bail!("Signed PSBT lacks sig for input {}", idx);
        }
    }
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

    #[test]
    fn range_proof_verify_success() {
        let gens = BulletproofGens::new(64, 1);
        let pg = PedersenGens::default();
        let v = 42u64;
        let blind = Scalar::from(7u64);
        let mut t = Transcript::new(b"por");
        let (proof, commitment) = RangeProof::prove_single(&gens, &pg, &mut t, v, &blind, 64).unwrap();
        let mut vt = Transcript::new(b"por");
        assert!(proof.verify_single(&gens, &pg, &mut vt, &commitment, 64).is_ok());
    }

    #[test]
    fn range_proof_verify_fail() {
        let gens = BulletproofGens::new(64, 1);
        let pg = PedersenGens::default();
        let v = 42u64;
        let blind = Scalar::from(7u64);
        let mut t = Transcript::new(b"por");
        let (proof, _commitment) = RangeProof::prove_single(&gens, &pg, &mut t, v, &blind, 64).unwrap();
        // commitment for wrong value
        let wrong_commitment = pg.commit(Scalar::from(43u64), Scalar::from(1u64)).compress();
        let mut vt = Transcript::new(b"por");
        assert!(proof.verify_single(&gens, &pg, &mut vt, &wrong_commitment, 64).is_err());
    }
}
