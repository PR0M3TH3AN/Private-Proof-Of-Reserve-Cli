# AGENTS.md  — P-PoR Repository

## Overview
This project uses **three** automated LLM agents, executed by the
`openai-codex-ci` GitHub Action. All agents run with model **gpt-4o-mini**,
temperature **0.2**, and max_tokens **2048** unless overridden.

| ID  | Trigger                    | Role summary                            |
|-----|----------------------------|-----------------------------------------|
| BP  | `tickets/BP-*.yaml`        | Bulletproofs/Halo2 circuit generation   |
| SN  | `tickets/SN-*.yaml`        | Snapshot & Merkle-tree code gen         |
| CLI | `tickets/CLI-*.yaml`       | Rust CLI glue, UX, docs                 |

---

## 1. `BP`  — Bulletproofs / Circuit Agent
- **Prompt template:** `prompts/bp_system.txt`
- **Capabilities:** Generates/edits Rust in `ppor_circuit`, adds unit tests.
- **Secrets:** _None_. Works only with public crates; never sees private keys.
- **Failure handling:** On `cargo test` failure, workflow auto-retries once;
  if still failing, PR is labeled `codex-fail` for human triage.

## 2. `SN`  — Snapshot Agent
- **Prompt template:** `prompts/sn_system.txt`
- **Env vars exposed:**  
  - `BITCOIN_RPC_USER` & `BITCOIN_RPC_PASS` (masked in logs)  
- **Allowed paths:** `snapshot_tool/**`  
- **Forbidden:** Must not modify anything under `ppor_circuit/`.

## 3. `CLI` — Front-End Agent
- **Prompt template:** `prompts/cli_system.txt`
- **Tasks:** Clap plumbing, progress bars, error messages, docs/README.
- **Special lints:** Must keep `#![forbid(unsafe_code)]` at crate root.

---

## Prompt lifecycle
1. **Author** writes a YAML ticket → pushes to `tickets/`.
2. GitHub Action `codex-run.yaml` detects new file → picks agent via filename.
3. Action assembles:
   - `agents/<ID>.system.txt`
   - `tickets/<ID-XYZ>.yaml`
   - Code context snippets defined in ticket
4. Sends to OpenAI `/chat/completions`.
5. Creates PR `codex/<ID-XYZ>` with generated diff + `codex-output.log`.
6. Human reviewer approves or requests changes.

---

## Versioning & audit
- All prompt templates live in `agents/`, reviewed like source.
- Any change to an agent template **must** bump `version:` field in header
  and include a changelog entry (`CHANGELOG.md` under “Prompt updates”).
- For security review, include this file + `agents/` directory in audit
  bundle so cryptographers understand model exposure.

---

## Future agents
If we add a **docs agent** or **refactor bot**, append a new section above and
update the table. Keep the ID short and unique.
