# Private Proof‑of‑Reserve CLI

> **Status – prototype / PoC**  ⚠️  Not production‑ready.

A single‑binary command‑line tool that lets any Bitcoin holder prove **privately** that they control *at least* `X` satoshis—without revealing their address, UTXOs, or exact balance.  A verifier only needs a fully‑synced Bitcoin Core node.

---

\## How it works (high level)

| Step | Role         | What happens                                                                                                                                     |
| ---- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------ |
|  1   | **Prover**   | Connects to their own Bitcoin Core node (RPC) and fetches the current UTXOs for the address.                                                     |
|  2   |              | Creates **Pedersen commitments** to each UTXO value plus a blinding factor.                                                                      |
|  3   |              | Computes a simple SHA‑256 Merkle root of those commitments. *(Prototype skips real inclusion proofs.)*                                           |
|  4   |              | Builds a **Bulletproof** on `(Σ value − min)` to prove the total balance is ≥ `min` sats without revealing the extra amount.                     |
|  5   |              | (future) Adds ZK proofs that each commitment is *in* the UTXO set *and* that the prover controls the keys.                                       |
|  6   |              | Serialises everything to a portable JSON file (`proof.json`).                                                                                    |
|  7   | **Verifier** | Loads the JSON, recomputes the Merkle root, parses the Bulletproof, checks the snapshot height, and (future) checks membership/ownership proofs. |

> **Security note** – Items marked *future* are essential for real security.  The current PoC hides values but still reveals the commitment list, and it trusts the prover’s word about set membership & ownership.

---

\## Build instructions

```bash
# Clone & build an optimised binary
cargo build --release
# (Optional) copy to PATH
sudo install -m755 target/release/private_proof_of_reserve_cli /usr/local/bin/por
```

---

\## Running a local Bitcoin node

Add this to your `bitcoin.conf` (mainnet) and restart **bitcoind**:

```
server=1
rpcuser=alice
rpcpassword=supersecret
txindex=1        # required so listunspent sees every tx
```

For quick testing, spin up **regtest**:

```bash
bitcoind -regtest -txindex -daemon -rpcuser=alice -rpcpassword=secret
bitcoin-cli -regtest -generate 150   # mature some coinbase blocks
```

---

\## Usage

### 1  Generate a proof

```bash
por generate \
  --rpc-url  http://127.0.0.1:8332 \
  --rpc-user alice \
  --rpc-pass supersecret \
  --address  bc1qexample... \
  --sk       KxPrivateKeyWIF... \
  --min      10000000        # 0.1 BTC
  --height   840000          # snapshot reference
  --out      my_proof.json
```

Outputs `my_proof.json` with:

```json
{
  "block_height": 840000,
  "utxo_root": "...",
  "commitments": ["..."],
  "range_proof": "BASE64...",
  "ownership_proofs": [],
  "min_amount": 10000000
}
```

\### 2  Verify a proof

```bash
por verify \
  --proof    my_proof.json \
  --rpc-url  http://127.0.0.1:8332 \
  --rpc-user bob \
  --rpc-pass secret
```

If all checks pass you’ll see `✅ proof verified`.

---

\## Design notes & roadmap

* **Snapshot at height** – PoC uses the *current* UTXO set; production must freeze or attest to a deterministic snapshot (e.g. assumeUTXO + block hash).
* **Merkle inclusion proofs** – each commitment needs a path to the UTXO‑set root so the verifier can rebuild the same Merkle root independently.
* **Ownership proofs** – Schnorr signatures inside or alongside the ZK circuit to prove control of the pubkeys that guard the UTXOs.
* **Full Bulletproof verification** – currently we only parse the bytes; enable full verification once the commitment‑diff scheme is finalised.
* **Base‑64 engine upgrade** – swap deprecated `base64::encode/decode` for the modern *engine* API.
* **Optional Halo 2 backend** – design allows dropping in a different ZKP backend with Cargo `--features halo2-backend` later.

---

\## Directory layout

```
.
├── Cargo.toml
└── src
    └── main.rs   # single‑binary CLI
```

---

\## License

MIT OR Apache‑2.0 – choose whichever works best for you.

---

\### Contact / contributions

PRs, issues, and discussions are welcome.  Feel free to fork, experiment, and
extend – especially around snapshotting, membership circuits, and audits.
