use clap::{Parser, Subcommand};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use vlt::generator::{self, GeneratorOptions};
use vlt::item::{Field, FieldKind, Item, ItemType, NOTES_FIELD};
use vlt::store::SecretStore;
use vlt::{crypto, env as envname, keychain, portable, reference, runner, totp};

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

    /// Store a secret (updates the primary field of an existing item)
    Set {
        /// Item key (e.g. openai/api-key)
        key: String,
        /// Value (omit to read from stdin, or use --file)
        value: Option<String>,
        /// Read the value from a file (stored as a document)
        #[arg(long, conflicts_with = "value")]
        file: Option<PathBuf>,
        /// Field to write instead of the primary one (id or label)
        #[arg(long, conflicts_with = "file")]
        field: Option<String>,
        /// Item type when creating a new item (see `vlt types`)
        #[arg(long = "type")]
        item_type: Option<String>,
        /// Store the field as a plain (non-secret) value: shown as-is and not masked
        /// in `vlt run` output. For new fields and environment variables (e.g. PORT).
        #[arg(long, requires = "field")]
        plain: bool,
    },

    /// Print a secret (the primary field, or --field)
    Get {
        key: String,
        /// Field to print (id or label; `notes` for the notes)
        #[arg(long)]
        field: Option<String>,
        /// Write the value to a file (required for documents)
        #[arg(long)]
        out: Option<PathBuf>,
    },

    /// Resolve a secret reference like vlt://github/login/username
    Read {
        reference: String,
        /// Do not print a trailing newline
        #[arg(short = 'n', long)]
        no_newline: bool,
        /// Write the value to a file (mode 600) instead of stdout
        #[arg(short, long)]
        out: Option<PathBuf>,
    },

    /// Replace {{ vlt://... }} placeholders in a template
    Inject {
        /// Template file (stdin if omitted)
        #[arg(short, long)]
        input: Option<PathBuf>,
        /// Output file (stdout if omitted; created with mode 600)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Show an item's fields and their secret references
    Show {
        key: String,
        /// Print concealed values instead of ********
        #[arg(long)]
        reveal: bool,
        /// Machine-readable output (concealed values are null unless --reveal)
        #[arg(long)]
        json: bool,
    },

    /// Print the current one-time password of an item
    Totp { key: String },

    /// Generate a random password
    Generate {
        #[arg(long, default_value_t = 20)]
        length: usize,
        #[arg(long)]
        no_digits: bool,
        #[arg(long)]
        no_symbols: bool,
        /// Digits only
        #[arg(long)]
        pin: bool,
    },

    /// List item types and their fields
    Types,

    /// Move an item to the trash (restorable for 30 days)
    #[command(alias = "rm")]
    Delete {
        key: String,
        /// Delete permanently without the trash
        #[arg(long)]
        purge: bool,
    },

    /// Restore the most recently deleted item with this key
    Restore { key: String },

    /// List the trash
    Trash {
        /// Delete everything in the trash permanently
        #[arg(long)]
        empty: bool,
    },

    /// List all items (keys only)
    #[command(alias = "ls")]
    List {
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },

    /// Run a command with secrets injected as env vars.
    /// Secret values in the command's output are replaced with <concealed by vlt>
    /// when the output is not a terminal (logs, CI, AI agents).
    Run {
        /// Load variables from a .env file (values may be vlt:// references)
        #[arg(long = "env-file")]
        env_files: Vec<PathBuf>,
        /// Inject every variable of an environment item (see `vlt types`)
        #[arg(long = "env")]
        env_items: Vec<String>,
        /// Mask secrets even when the output is a terminal (breaks interactive programs)
        #[arg(long, conflicts_with = "no_masking")]
        mask: bool,
        /// Never mask secrets in the output
        #[arg(long)]
        no_masking: bool,
        /// Command and arguments
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
    },

    /// Output shell export statements for resolved references and environment items
    /// (for direnv: `eval "$(vlt env --env envs/myapp)"` in .envrc)
    Env {
        #[arg(long = "env-file")]
        env_files: Vec<PathBuf>,
        #[arg(long = "env")]
        env_items: Vec<String>,
    },

    /// Print a shell completion script (e.g. `vlt completions zsh > ~/.zfunc/_vlt`)
    Completions { shell: clap_complete::Shell },

    /// Export all items to a passphrase-encrypted portable backup (.vltx)
    Export { output: PathBuf },

    /// Import items from a .vltx backup into this vault
    Import {
        input: PathBuf,
        /// Overwrite keys that already exist in this vault
        #[arg(long)]
        overwrite: bool,
    },
}

fn die(message: impl std::fmt::Display) -> ! {
    eprintln!("Error: {message}");
    std::process::exit(1);
}

trait OrDie<T> {
    fn or_die(self) -> T;
}
impl<T, E: std::fmt::Display> OrDie<T> for Result<T, E> {
    fn or_die(self) -> T {
        self.unwrap_or_else(|e| die(e))
    }
}

fn load_store() -> SecretStore {
    let key_bytes = keychain::load_master_key().or_die();
    let master_key: [u8; 32] = key_bytes
        .try_into()
        .unwrap_or_else(|_| die("Invalid master key length in Keychain. Run `vlt init` again."));
    SecretStore::open(master_key).or_die()
}

/// パスフレーズを取得する。非対話環境(スクリプト)では VLT_PASSPHRASE を使う。
fn read_passphrase(confirm: bool) -> String {
    if let Ok(p) = std::env::var("VLT_PASSPHRASE") {
        if !p.is_empty() {
            return p;
        }
    }
    let first = rpassword::prompt_password("Passphrase: ").or_die();
    if first.is_empty() {
        die("passphrase must not be empty");
    }
    if confirm {
        let second = rpassword::prompt_password("Confirm passphrase: ").or_die();
        if first != second {
            die("passphrases do not match");
        }
    }
    first
}

fn read_stdin_line() -> String {
    let mut buf = String::new();
    std::io::stdin().read_line(&mut buf).or_die();
    buf.trim_end().to_string()
}

/// .env ファイル群と現在の環境変数を合わせる（ファイルの値が優先）。
fn collect_env(env_files: &[PathBuf]) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = std::env::vars().collect();
    for path in env_files {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| die(format!("{} を読めません: {e}", path.display())));
        for (name, value) in reference::parse_env_file(&text).or_die() {
            vars.retain(|(n, _)| n != &name);
            vars.push((name, value));
        }
    }
    vars
}

fn write_private_file(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

fn print_item(key: &str, item: &Item, reveal: bool) {
    println!("{key}  ({})", item.item_type.label());
    if item.favorite {
        println!("  ★ favorite");
    }
    let primary = item.item_type.primary_field();
    // 環境変数の項目はラベルが変数名なので、id ではなくラベルで見せる（参照もラベルで解決できる）
    let is_env = item.item_type == ItemType::Environment;
    let name_of = |f: &Field| if is_env { f.label.clone() } else { f.id.clone() };
    let width = item.fields.iter().map(|f| name_of(f).chars().count()).max().unwrap_or(0).max(5);
    for field in &item.fields {
        let shown = match field.kind {
            FieldKind::File => format!(
                "<file {} bytes: {}>",
                field.file_bytes().map(|b| b.len()).unwrap_or(0),
                field.filename.as_deref().unwrap_or("")
            ),
            FieldKind::Totp if !field.value.is_empty() => match totp::current_code(&field.value) {
                Ok(code) => format!("{} ({}s)", code.code, code.remaining),
                Err(e) => format!("<invalid: {e}>"),
            },
            _ if field.value.is_empty() => "-".to_string(),
            kind if kind.is_secret() && !reveal => "********".to_string(),
            _ => field.value.replace('\n', "\n      "),
        };
        let reference = if primary == Some(field.id.as_str()) {
            reference::build(key, None)
        } else {
            reference::build(key, Some(&name_of(field)))
        };
        println!("  {:<width$}  {shown}\n  {:<width$}  └ {reference}", name_of(field), "");
    }
    if !item.notes.is_empty() {
        println!("  {:<width$}  {}", NOTES_FIELD, item.notes.replace('\n', "\n      "));
    }
    if !item.tags.is_empty() {
        println!("  tags: {}", item.tags.join(", "));
    }
}

/// `vlt show --json`。伏せる種類の値は --reveal が無ければ null、ファイルは大きさだけ。
fn item_json(key: &str, item: &Item, reveal: bool) -> String {
    let primary = item.item_type.primary_field();
    let fields: Vec<serde_json::Value> = item
        .fields
        .iter()
        .map(|f| {
            let value = match f.kind {
                FieldKind::File => serde_json::Value::Null,
                kind if kind.is_secret() && !reveal => serde_json::Value::Null,
                _ => serde_json::Value::String(f.value.clone()),
            };
            let reference = if primary == Some(f.id.as_str()) {
                reference::build(key, None)
            } else if item.item_type == ItemType::Environment {
                reference::build(key, Some(&f.label))
            } else {
                reference::build(key, Some(&f.id))
            };
            serde_json::json!({
                "id": f.id,
                "label": f.label,
                "kind": f.kind,
                "value": value,
                "concealed": f.kind.is_secret(),
                "filename": f.filename,
                "size": (f.kind == FieldKind::File).then(|| f.file_bytes().map(|b| b.len()).unwrap_or(0)),
                "reference": reference,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "key": key,
        "type": item.item_type.as_str(),
        "fields": fields,
        "notes": item.notes,
        "tags": item.tags,
        "favorite": item.favorite,
    }))
    .expect("JSON 化は失敗しない")
}

fn main() {
    // Rust は SIGPIPE を無視するので、`vlt list | head` でパイプが閉じると
    // println! がパニックする。普通の CLI と同じく静かに終わらせる。
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { force } => {
            // 既存のマスターキーを黙って上書きすると、いまの vault が
            // 復号不能になる（実質データ全損）。明示的な --force を要求する。
            if keychain::load_master_key().is_ok() && !force {
                die("A master key already exists in the OS Keychain.\n\
                     Re-initializing would make the current vault permanently unreadable.\n\
                     If you really want a fresh vault, run `vlt export` first, then `vlt init --force`.");
            }
            let master_key = crypto::generate_master_key();
            keychain::store_master_key(&master_key).or_die();
            SecretStore::open(master_key).or_die();
            println!("Vault initialized. Master key stored in OS Keychain.");
        }

        Commands::Set { key, value, file, field, item_type, plain } => {
            let store = load_store();
            let new_type = item_type
                .as_deref()
                .map(|t| ItemType::parse(t).unwrap_or_else(|| die(format!("unknown type: {t} (see `vlt types`)"))));
            if let Some(path) = file {
                let bytes = std::fs::read(&path)
                    .unwrap_or_else(|e| die(format!("reading {}: {e}", path.display())));
                if !store.exists(&key).or_die() {
                    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut item = Item::new(ItemType::Document);
                    item.fields = vec![Field::file(&filename, &bytes)];
                    store.put_item(&key, &item).or_die();
                } else {
                    store.set_bytes(&key, &bytes, true).or_die();
                }
                println!("Stored (document, {} bytes): {key}", bytes.len());
                return;
            }
            let secret_value = value.unwrap_or_else(read_stdin_line);
            let exists = store.exists(&key).or_die();
            match (field, exists) {
                (None, true) => store.set(&key, &secret_value).or_die(),
                (None, false) => {
                    let mut item = Item::new(new_type.unwrap_or(ItemType::Password));
                    item.set_primary_text(&secret_value).or_die();
                    store.put_item(&key, &item).or_die();
                }
                (Some(name), _) => {
                    let mut item = if exists {
                        store.get_item(&key).or_die()
                    } else {
                        Item::new(new_type.unwrap_or(ItemType::Password))
                    };
                    if item.item_type == ItemType::Environment && !envname::is_env_name(&name) {
                        die(format!("{name} is not a valid environment variable name"));
                    }
                    if name == NOTES_FIELD && item.field(&name).is_none() {
                        item.notes = secret_value;
                    } else if let Some(f) = item.field_mut(&name) {
                        if f.kind == FieldKind::File {
                            die("use --file for file fields");
                        }
                        f.value = secret_value;
                        if plain {
                            f.kind = FieldKind::Text;
                        }
                    } else {
                        let id = item.unique_field_id(&name);
                        let kind = if plain { FieldKind::Text } else { FieldKind::Concealed };
                        item.fields.push(Field { value: secret_value, ..Field::new(&id, &name, kind) });
                    }
                    store.put_item(&key, &item).or_die();
                }
            }
            println!("Secret stored: {key}");
        }

        Commands::Get { key, field, out } => {
            let store = load_store();
            let bytes = match field {
                None => {
                    let (bytes, binary) = store.get_bytes(&key).or_die();
                    if binary && out.is_none() {
                        die(format!("{key} is a document. Use `vlt get {key} --out <path>`."));
                    }
                    bytes
                }
                Some(name) => {
                    let item = store.get_item(&key).or_die();
                    match item.field(&name) {
                        Some(f) if f.kind == FieldKind::File => {
                            if out.is_none() {
                                die(format!("{name} is a file. Use --out <path>."));
                            }
                            f.file_bytes().or_die()
                        }
                        Some(f) => f.value.clone().into_bytes(),
                        None if name == NOTES_FIELD => item.notes.clone().into_bytes(),
                        None => die(format!("{key} has no field {name} (see `vlt show {key}`)")),
                    }
                }
            };
            match out {
                Some(path) => {
                    write_private_file(&path, &bytes)
                        .unwrap_or_else(|e| die(format!("writing {}: {e}", path.display())));
                    println!("Wrote {} bytes to {}", bytes.len(), path.display());
                }
                None => print!("{}", String::from_utf8(bytes).or_die()),
            }
        }

        Commands::Read { reference: r, no_newline, out } => {
            if !reference::is_reference(&r) {
                die(format!("not a secret reference (must start with {}): {r}", reference::PREFIX));
            }
            let store = load_store();
            let value = reference::resolve_value(&store, &r).or_die();
            if let Some(path) = out {
                write_private_file(&path, value.as_bytes())
                    .unwrap_or_else(|e| die(format!("writing {}: {e}", path.display())));
                eprintln!("Wrote {}", path.display());
            } else if no_newline {
                print!("{value}");
            } else {
                println!("{value}");
            }
        }

        Commands::Inject { input, output } => {
            let template = match input {
                Some(path) => std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| die(format!("reading {}: {e}", path.display()))),
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf).or_die();
                    buf
                }
            };
            let store = load_store();
            let rendered = reference::inject(&store, &template).or_die();
            match output {
                Some(path) => {
                    write_private_file(&path, rendered.as_bytes())
                        .unwrap_or_else(|e| die(format!("writing {}: {e}", path.display())));
                    eprintln!("Wrote {}", path.display());
                }
                None => print!("{rendered}"),
            }
        }

        Commands::Show { key, reveal, json } => {
            let store = load_store();
            let item = store.get_item(&key).or_die();
            if json {
                println!("{}", item_json(&key, &item, reveal));
            } else {
                print_item(&key, &item, reveal);
            }
        }

        Commands::Totp { key } => {
            let store = load_store();
            let code = reference::resolve(
                &store,
                &reference::Reference { path: key, attribute: Some("otp".into()) },
            )
            .or_die();
            println!("{code}");
        }

        Commands::Generate { length, no_digits, no_symbols, pin } => {
            let password = generator::generate(&GeneratorOptions {
                length,
                digits: !no_digits,
                symbols: !no_symbols,
                pin,
            })
            .or_die();
            println!("{password}");
        }

        Commands::Types => {
            for t in ItemType::ALL {
                let fields: Vec<String> = t
                    .template()
                    .iter()
                    .map(|f| if t.primary_field() == Some(f.id.as_str()) { format!("*{}", f.id) } else { f.id.clone() })
                    .collect();
                println!("{:<17} {:<22} {}", t.as_str(), t.label(), fields.join(" "));
            }
            println!("\n* = primary field (what vlt://<key> resolves to)");
        }

        Commands::Delete { key, purge } => {
            let store = load_store();
            let found = if purge { store.purge(&key) } else { store.delete(&key) }.or_die();
            if !found {
                die(format!("Secret not found: {key}"));
            }
            if purge {
                println!("Permanently deleted: {key}");
            } else {
                println!("Moved to trash: {key}  (undo: vlt restore {key})");
            }
        }

        Commands::Restore { key } => {
            let store = load_store();
            let entry = store
                .list_trash()
                .or_die()
                .into_iter()
                .find(|t| t.key == key)
                .unwrap_or_else(|| die(format!("{key} is not in the trash")));
            store.restore(entry.id).or_die();
            println!("Restored: {key}");
        }

        Commands::Trash { empty } => {
            let store = load_store();
            if empty {
                store.empty_trash().or_die();
                println!("Trash emptied.");
                return;
            }
            let trash = store.list_trash().or_die();
            if trash.is_empty() {
                println!("Trash is empty.");
                return;
            }
            println!("{:<38} {:<18} DELETED (UTC)", "KEY", "TYPE");
            for t in trash {
                println!("{:<38} {:<18} {}", t.key, t.item_type.as_str(), t.deleted_at);
            }
            println!("\nItems are purged {} days after deletion.", vlt::store::TRASH_RETENTION_DAYS);
        }

        Commands::List { json } => {
            let store = load_store();
            let items = store.list_items().or_die();
            if json {
                let rows: Vec<serde_json::Value> = items
                    .iter()
                    .map(|i| serde_json::json!({
                        "key": i.key,
                        "type": i.item_type.as_str(),
                        "reference": reference::build(&i.key, None),
                        "created_at": i.created_at,
                        "updated_at": i.updated_at,
                    }))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&rows).or_die());
                return;
            }
            if items.is_empty() {
                println!("No secrets stored. Use `vlt set <key> <value>` to add one.");
                return;
            }
            println!("{:<38} {:<17} {:<20} {:<20}", "KEY", "TYPE", "CREATED", "UPDATED");
            println!("{}", "-".repeat(97));
            for item in items {
                println!(
                    "{:<38} {:<17} {:<20} {:<20}",
                    item.key,
                    item.item_type.as_str(),
                    item.created_at,
                    item.updated_at
                );
            }
        }

        Commands::Run { env_files, env_items, mask, no_masking, cmd } => {
            let store = load_store();
            let vars = reference::build_env(&store, collect_env(&env_files), &env_items).or_die();
            drop(store);
            let env: Vec<(String, String)> = vars.iter().map(|v| (v.name.clone(), v.value.clone())).collect();
            let secrets: Vec<String> = vars.iter().filter(|v| v.secret).map(|v| v.value.clone()).collect();
            let force = if mask { Some(true) } else if no_masking { Some(false) } else { None };
            let streams = runner::mask_streams(force, runner::is_tty(1), runner::is_tty(2));
            let has_secrets = secrets.iter().any(|s| s.len() >= vlt::mask::MIN_MASK_LEN);

            if !has_secrets || (!streams.stdout && !streams.stderr) {
                // 伏せる物が無ければ exec で置き換わる（端末・シグナルとも素のまま）
                let mut command = Command::new(&cmd[0]);
                command.args(&cmd[1..]).env_clear().envs(env);
                let err = command.exec();
                die(format!("Failed to exec {}: {err}", cmd[0]));
            }
            let code = runner::run_masked(
                &cmd,
                &env,
                &secrets,
                streams,
                Box::new(std::io::stdout()),
                Box::new(std::io::stderr()),
            )
            .or_die();
            std::process::exit(code);
        }

        Commands::Env { env_files, env_items } => {
            let store = load_store();
            let vars = reference::build_env(&store, collect_env(&env_files), &env_items).or_die();
            let mut exported: Vec<_> = vars.into_iter().filter(|v| v.from_vlt).collect();
            exported.sort_by(|a, b| a.name.cmp(&b.name));
            for v in exported {
                let escaped = v.value.replace('\'', "'\\''");
                println!("export {}='{escaped}'", v.name);
            }
        }

        Commands::Completions { shell } => {
            use clap::CommandFactory;
            clap_complete::generate(shell, &mut Cli::command(), "vlt", &mut std::io::stdout());
        }

        Commands::Export { output } => {
            let store = load_store();
            let entries = store.export_entries().or_die();
            if entries.is_empty() {
                die("vault is empty, nothing to export");
            }
            let passphrase = read_passphrase(true);
            let sealed = portable::seal(&entries, &passphrase).or_die();
            std::fs::write(&output, &sealed)
                .unwrap_or_else(|e| die(format!("writing {}: {e}", output.display())));
            println!(
                "Exported {} items to {} ({} bytes, argon2id + AES-256-GCM)",
                entries.len(),
                output.display(),
                sealed.len()
            );
        }

        Commands::Import { input, overwrite } => {
            let store = load_store();
            let data = std::fs::read(&input)
                .unwrap_or_else(|e| die(format!("reading {}: {e}", input.display())));
            let passphrase = read_passphrase(false);
            let entries = portable::unseal(&data, &passphrase).or_die();
            let (imported, skipped) = store.import_entries(&entries, overwrite).or_die();
            println!("Imported {imported} items ({skipped} skipped; use --overwrite to replace)");
        }
    }
}
