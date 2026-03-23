# nautilus-tee-app

A reference TEE application built on the [Nautilus](https://docs.sui.io/concepts/cryptography/nautilus) protocol by Mysten Labs. Use this as a starting point for building your own verifiable off-chain applications on Sui using AWS Nitro Enclaves.

> **Managed by [nautilus-ops](https://github.com/Ashwin-3cS/nautilus-cli)** — the CLI that handles building, deploying, attesting, and verifying this app end-to-end.

---

## What This Is

This is a sign server that runs inside an AWS Nitro Enclave. It:

- Generates an ephemeral Ed25519 keypair on startup (using NSM entropy in production)
- Signs arbitrary payloads with an intent-scoped message format compatible with Sui's on-chain verifier
- Exposes an attestation endpoint so the enclave's identity can be registered on-chain
- Is fully verifiable — any dApp on Sui can confirm a response came from this exact code image

This app is intentionally simple. Fork it and replace `sign_name` with your own business logic.

---

## Architecture

```
┌──────────────────────────────────┐
│         AWS Nitro Enclave        │
│                                  │
│  ┌────────────────────────────┐  │
│  │       sign-server          │  │
│  │  (Axum HTTP on port 4000)  │  │
│  │                            │  │
│  │  uses nautilus-enclave:    │  │
│  │  · EnclaveKeyPair          │  │
│  │  · get_attestation()       │  │
│  │  · kp.sign()               │  │
│  └────────────────────────────┘  │
│                                  │
│  VSOCK ←→ socat ←→ TCP:4000     │
└──────────────────────────────────┘
         ↕ VSOCK port 4000
┌──────────────────────────────────┐
│         EC2 Host (parent)        │
│  parent_forwarder.sh             │
│  TCP:4000 ←→ VSOCK:4000         │
└──────────────────────────────────┘
         ↕ HTTP
    CLI / dApp clients
```

---

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Ping — returns `"Nautilus TEE Sign Server Ready!"` |
| `GET` | `/health` | Returns enclave public key + status |
| `GET` | `/get_attestation` | Returns raw CBOR attestation document (hex) |
| `POST` | `/sign_name` | Signs a name with an intent-scoped message |
| `GET` | `/logs` | Returns recent log lines from the in-memory ring buffer |

### `GET /health`
```json
{
  "public_key": "abc123...",
  "status": "ok"
}
```

### `GET /get_attestation`
```json
{
  "attestation": "8443a10126a0590..."
}
```
The attestation contains the enclave's public key in the `public_key` field of the NSM document — this is what Sui's `nitro_attestation` module reads on-chain.

### `POST /sign_name`
```json
// Request
{ "name": "Alice" }

// Response
{
  "response": {
    "intent": 1,
    "timestamp_ms": 1700000000000,
    "data": {
      "name": "Alice",
      "message": "Hello Alice! This message was signed inside a Nitro Enclave."
    }
  },
  "signature": "deadbeef..."
}
```

The signature covers `bcs(IntentMessage { intent: u8, timestamp_ms: u64, data: SignedName })` — BCS-compatible with the on-chain Move verifier.

### `GET /logs`
```
GET /logs?lines=50
```
```json
{
  "lines": [
    "2025-03-23T10:00:01Z  INFO sign_server Starting sign-server...",
    "2025-03-23T10:00:01Z  INFO sign_server sign-server listening on 0.0.0.0:4000",
    "2025-03-23T10:00:05Z  INFO sign_server::common get_attestation called"
  ],
  "count": 3
}
```

Returns the most recent `lines` log entries (default: 100, max: 1000) from the in-memory ring buffer. Logs are captured via a custom `tracing` layer that writes to both stdout (for VSOCK streaming) and the buffer simultaneously.

Use from the CLI:
```bash
nautilus logs --host <EC2_IP> --template rust -n 50
nautilus logs --host <EC2_IP> --template rust --follow
```

---

## Using `nautilus-enclave`

This app uses [`nautilus-enclave`](https://github.com/Ashwin-3cS/nautilus-cli) — a library that handles all TEE crypto primitives. Add it to your own TEE app:

```toml
# Cargo.toml
[dependencies]
nautilus-enclave = { git = "https://github.com/Ashwin-3cS/nautilus-cli.git" }

[features]
default = []
aws = ["nautilus-enclave/nsm"]   # enable real NSM in production
```

```rust
use nautilus_enclave::{EnclaveKeyPair, get_attestation};

// Generate keypair — NSM entropy in enclave, OsRng locally
let kp = EnclaveKeyPair::generate();

// Sign any payload
let sig = kp.sign(&bcs_bytes);

// Get attestation doc (public key embedded in NSM document)
let att = get_attestation(&kp.public_key_bytes(), &[])?;
```

No fastcrypto boilerplate. No raw NSM driver calls. Works locally without any enclave setup — enable `--features aws` for production.

---

## Project Structure

```
nautilus-tee-app/
├── src/
│   ├── sign-server/        # Main TEE application (Axum HTTP server)
│   │   ├── src/
│   │   │   ├── main.rs     # Server setup, keypair generation, tracing layer
│   │   │   ├── lib.rs      # AppState, LogBuffer ring buffer, error types
│   │   │   └── common.rs   # Route handlers, signing logic, /logs endpoint
│   │   └── Cargo.toml
│   ├── init/               # Enclave init binary (bootstraps the enclave OS)
│   ├── aws/                # AWS helper utilities
│   └── system/             # System-level enclave helpers
├── run.sh                  # Entrypoint inside the enclave
├── parent_forwarder.sh     # Host-side VSOCK → TCP forwarder
├── Makefile                # Local dev + enclave build targets
└── Containerfile           # Docker image for enclave build
```

---

## Local Development

Run the sign server locally without any enclave or AWS setup:

```bash
cd src/sign-server
cargo run
```

Server starts at `http://localhost:4000`. All endpoints work — attestation returns a mock document in local mode.

Test it:
```bash
curl http://localhost:4000/health
curl http://localhost:4000/logs?lines=10
curl -X POST http://localhost:4000/sign_name \
  -H 'Content-Type: application/json' \
  -d '{"name": "Alice"}'
```

---

## Production Deployment (via nautilus-ops CLI)

The recommended way to deploy this app is using the [nautilus-ops CLI](https://github.com/Ashwin-3cS/nautilus-cli). It handles the full lifecycle:

```bash
# Install the CLI
cargo install --git https://github.com/Ashwin-3cS/nautilus-cli nautilus-cli --features aws

# 1. Initialize CI/CD config
nautilus init-ci

# 2. Build reproducible enclave image
nautilus build

# 3. Deploy to EC2 + launch enclave
nautilus deploy

# 4. Deploy on-chain contract
nautilus deploy-contract --network testnet

# 5. Update expected PCR values
nautilus update-pcrs

# 6. Register enclave attestation on-chain
nautilus register-enclave --host <EC2_IP>

# 7. Verify a signature end-to-end
nautilus verify-signature --host <EC2_IP> --name Alice
```

Refer to the [nautilus-ops README](https://github.com/Ashwin-3cS/nautilus-cli) for full setup instructions, AWS prerequisites, and Sui wallet configuration.

---

## On-Chain Verification

Once the enclave is registered on-chain via `nautilus register-enclave`, any Sui dApp can verify responses:

```move
// In your Move contract
use nautilus::enclave::{Enclave, verify_signature};
use nautilus::enclave::ENCLAVE;

public fun verify_response(
    enclave: &Enclave<ENCLAVE>,
    intent_scope: u8,
    timestamp_ms: u64,
    payload: SignedName,
    signature: vector<u8>,
): bool {
    verify_signature(enclave, intent_scope, timestamp_ms, payload, &signature)
}
```

Or directly from the CLI:
```bash
nautilus verify-signature --host <EC2_IP> --name Alice
# ✓ Signature verified on-chain
```

---

## Forking This App

To build your own TEE app on Nautilus:

1. Fork this repository
2. Replace `sign_name` in `src/sign-server/src/common.rs` with your business logic
3. Add your payload struct and intent scope
4. Keep `get_attestation`, `health_check`, and `get_logs` as-is — they're required by the CLI
5. Add the corresponding `verify_*` entry function to the Move contract
6. Deploy with `nautilus-ops`

The `nautilus-enclave` library, the intent message format, and the BCS signing pattern are all reusable across any TEE app on Nautilus.

---

## Related

- [nautilus-ops](https://github.com/Ashwin-3cS/nautilus-cli) — CLI + library + Move contracts
- [Nautilus Protocol Docs](https://docs.sui.io/concepts/cryptography/nautilus) — Mysten Labs
- [Sui Nitro Attestation](https://docs.sui.io/concepts/cryptography/nautilus) — on-chain attestation verification
