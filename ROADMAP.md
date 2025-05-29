# Project Roadmap

This document outlines the current status of the Private Proof-of-Reserve CLI prototype and the recommended next steps.

## Overall assessment

| Aspect | What’s strong already | Gaps / caveats | Priority fixes |
| --- | --- | --- | --- |
| **Privacy** | • Pedersen commitments hide individual UTXO values.<br>• Bulletproof reveals only `total – min ≥ 0`.<br>• Merkle root hides which UTXOs are included. | • Still leaks **count** of UTXOs.<br>• Proof JSON exposes the commitment list. | • Consider batching commitments (e.g., hash-to-point of all UTXOs) to hide count. |
| **Security / key handling** | • PSBT round-trip keeps seeds on the hardware device.<br>• Works for single-sig and (with spec) multisig.<br>• No private data shown in CLI logs. | • Ownership-proof step not implemented yet.<br>• Snapshot is current UTXO set—no guarantee it matches the declared height. | **High:** implement ownership-proof verification and snapshot-at-height. |
| **Reproducible builds** | • `Cargo.lock` committed again → deterministic CI.<br>• Clean `.gitignore`. | — | — |
| **Extensibility** | • CLI sub-command design is clean; easy to add `build-psbt`, `attach-sigs`.<br>• Proof JSON is versioned. | • Versioning/upgrade path not encoded yet (add `"proof_format": 1`). | Medium. |
| **Usability** | • Works headless (`cargo run`) or with QR hardware.<br>• Detailed README planned. | • Animated QR in terminal may be slow on some shells.<br>• Need progress bars for large UTXO sets. | Low/Med. |
| **Performance** | • Pure Rust; no FFI headaches.<br>• Commitment & Merkle root linear in UTXO count. | • Range-proof verification still `TODO` (Bulletproofs verify cost ~O(n log n)).<br>• For thousands of UTXOs, QR frames explode. | Consider commitment aggregation. |
| **Compliance / auditability** | • Clear separation of cryptographic steps makes audit easier. | • Bulletproof gadget not audited; same for Merkle inclusion logic. | Engage external audit once code stabilizes. |

## Where we are right now

| Area | Status | Notes |
| --- | --- | --- |
| **Compiles & unit-tests** | ✅ | All code builds with `--locked`; the three sanity-tests pass. |
| **CLI UX** | ✅ | `generate`, `verify`, `build-psbt`, `attach-sigs` all run end-to-end (with a local Core node). |
| **Pedersen & Bulletproof plumbing** | ✅ (prototype) | Commitments + range-proof verify correctly. |
| **UTXO gathering** | ⚠️ | Uses `listunspent` → **current** UTXO set, not a historical snapshot at `height`. |
| **Ownership proof** | ⛔ | Currently just records the raw signature bytes; no validation logic. |
| **Merkle inclusion** | ⛔ | No membership paths; verifier re-hashes the full commitment list (trusts prover). |
| **PSBT flow** | ✅ (MVP) | Draft PSBT is built, QR chunking works, signatures merged back. |
| **Security / audit trail** | ⚠️ | No domain-separation tags in transcripts, no zeroization of secrets, no formal review. |
| **Dependencies** | ⚠️ | Bulletproofs `4.x`, curve25519-dalek-ng `4.1`: both a year behind upstream. |
| **Test coverage** | 🟡 | Only 3 unit tests; no property tests, no integration tests. |
| **Docs / README** | ✅ | High-level README, dev-setup instructions, and build badges present. |

## Should we keep going with this architecture?

Yes. Offline signing via PSBT leverages a mature ecosystem, and Pedersen commitments plus Bulletproofs follow well-tested cryptographic designs. The remaining features for multisig, descriptor wallets and snapshot-at-height do not require an architectural overhaul.

## Recommended next steps

| Priority | Task | Why it matters |
| -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| **1** | **Historical snapshot** – replace `listunspent` with a deterministic view at `block_height` (e.g. `gettxoutsetinfo` → assumeUTXO or a pruned node halted at that height). | Prevents a prover from “cheating” by spending after the snapshot. |
| **1** | **Ownership gadget** – decide on a signature-verification strategy (Schnorr in-circuit vs. OP_RETURN PSBT anchor) and implement verification logic. | Without it you only prove that *someone* had those UTXOs. |
| **1** | **Inclusion proofs** – emit Merkle branches, switch verifier to path-checking. | Makes the proof self-contained; verifier no longer has to trust the entire list. |
| **2** | **Upgrade deps** – Bulletproofs `5.0`, dalek `4.1.4` (or switch to upstream non-ng). | Brings in subtle 3.0 API and fixes small-scalar bugs. |
| **2** | **Transcript domain separation** – give each proof type its own label (`b"por-range"`, etc.). | Standard best practice to avoid cross-protocol attacks. |
| **2** | **Zeroize secrets** – wipe blinding factors & PSBT bytes after use (`zeroize` crate). | Limits key-material leakage. |
| **3** | **Property / fuzz tests** – proptest that any random set of UTXOs passes round-trip. | Hardens code against edge cases. |
| **3** | **CI matrix** – run GitHub Actions on stable, beta, nightly; include `cargo audit`. | Keeps supply-chain and MSRV healthy. |
| **3** | **Multisig support** – treat each PSBT input’s redeem script, gather all sigs, update ownership gadget. | Lets treasuries prove reserves without revealing co-signers. |

When these land the project should be a viable MVP for exchanges or treasury desks to trial on testnet.
