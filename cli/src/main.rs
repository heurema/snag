mod file;
mod submit;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "snag", version, about = "Hit a snag? File it.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Detect which product you're working in
    Detect {
        /// Override config file path
        #[arg(long)]
        config: Option<String>,
    },
    /// Check if a similar issue already exists
    Check {
        /// Issue title or keywords to search
        title: String,
        /// Target product (skip auto-detect)
        #[arg(long)]
        product: Option<String>,
        /// Config file path
        #[arg(long)]
        config: Option<String>,
    },
    /// File a bug report on the detected repo
    File {
        /// Non-interactive: auto-detect, auto-title from context
        #[arg(long)]
        auto: bool,
        /// Target product (skip auto-detect)
        #[arg(long)]
        product: Option<String>,
        /// Issue title
        #[arg(long)]
        title: Option<String>,
        /// Issue body (reads from stdin if absent)
        #[arg(long)]
        body: Option<String>,
        /// Skip duplicate check
        #[arg(long)]
        no_check: bool,
        /// Force: ignore duplicates and session limit
        #[arg(long)]
        force: bool,
        /// Config file path
        #[arg(long)]
        config: Option<String>,
    },
    /// Submit a saved bundle to GitHub
    Submit {
        /// Path to the bundle JSON file
        bundle_path: String,
        /// Skip confirmation prompt and session limit
        #[arg(long)]
        force: bool,
        /// Config file path
        #[arg(long)]
        config: Option<String>,
    },
    /// Initialize config for your org
    Init,
}

fn main() {
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Commands::Detect { config } => snag_lib::detect::run(config.as_deref()),
        Commands::Check {
            title,
            product,
            config,
        } => snag_lib::check::run(&title, product.as_deref(), config.as_deref()),
        Commands::File {
            auto,
            product,
            title,
            body,
            no_check,
            force,
            config,
        } => file::run(
            auto,
            product.as_deref(),
            title.as_deref(),
            body.as_deref(),
            no_check,
            force,
            config.as_deref(),
        ),
        Commands::Submit {
            bundle_path,
            force,
            config,
        } => submit::run(&bundle_path, force, config.as_deref()),
        Commands::Init => init_config(),
    };

    std::process::exit(exit_code);
}

fn init_config() -> i32 {
    let config_dir = snag_lib::registry::config_dir();
    let config_path = config_dir.join("config.toml");

    if config_path.exists() {
        eprintln!("Config already exists: {}", config_path.display());
        eprintln!("Edit it directly or delete to re-initialize.");
        return 1;
    }

    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("Cannot create config dir: {e}");
        return 1;
    }

    let template = r#"# snag configuration
# Docs: https://github.com/heurema/snag

[org]
name = "myorg"
github = "myorg"        # GitHub org or user

# Define products manually or run `gh repo list <org>` to populate:
#
# [[products]]
# name = "myproject"
# repo = "myproject"    # defaults to name
# markers = ["Cargo.toml:myproject", ".myproject/"]

[settings]
max_issues_per_session = 5
"#;

    if let Err(e) = std::fs::write(&config_path, template) {
        eprintln!("Cannot write config: {e}");
        return 1;
    }

    println!("Created: {}", config_path.display());
    println!("Edit [org].github, add [[products]], then `snag detect` to test.");
    0
}
