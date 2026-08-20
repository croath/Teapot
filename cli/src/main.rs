//! Teapot CLI — start the ChatGPT/Claude compatible API server and manage provider auth.

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use tracing_subscriber::EnvFilter;

use teapot_core::{
  AuthMethod, AuthStore, Config, LoginOptions, ProviderKind, all_providers, default_config_paths,
  ensure_legacy_migrated, import_service_account, provider_for, serve,
};

#[derive(Debug, Parser)]
#[command(
  name = "teapotx",
  about = "Teapot: expose local provider CLIs as ChatGPT/Claude compatible HTTP APIs",
  version
)]
struct Cli {
  #[command(subcommand)]
  command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
  /// Start the HTTP API server (single pinned provider)
  Serve {
    /// Provider to pin for this server process (required unless set in config)
    #[arg(short = 'p', long = "provider", env = "TEAPOT_PROVIDER")]
    provider: Option<String>,

    /// Path to TOML config file
    #[arg(short, long, env = "TEAPOT_CONFIG")]
    config: Option<PathBuf>,

    /// Listen address (overrides config)
    #[arg(short, long, env = "TEAPOT_LISTEN")]
    listen: Option<String>,

    /// API key (overrides config)
    #[arg(long, env = "TEAPOT_API_KEY")]
    api_key: Option<String>,
  },

  /// Print the default configuration as TOML
  DefaultConfig,

  /// List registered providers
  Providers {
    #[arg(short, long, env = "TEAPOT_CONFIG")]
    config: Option<PathBuf>,
  },

  /// Manage provider authentication (JSON under local app data)
  Auth {
    #[command(subcommand)]
    command: AuthCommands,
  },
}

#[derive(Debug, Subcommand)]
enum AuthCommands {
  /// Log in to a provider (OAuth browser, device code, or credential import)
  Login {
    /// Provider identity
    provider: CliProvider,

    /// Do not open a browser; print the URL instead
    #[arg(long)]
    no_browser: bool,

    /// Local OAuth callback port (browser flows)
    #[arg(long)]
    port: Option<u16>,

    /// Service-account JSON path (vertex only)
    #[arg(long, short = 'c')]
    credential: Option<PathBuf>,

    /// Vertex default location (e.g. us-central1)
    #[arg(long)]
    location: Option<String>,

    /// Vertex model-name prefix
    #[arg(long)]
    prefix: Option<String>,

    /// Override auth store path/dir
    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },

  /// List stored credentials
  List {
    provider: Option<CliProvider>,

    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },

  /// Show auth status for providers
  Status {
    provider: Option<CliProvider>,

    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },

  /// Remove stored credentials
  Logout {
    provider: CliProvider,

    /// Account label (email / subject); omit to remove all for the provider
    #[arg(long)]
    account: Option<String>,

    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },

  /// Refresh tokens for a provider (when near expiry)
  Refresh {
    provider: CliProvider,

    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },

  /// Print the auth store file path
  Path {
    #[arg(long, env = "TEAPOT_AUTH_DIR")]
    auth_dir: Option<PathBuf>,
  },
}

/// CLI provider selection (maps 1:1 to [`ProviderKind`]).
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliProvider {
  Codex,
  Claude,
  Xai,
  Antigravity,
  Vertex,
}

impl CliProvider {
  fn kind(self) -> ProviderKind {
    match self {
      Self::Codex => ProviderKind::Codex,
      Self::Claude => ProviderKind::Claude,
      Self::Xai => ProviderKind::Xai,
      Self::Antigravity => ProviderKind::Antigravity,
      Self::Vertex => ProviderKind::Vertex,
    }
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  init_tracing();
  match ensure_legacy_migrated() {
    Ok(dir) => tracing::debug!(dest = %dir.display(), "data local dir"),
    Err(error) => tracing::warn!(%error, "could not resolve data local dir"),
  }

  let cli = Cli::parse();
  match cli.command {
    Commands::Serve {
      provider,
      config,
      listen,
      api_key,
    } => {
      let mut cfg = load_config(config)?;
      if let Some(l) = listen {
        cfg.listen = l;
      }
      if let Some(k) = api_key {
        cfg.api_key = Some(k);
      }
      if let Some(p) = provider {
        cfg.set_provider(p);
      }
      if cfg.provider_name().is_none() {
        anyhow::bail!(
          "provider is required; use `teapotx serve -p <provider>` or set config.provider"
        );
      }
      tracing::info!(provider = ?cfg.provider_name(), "starting server");
      serve(cfg).await.context("server exited with error")?;
    }
    Commands::DefaultConfig => {
      let cfg = Config::default();
      let toml = toml::to_string_pretty(&cfg).context("serialize default config")?;
      println!("{toml}");
    }
    Commands::Providers { config } => {
      let cfg = load_config(config)?;
      println!(
        "{:<14} {:<10} {:<12} {:<16} {}",
        "NAME", "INSTALLED", "AUTH", "COMMAND", "DESCRIPTION"
      );
      println!("{}", "-".repeat(90));
      for p in all_providers() {
        let installed = if p.is_installed() { "yes" } else { "no" };
        let auth = match p.auth_method() {
          AuthMethod::None => "-",
          AuthMethod::BrowserOAuth => "oauth",
          AuthMethod::DeviceCode => "device",
          AuthMethod::CredentialImport => "import",
        };
        println!(
          "{:<14} {:<10} {:<12} {:<16} {}",
          p.kind().as_str(),
          installed,
          auth,
          p.command(),
          p.description()
        );
      }
      println!();
      println!("config provider = {:?}", cfg.provider_name());
    }
    Commands::Auth { command } => {
      run_auth(command).await?;
    }
  }

  Ok(())
}

async fn run_auth(cmd: AuthCommands) -> anyhow::Result<()> {
  match cmd {
    AuthCommands::Path { auth_dir } => {
      let store = open_store(auth_dir)?;
      println!("auth dir: {}", store.path().display());
      for kind in ProviderKind::ALL {
        let p = store.provider_path(*kind);
        let mark = if p.is_file() { "*" } else { " " };
        println!("  {mark} {}", p.display());
      }
    }
    AuthCommands::List { provider, auth_dir } => {
      let store = open_store(auth_dir)?;
      let providers = match provider {
        Some(p) => vec![provider_for(p.kind())],
        None => all_providers(),
      };
      let mut entries = Vec::new();
      for p in providers {
        entries.extend(p.load_auth(&store)?);
      }
      if entries.is_empty() {
        println!("(no credentials stored under {})", store.path().display());
        return Ok(());
      }
      println!(
        "{:<14} {:<28} {:<12} {:<22}",
        "PROVIDER", "ACCOUNT", "KIND", "EXPIRES"
      );
      println!("{}", "-".repeat(80));
      for e in entries {
        println!(
          "{:<14} {:<28} {:<12} {:<22}",
          e.provider().as_str(),
          e.account_key(),
          e.auth_kind(),
          e.expired().unwrap_or("-"),
        );
      }
    }
    AuthCommands::Status { provider, auth_dir } => {
      let store = open_store(auth_dir)?;
      let providers = match provider {
        Some(p) => vec![provider_for(p.kind())],
        None => all_providers(),
      };
      for p in providers {
        let entries = p.load_auth(&store)?;
        if entries.is_empty() {
          println!(
            "{:<14} not logged in  (auth={})",
            p.kind(),
            auth_method_label(p.auth_method())
          );
        } else {
          for e in entries {
            let need = if e.needs_refresh(chrono::Duration::minutes(5)) {
              "needs_refresh"
            } else {
              "ok"
            };
            println!(
              "{:<14} {}  account={}  expired={}  status={}",
              e.provider(),
              e.auth_kind(),
              e.account_key(),
              e.expired().unwrap_or("-"),
              need
            );
          }
        }
      }
    }
    AuthCommands::Login {
      provider,
      no_browser,
      port,
      credential,
      location,
      prefix,
      auth_dir,
    } => {
      let store = open_store(auth_dir)?;
      let kind = provider.kind();
      let p = provider_for(kind);
      if !p.supports_auth() {
        anyhow::bail!("provider `{kind}` does not support auth");
      }
      println!("Logging in to {kind}…");
      let entry = if matches!(p.auth_method(), AuthMethod::CredentialImport) {
        let path = credential.ok_or_else(|| {
          anyhow::anyhow!("provider `{kind}` requires --credential <service-account.json>")
        })?;
        import_service_account(&store, &path, location, prefix).await?
      } else {
        let opts = LoginOptions {
          no_browser,
          callback_port: port,
        };
        p.login(&store, opts).await?
      };
      let saved = store.provider_path(entry.provider());
      println!("Authentication saved to {}", saved.display());
      println!(
        "Authenticated as {} ({})",
        entry.account_key(),
        entry.provider()
      );
    }
    AuthCommands::Logout {
      provider,
      account,
      auth_dir,
    } => {
      let store = open_store(auth_dir)?;
      let kind = provider.kind();
      let p = provider_for(kind);
      let n = p.clear_auth(&store, account.as_deref())?;
      if n == 0 {
        println!("No credentials removed for {kind}");
      } else {
        println!("Removed {n} credential(s) for {kind}");
      }
    }
    AuthCommands::Refresh { provider, auth_dir } => {
      let store = open_store(auth_dir)?;
      let kind = provider.kind();
      let p = provider_for(kind);
      let entries = p.load_auth(&store)?;
      if entries.is_empty() {
        anyhow::bail!("no credentials for {kind}; run `teapotx auth login {kind}`");
      }
      for entry in entries {
        println!("Refreshing {} ({})…", entry.provider(), entry.account_key());
        let refreshed = p.refresh_auth(&store, &entry).await?;
        let path = p.save_auth(&store, &refreshed)?;
        println!(
          "  saved {}  expired={}",
          path.display(),
          refreshed.expired().unwrap_or("-")
        );
      }
    }
  }
  Ok(())
}

fn open_store(auth_dir: Option<PathBuf>) -> anyhow::Result<AuthStore> {
  match auth_dir {
    Some(dir) => {
      let store = AuthStore::new(dir);
      store.ensure_parent()?;
      Ok(store)
    }
    None => AuthStore::local().context("open local auth store"),
  }
}

fn auth_method_label(m: AuthMethod) -> &'static str {
  match m {
    AuthMethod::None => "none",
    AuthMethod::BrowserOAuth => "browser-oauth",
    AuthMethod::DeviceCode => "device-code",
    AuthMethod::CredentialImport => "credential-import",
  }
}

fn load_config(explicit: Option<PathBuf>) -> anyhow::Result<Config> {
  if let Some(path) = explicit {
    return Config::load_from_path(&path)
      .with_context(|| format!("load config from {}", path.display()));
  }
  for path in default_config_paths() {
    if path.exists() {
      tracing::info!(path = %path.display(), "loading config");
      return Config::load_from_path(&path)
        .with_context(|| format!("load config from {}", path.display()));
    }
  }
  Ok(Config::default())
}

fn init_tracing() {
  let filter =
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,teapot_core=debug"));
  tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_target(true)
    .compact()
    .init();
}
