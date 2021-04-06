//! Inside the compiler, types may have certain suffixes to declare what information they contain and where they should be used:
//! - `P`: just been Parsed, no extra information has been deduced.
//!   No type has been deduced, and no effort has been made to ensure syntactic correctness
//!   past the (lenient) parser.
//! - `C`: an intermediate data Cache, used when we're still in the middle of computing the index.
//!   After the index has been computed, we should not need to use `P` or `C` data,
//!   only `I` data should be required. This type suffix is *internal to the `quill_index` crate*.
//! - `I`: an Index entry for the item.
//! - `T`: currently being type checked.
//! - `M`: part of the MIR intermediate representation.
//! - (no suffix): types have been deduced and references have been resolved.

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    process::Command,
};

use clap::ArgMatches;
use quill_target::{
    BuildInfo, TargetArchitecture, TargetEnvironment, TargetOS, TargetTriple, TargetVendor,
};
use quillc_api::QuillcInvocation;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

struct CliConfig {
    compiler_location: CompilerLocation,
}

/// Where is the Quill compiler stored?
/// By default, `quillc` and related tools are installed by the `quill` program, so it knows where to find them.
/// If we're running `quill` directly from `cargo`, we might instead want to use `cargo` to run `quillc`.
enum CompilerLocation {
    /// Runs the `quillc` whose source is in the given folder, by using `cargo`.
    Cargo { source: PathBuf },
}

impl CompilerLocation {
    fn invoke_quillc(&self, invocation: &QuillcInvocation) {
        let json = serde_json::to_string(invocation).unwrap();
        let mut command = match self {
            CompilerLocation::Cargo { source } => {
                let mut command = Command::new("cargo");
                command.current_dir(source);
                command.arg("run");
                command.arg("--bin");
                command.arg("quillc");
                // command.arg("-q");
                command.arg("--");
                command
            }
        };
        command.arg(json);
        let status = command.status().unwrap();
        if !status.success() {
            std::process::exit(1);
        }
    }
}

struct ProjectConfig {
    code_folder: PathBuf,
    build_folder: PathBuf,
}

fn error<S: Display>(message: S) -> ! {
    println!("{} {}", console::style("error:").red().bright(), message);
    std::process::exit(1);
}

fn main() {
    let yaml = clap::load_yaml!("cli.yml");
    let args = clap::App::from_yaml(yaml).get_matches();

    let cli_config = gen_cli_config(&args);

    match args.subcommand() {
        ("build", Some(sub_args)) => {
            process_build(&cli_config, &gen_project_config(&args), sub_args)
        }
        ("run", Some(sub_args)) => process_run(&cli_config, &gen_project_config(&args), sub_args),
        ("", _) => {
            clap::App::from_yaml(yaml).print_help().unwrap();
        }
        _ => {}
    }
}

fn gen_cli_config(args: &ArgMatches) -> CliConfig {
    let verbose = args.is_present("verbose");

    let log_level = if verbose { Some(Level::TRACE) } else { None };
    if let Some(log_level) = log_level {
        let subscriber = FmtSubscriber::builder().with_max_level(log_level).finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
        info!("initialised logging with verbosity level {}", log_level);
    }

    // TODO change this so that `quill` uses its own `quillc` rather than relying on being inside a source tree.
    let compiler_location = CompilerLocation::Cargo {
        source: Path::new(".").into(),
    };

    CliConfig { compiler_location }
}

fn gen_project_config(args: &ArgMatches) -> ProjectConfig {
    let provided_code_folder = args
        .value_of_os("project")
        .map_or_else(|| Path::new("."), Path::new);

    let code_folder = if let Ok(code_folder) = provided_code_folder.canonicalize() {
        code_folder
    } else {
        error(format!(
            "project folder '{}' did not exist",
            args.value_of_os("project")
                .map_or(".".to_string(), |os_str| os_str.to_string_lossy().into())
        ))
    };

    // Check that the code folder contains a `quill.toml` file.
    // TODO check parent directories to see if we're in a subfolder of a Quill project.
    if !code_folder.join("quill.toml").is_file() {
        error(format!(
            "project folder '{}' did not contain a 'quill.toml' file",
            code_folder.to_string_lossy()
        ))
    }

    std::fs::create_dir_all(code_folder.join("build")).unwrap();
    let build_folder = code_folder.join("build").canonicalize().unwrap();

    ProjectConfig {
        code_folder,
        build_folder,
    }
}

fn string_to_target(target: &str) -> TargetTriple {
    match target {
        "win" => TargetTriple {
            arch: TargetArchitecture::X86_64,
            vendor: TargetVendor::Pc,
            os: TargetOS::Windows,
            env: Some(TargetEnvironment::Msvc),
        },
        "linux" => TargetTriple {
            arch: TargetArchitecture::X86_64,
            vendor: TargetVendor::Unknown,
            os: TargetOS::Linux,
            env: Some(TargetEnvironment::Gnu),
        },
        other => clap::Error {
            message: format!(
                "'{}' was not a valid target, expected one of 'win', 'linux'",
                other
            ),
            kind: clap::ErrorKind::ValueValidation,
            info: None,
        }
        .exit(),
    }
}

#[cfg(target_family = "windows")]
fn default_target() -> TargetTriple {
    TargetTriple {
        arch: TargetArchitecture::X86_64,
        vendor: TargetVendor::Pc,
        os: TargetOS::Windows,
        env: Some(TargetEnvironment::Msvc),
    }
}

#[cfg(target_family = "unix")]
fn default_target() -> TargetTriple {
    TargetTriple {
        arch: TargetArchitecture::X86_64,
        vendor: TargetVendor::Unknown,
        os: TargetOS::Linux,
        env: Some(TargetEnvironment::Gnu),
    }
}

fn process_build(cli_config: &CliConfig, project_config: &ProjectConfig, args: &ArgMatches) {
    let targets_str = args.values_of_lossy("target");
    let targets = targets_str
        .map(|targets_str| {
            targets_str
                .into_iter()
                .map(|target| string_to_target(&target))
                .collect()
        })
        .unwrap_or_else(|| vec![default_target()]);

    for target_triple in targets {
        let build_info = BuildInfo {
            target_triple,
            code_folder: project_config.code_folder.clone(),
            build_folder: project_config.build_folder.clone(),
        };
        build(cli_config, project_config, build_info);
    }
}

fn process_run(cli_config: &CliConfig, project_config: &ProjectConfig, _args: &ArgMatches) {
    let info = BuildInfo {
        target_triple: default_target(),
        code_folder: project_config.code_folder.clone(),
        build_folder: project_config.build_folder.clone(),
    };
    run(cli_config, project_config, info);
}

fn build(cli_config: &CliConfig, project_config: &ProjectConfig, build_info: BuildInfo) {
    cli_config
        .compiler_location
        .invoke_quillc(&QuillcInvocation { build_info });
}

fn run(cli_config: &CliConfig, project_config: &ProjectConfig, build_info: BuildInfo) {
    build(cli_config, project_config, build_info.clone());
}
