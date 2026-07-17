mod crypto;
mod keychain;
mod portable;
mod resolve;
mod store;

use clap::{Parser, Subcommand};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use store::SecretStore;

#[derive(Parser)]
#[command(name = "vlt", version, about = "Lightweight secret manager for AI developers")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize vault and store master key in OS Keychain
    Init {
        /// Replace an existing master key (DANGER: existing vault becomes unreadable)
        #[arg(long)]
        force: bool,
    },

    /// Store a secret
    Set {
        /// Secret key (e.g. openai/api-key)
        key: String,
        /// Secret value (omit to read from stdin, or use --file)
        value: Option<String>,
        /// Read the value from a file (stored as binary)
        #[arg(long, conflicts_with = "value")]
        file: Option<PathBuf>,
    },

    /// Retrieve a secret
    Get {
        /// Secret key
        key: String,
        /// Write the value to a file (required to restore binary secrets)
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Delete a secret
    #[command(alias = "rm")]
    Delete {
        /// Secret key
        key: String,
    },

    /// List all stored secrets (keys only)
    #[command(alias = "ls")]
    List,

    /// Run a command with vlt:// env vars resolved
    Run {
        /// Command and arguments
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },

    /// Output shell export statements for resolved secrets
    Env,

    /// Export all secrets to a passphrase-encrypted portable backup (.vltx)
    Export {
        /// Output file path (e.g. backup.vltx)
        output: PathBuf,
    },

    /// Import secrets from a .vltx backup into this vault
    Import {
        /// Backup file path
        input: PathBuf,
        /// Overwrite keys that already exist in this vault
        #[arg(long)]
        overwrite: bool,
    },
}

fn load_store() -> SecretStore {
    let key_bytes = keychain::load_master_key().unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1);
    });
    let master_key: [u8; 32] = key_bytes.try_into().unwrap_or_else(|_| {
        eprintln!("Error: Invalid master key length in Keychain. Run `vlt init` again.");
        std::process::exit(1);
    });
    SecretStore::open(master_key).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        std::process::exit(1);
    })
}

/// パスフレーズを取得する。非対話環境(スクリプト)では VLT_PASSPHRASE を使う。
fn read_passphrase(confirm: bool) -> String {
    if let Ok(p) = std::env::var("VLT_PASSPHRASE") {
        if !p.is_empty() {
            return p;
        }
    }
    let first = rpassword::prompt_password("Passphrase: ").unwrap_or_else(|e| {
        eprintln!("Error reading passphrase: {e}");
        std::process::exit(1);
    });
    if first.is_empty() {
        eprintln!("Error: passphrase must not be empty");
        std::process::exit(1);
    }
    if confirm {
        let second = rpassword::prompt_password("Confirm passphrase: ").unwrap_or_else(|e| {
            eprintln!("Error reading passphrase: {e}");
            std::process::exit(1);
        });
        if first != second {
            eprintln!("Error: passphrases do not match");
            std::process::exit(1);
        }
    }
    first
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { force } => {
            // 既存のマスターキーを黙って上書きすると、いまの vault が
            // 復号不能になる（実質データ全損）。明示的な --force を要求する。
            if keychain::load_master_key().is_ok() && !force {
                eprintln!(
                    "Error: A master key already exists in the OS Keychain.\n\
                     Re-initializing would make the current vault permanently unreadable.\n\
                     If you really want a fresh vault, run `vlt export` first, then `vlt init --force`."
                );
                std::process::exit(1);
            }
            let master_key = crypto::generate_master_key();
            keychain::store_master_key(&master_key).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            // Open store to create the database
            SecretStore::open(master_key).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            println!("Vault initialized. Master key stored in OS Keychain.");
        }

        Commands::Set { key, value, file } => {
            let store = load_store();
            if let Some(path) = file {
                let bytes = std::fs::read(&path).unwrap_or_else(|e| {
                    eprintln!("Error reading {}: {e}", path.display());
                    std::process::exit(1);
                });
                store.set_bytes(&key, &bytes, true).unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                });
                println!("Secret stored (binary, {} bytes): {key}", bytes.len());
                return;
            }
            let secret_value = match value {
                Some(v) => v,
                None => {
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_line(&mut buf)
                        .expect("Failed to read from stdin");
                    buf.trim_end().to_string()
                }
            };
            store.set(&key, &secret_value).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            println!("Secret stored: {key}");
        }

        Commands::Get { key, out } => {
            let store = load_store();
            let (bytes, binary) = store.get_bytes(&key).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            match out {
                Some(path) => {
                    std::fs::write(&path, &bytes).unwrap_or_else(|e| {
                        eprintln!("Error writing {}: {e}", path.display());
                        std::process::exit(1);
                    });
                    println!("Wrote {} bytes to {}", bytes.len(), path.display());
                }
                None => {
                    if binary {
                        eprintln!(
                            "Error: {key} is a binary secret. Use `vlt get {key} --out <path>`."
                        );
                        std::process::exit(1);
                    }
                    let value = String::from_utf8(bytes).unwrap_or_else(|e| {
                        eprintln!("Error: UTF-8 decode error: {e}");
                        std::process::exit(1);
                    });
                    print!("{value}");
                }
            }
        }

        Commands::Delete { key } => {
            let store = load_store();
            let deleted = store.delete(&key).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            if deleted {
                println!("Secret deleted: {key}");
            } else {
                eprintln!("Secret not found: {key}");
                std::process::exit(1);
            }
        }

        Commands::List => {
            let store = load_store();
            let secrets = store.list().unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });

            if secrets.is_empty() {
                println!("No secrets stored. Use `vlt set <key> <value>` to add one.");
                return;
            }

            println!("{:<38} {:<6} {:<20} {:<20}", "KEY", "TYPE", "CREATED", "UPDATED");
            println!("{}", "-".repeat(86));
            for (key, created, updated, binary) in secrets {
                let kind = if binary { "bin" } else { "text" };
                println!("{:<38} {:<6} {:<20} {:<20}", key, kind, created, updated);
            }
        }

        Commands::Run { cmd } => {
            let store = load_store();
            let resolved = resolve::resolve_env(&store).unwrap_or_else(|e| {
                eprintln!("Error resolving secrets: {e}");
                std::process::exit(1);
            });

            let program = &cmd[0];
            let args = &cmd[1..];

            let mut command = Command::new(program);
            command.args(args);

            for (name, value) in &resolved {
                command.env(name, value);
            }

            // exec replaces the current process (Unix only)
            let err = command.exec();
            eprintln!("Error: Failed to exec {program}: {err}");
            std::process::exit(1);
        }

        Commands::Env => {
            let store = load_store();
            let resolved = resolve::resolve_env(&store).unwrap_or_else(|e| {
                eprintln!("Error resolving secrets: {e}");
                std::process::exit(1);
            });

            for (name, value) in &resolved {
                let escaped = value.replace('\'', "'\\''");
                println!("export {name}='{escaped}'");
            }
        }

        Commands::Export { output } => {
            let store = load_store();
            let entries = store.export_entries().unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            if entries.is_empty() {
                eprintln!("Error: vault is empty, nothing to export");
                std::process::exit(1);
            }
            let passphrase = read_passphrase(true);
            let sealed = portable::seal(&entries, &passphrase).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            std::fs::write(&output, &sealed).unwrap_or_else(|e| {
                eprintln!("Error writing {}: {e}", output.display());
                std::process::exit(1);
            });
            println!(
                "Exported {} secrets to {} ({} bytes, argon2id + AES-256-GCM)",
                entries.len(),
                output.display(),
                sealed.len()
            );
        }

        Commands::Import { input, overwrite } => {
            let store = load_store();
            let data = std::fs::read(&input).unwrap_or_else(|e| {
                eprintln!("Error reading {}: {e}", input.display());
                std::process::exit(1);
            });
            let passphrase = read_passphrase(false);
            let entries = portable::unseal(&data, &passphrase).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            let (imported, skipped) = store
                .import_entries(&entries, overwrite)
                .unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                });
            println!("Imported {imported} secrets ({skipped} skipped; use --overwrite to replace)");
        }
    }
}
