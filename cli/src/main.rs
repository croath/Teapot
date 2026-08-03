//! Teaport CLI — start the ChatGPT/Claude compatible API server.

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use teaport_core::{
  Config, ModelSource, default_config_paths, discover_models, list_agent_infos, serve,
};

#[derive(Debug, Parser)]
#[command(
  name = "teaport",
  about = "Teaport: expose local agent CLIs (codex, claude, grok, antigravity-cli) as ChatGPT/Claude compatible HTTP APIs",
  version
)]
struct Cli {
  #[command(subcommand)]
  command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
  /// Start the HTTP API server
  Serve {
    /// Path to TOML config file
    #[arg(short, long, env = "TEAPORT_CONFIG")]
    config: Option<PathBuf>,

    /// Listen address (overrides config)
    #[arg(short, long, env = "TEAPORT_LISTEN")]
    listen: Option<String>,

    /// API key (overrides config)
    #[arg(long, env = "TEAPORT_API_KEY")]
    api_key: Option<String>,

    /// Default agent name (overrides config)
    #[arg(long, env = "TEAPORT_DEFAULT_AGENT")]
    default_agent: Option<String>,

    /// Pin to a single agent CLI: routes traffic to it and loads models only from it
    #[arg(short = 'a', long = "agent", env = "TEAPORT_AGENT")]
    agent: Option<String>,
  },

  /// Print the default configuration as TOML
  DefaultConfig,

  /// List configured agents (from config or defaults)
  Agents {
    #[arg(short, long, env = "TEAPORT_CONFIG")]
    config: Option<PathBuf>,
  },

  /// List models discovered from installed agent CLIs
  Models {
    #[arg(short, long, env = "TEAPORT_CONFIG")]
    config: Option<PathBuf>,

    /// Only list models for this agent CLI
    #[arg(short = 'a', long = "agent", env = "TEAPORT_AGENT")]
    agent: Option<String>,
  },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  init_tracing();

  let cli = Cli::parse();
  match cli.command {
    Commands::Serve {
      config,
      listen,
      api_key,
      default_agent,
      agent,
    } => {
      let mut cfg = load_config(config)?;
      if let Some(l) = listen {
        cfg.listen = l;
      }
      if let Some(k) = api_key {
        cfg.api_key = Some(k);
      }
      if let Some(a) = default_agent {
        cfg.default_agent = a;
      }
      if let Some(a) = agent {
        cfg
          .set_active_agent(a)
          .context("invalid --agent / TEAPORT_AGENT")?;
        tracing::info!(
          agent = %cfg.effective_default_agent(),
          "pinned to agent CLI; models will load from this agent only"
        );
      }
      serve(cfg).await.context("server exited with error")?;
    }
    Commands::DefaultConfig => {
      let cfg = Config::default();
      let toml = toml::to_string_pretty(&cfg).context("serialize default config")?;
      println!("{toml}");
    }
    Commands::Agents { config } => {
      let cfg = load_config(config)?;
      println!(
        "{:<20} {:<10} {:<24} {}",
        "NAME", "INSTALLED", "COMMAND", "DESCRIPTION"
      );
      println!("{}", "-".repeat(80));
      for info in list_agent_infos(&cfg) {
        println!(
          "{:<20} {:<10} {:<24} {}",
          info.name,
          if info.installed { "yes" } else { "no" },
          info.command,
          info.description
        );
      }
      println!();
      println!("default_agent = {}", cfg.default_agent);
      if let Some(a) = &cfg.active_agent {
        println!("active_agent  = {a}");
      }
    }
    Commands::Models { config, agent } => {
      let mut cfg = load_config(config)?;
      if let Some(a) = agent {
        cfg
          .set_active_agent(a)
          .context("invalid --agent / TEAPORT_AGENT")?;
      }
      let models = discover_models(&cfg).await;
      if models.is_empty() {
        println!("No models found (no matching agent CLI installed on PATH).");
        return Ok(());
      }
      if let Some(a) = &cfg.active_agent {
        println!("# active agent: {a}");
      }
      println!(
        "{:<36} {:<16} {:<12} {}",
        "MODEL", "AGENT", "SOURCE", "DISPLAY"
      );
      println!("{}", "-".repeat(96));
      for m in models {
        let source = match m.source {
          ModelSource::Agent => "agent",
          ModelSource::Alias => "alias",
          ModelSource::CliProbe => "cli",
          ModelSource::Builtin => "builtin",
        };
        println!(
          "{:<36} {:<16} {:<12} {}",
          m.id, m.agent, source, m.display_name
        );
      }
    }
  }

  Ok(())
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
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,teaport_core=debug"));
  tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_target(true)
    .compact()
    .init();
}
