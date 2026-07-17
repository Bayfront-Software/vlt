# vlt

[日本語](README.ja.md)

Lightweight secret manager for AI developers.

Stop scattering API keys across `.env` files. `vlt` stores secrets in an encrypted local vault and resolves `vlt://` references at runtime — nothing sensitive touches disk or git history.

## Features

- **AES-256-GCM** encrypted local vault (SQLite)
- **OS Keychain** integration for master key storage (macOS Keychain)
- **`vlt://` reference scheme** — use references in env vars, resolve at runtime
- **Zero config** — single binary, no daemon, no cloud account required
- **Shell integration** — `eval "$(vlt env)"` for seamless workflow

## Install

### From source (requires Rust 1.70+)

```bash
cargo install --path .
```

### Build from GitHub

```bash
git clone https://github.com/Bayfront-Software/vlt.git
cd vlt
cargo build --release
cp target/release/vlt ~/.cargo/bin/
```

## Quick Start

```bash
# Initialize vault (stores master key in OS Keychain)
vlt init

# Store secrets
vlt set openai/api-key "sk-..."
vlt set anthropic/api-key "sk-ant-..."

# Pipe from another command
cat key.txt | vlt set github/token

# Retrieve a secret
vlt get openai/api-key

# List all keys
vlt list

# Delete a secret
vlt delete openai/api-key
```

## Usage

### Run with resolved secrets

Set env vars to `vlt://` references, and `vlt run` resolves them in-memory before executing your command:

```bash
OPENAI_API_KEY="vlt://openai/api-key" vlt run -- python app.py
```

The child process receives the real secret value. The reference never leaves your shell config.

### Shell integration

Add to your `~/.zshrc` or `~/.bashrc`:

```bash
export OPENAI_API_KEY="vlt://openai/api-key"
export ANTHROPIC_API_KEY="vlt://anthropic/api-key"
```

Then launch any tool through `vlt run`:

```bash
vlt run -- claude
vlt run -- python train.py
```

Or export all resolved secrets into your current shell:

```bash
eval "$(vlt env)"
```

## Binary secrets & portable backup

Store whole files (signing keys, certificates) as binary secrets:

```bash
# Store a file
vlt set android/upload-keystore --file ~/keystores/upload.jks

# Restore it (binary secrets require --out)
vlt get android/upload-keystore --out ./upload.jks
```

The vault's master key never leaves the OS Keychain, so a copy of
`vault.db` alone is **not** a backup. To make a machine-independent
backup, export a passphrase-encrypted `.vltx` bundle:

```bash
# Export everything (prompts for a passphrase; argon2id + AES-256-GCM)
vlt export backup.vltx

# Restore on any machine (existing keys are skipped unless --overwrite)
vlt import backup.vltx --overwrite
```

`VLT_PASSPHRASE` supplies the passphrase non-interactively for scripts,
and `VLT_DB` overrides the vault path (useful to verify a restore into a
scratch vault before trusting a backup).

`vlt init` refuses to overwrite an existing master key — re-initializing
would make the current vault permanently unreadable. Use `vlt init --force`
only after exporting.

## How It Works

```
┌──────────────────────────────────────┐
│  vlt CLI                             │
│  init / set / get / run / env        │
├──────────────────────────────────────┤
│  Resolve Engine                      │
│  Scans env for vlt:// references     │
│  Replaces in-memory only             │
├──────────────┬───────────────────────┤
│  Master Key  │  Encrypted Vault      │
│  OS Keychain │  SQLite + AES-256-GCM │
└──────────────┴───────────────────────┘
```

1. `vlt init` generates a 256-bit master key and stores it in the OS Keychain
2. `vlt set` encrypts each secret value with AES-256-GCM (unique nonce per value) and stores it in a local SQLite database
3. `vlt run` scans environment variables for `vlt://` prefixes, decrypts the referenced secrets, and passes them to the child process via `exec`
4. Secrets exist in plaintext only in process memory — never on disk, never in git

## Security Model

| Layer | Implementation |
|---|---|
| Encryption | AES-256-GCM with random 12-byte nonce per value |
| Key storage | macOS Keychain (protected by system auth) |
| Vault storage | `~/Library/Application Support/vlt/vault.db` |
| In transit | Secrets only in process memory, passed via env to child |

## License

MIT

## Contributing

Contributions are welcome. Please open an issue first to discuss what you'd like to change.
