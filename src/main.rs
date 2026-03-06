mod crypto;
mod keychain;
mod resolve;
mod store;

use clap::{Parser, Subcommand};
use std::os::unix::process::CommandExt;
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
    Init,

    /// Store a secret
    Set {
        /// Secret key (e.g. openai/api-key)
        key: String,
        /// Secret value (omit to read from stdin)
        value: Option<String>,
    },

    /// Retrieve a secret
    Get {
        /// Secret key
        key: String,
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

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => {
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

        Commands::Set { key, value } => {
            let store = load_store();
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

        Commands::Get { key } => {
            let store = load_store();
            let value = store.get(&key).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                std::process::exit(1);
            });
            print!("{value}");
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

            println!("{:<30} {:<20} {:<20}", "KEY", "CREATED", "UPDATED");
            println!("{}", "-".repeat(70));
            for (key, created, updated) in secrets {
                println!("{:<30} {:<20} {:<20}", key, created, updated);
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
    }
}
