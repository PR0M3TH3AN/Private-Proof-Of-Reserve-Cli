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

## Should we keep going with this architecture?

Yes. Offline signing via PSBT leverages a mature ecosystem, and Pedersen commitments plus Bulletproofs follow well-tested cryptographic designs. The remaining features for multisig, descriptor wallets and snapshot-at-height do not require an architectural overhaul.

## Suggested near-term roadmap

1. **Ownership verification** – implement signature extraction and on-verify checks; support `m-of-n` (1–2 dev‑days).
2. **Snapshot at declared height** – either pause a pruned node at `--height` or fetch an assumeUTXO hash (2–4 dev‑days).
3. **Finalize proof JSON v1** – add `proof_format`, `signing_type`, and `psbt_hash`; lock the field set (0.5 day).
4. **Automated regtest suite** – spin up `bitcoind`, auto-mine, generate a proof and verify it (1 day).
5. **Docs & README** – deployment guide, security model description and QR signer list (0.5 day).

When these land the project should be a viable MVP for exchanges or treasury desks to trial on testnet.
