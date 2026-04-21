mod alpm_handle;
mod callbacks;
mod commands;
mod journal;
mod pkgbuild;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nix::libc;

#[derive(Parser)]
#[command(name = "arch")]
#[command(about = "Sane Arch Linux package manager", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install packages (always syncs and upgrades first)
    #[command(visible_alias = "i")]
    Install {
        /// Package names to install
        #[arg(required = true)]
        packages: Vec<String>,

        /// Force reinstall even if package is current
        #[arg(long)]
        reinstall: bool,
    },

    /// Remove packages and their dependencies
    #[command(visible_alias = "r")]
    Remove {
        /// Package names to remove
        #[arg(required = true)]
        packages: Vec<String>,
    },

    /// Remove orphaned packages (installed as deps, no longer needed)
    #[command(visible_alias = "ar")]
    Autoremove,

    /// Upgrade all packages
    #[command(visible_alias = "u")]
    Upgrade,

    /// List installed packages
    #[command(visible_alias = "l")]
    List {
        /// Pattern to match (glob by default, e.g., 'linux' matches '*linux*')
        pattern: Option<String>,

        /// Match exact package name only
        #[arg(long)]
        exact: bool,

        /// Show only packages with available upgrades
        #[arg(long)]
        upgradable: bool,

        /// Show only orphaned packages (deps no longer needed)
        #[arg(long)]
        orphans: bool,

        /// Show only explicitly installed packages
        #[arg(long)]
        manual: bool,

        /// Show only external packages (AUR/manual, not in sync dbs)
        #[arg(long)]
        external: bool,
    },

    /// Search packages in sync databases
    #[command(visible_alias = "s")]
    Search {
        /// Pattern to search for
        pattern: String,

        /// Also search in package descriptions
        #[arg(long)]
        desc: bool,
    },

    /// Show package information
    Info {
        /// Package name
        package: String,
    },

    /// Show what packages a package depends on
    Needs {
        /// Package name
        package: String,
    },

    /// Show what installed packages depend on a package
    #[command(name = "needed-by")]
    NeededBy {
        /// Package name
        package: String,
    },

    /// Mark packages' install reason
    Mark {
        #[command(subcommand)]
        action: MarkAction,
    },

    /// List files owned by a package
    Files {
        /// Package name
        package: String,
    },

    /// Find which package owns a file
    Belongs {
        /// File path to search for
        path: String,
    },

    /// Search for packages that provide a file
    Provides {
        /// File pattern to search for
        pattern: String,

        /// Skip syncing file databases (use cached data)
        #[arg(long)]
        no_sync: bool,
    },

    /// Verify installed packages have all their files
    #[command(visible_alias = "v")]
    Verify {
        /// Optional package name to verify (default: all packages)
        package: Option<String>,

        /// Only print package names with issues
        #[arg(short, long)]
        quiet: bool,
    },

    /// Build a package from PKGBUILD (sandboxed)
    #[command(visible_alias = "b")]
    Build {
        /// Directory containing PKGBUILD (default: current directory)
        directory: Option<std::path::PathBuf>,

        /// Install package after building
        #[arg(short, long)]
        install: bool,
    },
}

#[derive(Subcommand)]
enum MarkAction {
    /// Mark packages as explicitly installed
    Manual {
        /// Package names
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Mark packages as dependencies (eligible for autoremove)
    Auto {
        /// Package names
        #[arg(required = true)]
        packages: Vec<String>,
    },
}

fn run_list_command(
    pattern: Option<String>,
    exact: bool,
    upgradable: bool,
    orphans: bool,
    manual: bool,
    external: bool,
) -> Result<()> {
    if upgradable {
        return commands::list::upgradable();
    }
    if orphans {
        return commands::list::orphans();
    }
    if manual {
        return commands::list::manual(pattern.as_deref());
    }
    if external {
        return commands::list::external();
    }
    commands::list::run(pattern.as_deref(), exact)
}

fn run_mark_command(action: MarkAction) -> Result<()> {
    match action {
        MarkAction::Manual { packages } => commands::remove::mark_manual(&packages),
        MarkAction::Auto { packages } => commands::remove::mark_auto(&packages),
    }
}

fn run_command(command: Commands) -> Result<()> {
    match command {
        Commands::Install {
            packages,
            reinstall,
        } => commands::install::run(&packages, reinstall),
        Commands::Remove { packages } => commands::remove::run(&packages),
        Commands::Autoremove => commands::remove::autoremove(),
        Commands::Upgrade => commands::install::upgrade(),
        Commands::List {
            pattern,
            exact,
            upgradable,
            orphans,
            manual,
            external,
        } => run_list_command(pattern, exact, upgradable, orphans, manual, external),
        Commands::Search { pattern, desc } => commands::search::run(&pattern, desc),
        Commands::Info { package } => commands::info::run(&package),
        Commands::Needs { package } => commands::depends::needs(&package),
        Commands::NeededBy { package } => commands::depends::needed_by(&package),
        Commands::Mark { action } => run_mark_command(action),
        Commands::Files { package } => commands::files::files(&package),
        Commands::Belongs { path } => commands::files::belongs(&path),
        Commands::Provides { pattern, no_sync } => commands::files::provides(&pattern, !no_sync),
        Commands::Verify { package, quiet } => commands::verify::run(quiet, package.as_deref()),
        Commands::Build { directory, install } => commands::build::run(directory, install),
    }
}

fn main() -> Result<()> {
    // Reset SIGPIPE to default behavior (silent exit) to avoid panics when piping to head/less
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    run_command(cli.command)
}
