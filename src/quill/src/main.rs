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
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use clap::ArgMatches;
use quill_common::{
    diagnostic::{Diagnostic, ErrorMessage, Severity},
    location::{Location, SourceFileIdentifier, SourceFileType},
};
use quill_source_file::{ErrorEmitter, PackageFileSystem};
use quill_target::{
    BuildInfo, TargetArchitecture, TargetEnvironment, TargetOS, TargetTriple, TargetVendor,
};
use quillc_api::{ProjectInfo, QuillcInvocation};
use tokio::process::Command;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod update;

pub struct CliConfig {
    verbose: bool,
    compiler_location: CompilerLocation,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum HostType {
    Linux,
    Windows,
}

impl HostType {
    /// Make the file have the right extension to be an executable on this platform.
    /// On windows, this adds the `.exe` extension.
    pub fn as_executable<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        match self {
            HostType::Linux => path.as_ref().to_owned(),
            HostType::Windows => path.as_ref().with_extension("exe"),
        }
    }

    /// Returns the component name prefix assigned to this host.
    /// Returns `ubuntu-latest` for Linux, `windows-latest` for Windows.
    pub fn component_prefix(&self) -> &'static str {
        match self {
            HostType::Linux => "ubuntu-latest",
            HostType::Windows => "windows-latest",
        }
    }
}

/// Where is the Quill compiler stored?
/// By default, `quillc` and related tools are installed by the `quill` program, so it knows where to find them.
/// If we're running `quill` directly from `cargo`, we might instead want to use `cargo` to run `quillc`.
enum CompilerLocation {
    /// Runs the `quillc` whose source is in the given folder, by using `cargo`.
    Cargo { source: PathBuf },
    /// The root dir contains `compiler-deps` and a folder for each component e.g. `quillc`, `quill_lsp`.
    /// If the host is windows, a `.exe` file extension is appended to executables.
    Installed { host: HostType, root: PathBuf },
}

impl CompilerLocation {
    async fn invoke_quillc(&self, verbose: bool, invocation: &QuillcInvocation) {
        let json = serde_json::to_string(invocation).unwrap();
        let mut command = match self {
            CompilerLocation::Cargo { source } => {
                let mut command = Command::new("cargo");
                command.current_dir(source);
                command.arg("run");
                command.arg("--bin");
                command.arg("quillc");
                if !verbose {
                    command.arg("-q");
                }
                command.arg("--");
                command
            }
            CompilerLocation::Installed { host, root } => {
                Command::new(host.as_executable(root.join("quillc").join("quillc")))
            }
        };
        command.arg(json);
        info!("Executing {:#?}", command);
        let status = command.status().await.unwrap();
        if !status.success() {
            std::process::exit(1);
        }
    }

    /// Where is the Zig compiler associated with this Quill compiler stored?
    fn zig_compiler(&self) -> PathBuf {
        match self {
            CompilerLocation::Cargo { .. } => {
                // Use the system Zig installation.
                PathBuf::from("zig")
            }
            CompilerLocation::Installed { root, .. } => root
                .join("compiler-deps")
                .canonicalize()
                .unwrap()
                .join("zig")
                .join("zig"),
        }
    }
}

struct ProjectConfig {
    code_folder: PathBuf,
    build_folder: PathBuf,
    project_info: ProjectInfo,
}

fn error<S: Display>(message: S) -> ! {
    println!("{} {}", console::style("error:").red().bright(), message);
    std::process::exit(1);
}

async fn exit(mut emitter: ErrorEmitter<'_>, error_message: ErrorMessage) -> ! {
    emitter.process(vec![error_message]);
    emitter.emit_all().await;
    std::process::exit(1)
}

#[tokio::main]
async fn main() {
    let yaml = clap::load_yaml!("cli.yml");
    let args = clap::App::from_yaml(yaml).get_matches();

    let cli_config = gen_cli_config(&args);

    match args.subcommand() {
        ("build", Some(sub_args)) => {
            process_build(&cli_config, &gen_project_config(&args).await, sub_args).await
        }
        ("run", Some(sub_args)) => {
            process_run(&cli_config, &gen_project_config(&args).await, sub_args).await
        }
        ("update", Some(sub_args)) => update::process_update(&cli_config, sub_args).await,
        ("", _) => {
            clap::App::from_yaml(yaml).print_help().unwrap();
            println!();
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

    let compiler_location = if args.is_present("source") {
        // Find where the root directory of the `quill` source code is.
        let mut compiler_location_path: PathBuf = Path::new(".").into();
        while compiler_location_path.is_dir()
            && !compiler_location_path.join("Cargo.lock").is_file()
        {
            compiler_location_path = compiler_location_path.join("..");
        }
        CompilerLocation::Cargo {
            source: compiler_location_path,
        }
    } else {
        CompilerLocation::Installed {
            host: if cfg!(windows) {
                HostType::Windows
            } else {
                HostType::Linux
            },
            root: dirs::home_dir().unwrap().join(".ql"),
        }
    };

    CliConfig {
        verbose,
        compiler_location,
    }
}

async fn gen_project_config(args: &ArgMatches<'_>) -> ProjectConfig {
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

    let dummy_fs = PackageFileSystem::new(HashMap::new());

    let project_info = if let Ok(project_config_str) = std::fs::read(code_folder.join("quill.toml"))
    {
        match std::str::from_utf8(&project_config_str) {
            Ok(project_config_str) => {
                dummy_fs
                    .overwrite_source_file(
                        SourceFileIdentifier {
                            module: vec![].into(),
                            file: "quill".into(),
                            file_type: SourceFileType::Toml,
                        },
                        project_config_str.to_string(),
                    )
                    .await;
                match toml::from_str::<ProjectInfo>(project_config_str) {
                    Ok(toml) => toml,
                    Err(err) => {
                        let (line, col) = err.line_col().unwrap_or((0, 0));
                        exit(
                            ErrorEmitter::new(&dummy_fs),
                            ErrorMessage::new(
                                format!("'quill.toml' contained an error: {}", err),
                                Severity::Error,
                                Diagnostic::at_location(
                                    &SourceFileIdentifier {
                                        module: vec![].into(),
                                        file: "quill".into(),
                                        file_type: SourceFileType::Toml,
                                    },
                                    Location {
                                        line: line as u32,
                                        col: col as u32,
                                    },
                                ),
                            ),
                        )
                        .await
                    }
                }
            }
            Err(err) => {
                let (valid, after_valid) = project_config_str.split_at(err.valid_up_to());
                let safe_str = unsafe { std::str::from_utf8_unchecked(valid) };
                let safe_str_chars = safe_str.chars().collect::<Vec<_>>();
                let safe_str_slice: String = safe_str_chars
                    [std::cmp::max(20, safe_str_chars.len()) - 20..]
                    .iter()
                    .collect();
                exit(
                    ErrorEmitter::new(&dummy_fs),
                    ErrorMessage::new(
                        format!(
                        "'quill.toml' contained invalid UTF-8 after '...{}', bytes were {:02X?}",
                        safe_str_slice,
                        &after_valid[..std::cmp::min(10, after_valid.len())]
                    ),
                        Severity::Error,
                        Diagnostic::in_file(&SourceFileIdentifier {
                            module: vec![].into(),
                            file: "quill".into(),
                            file_type: SourceFileType::Toml,
                        }),
                    ),
                )
                .await
            }
        }
    } else {
        exit(
            ErrorEmitter::new(&dummy_fs),
            ErrorMessage::new(
                "'quill.toml' file could not be found".to_string(),
                Severity::Error,
                Diagnostic::in_file(&SourceFileIdentifier {
                    module: vec![].into(),
                    file: "quill".into(),
                    file_type: SourceFileType::Toml,
                }),
            ),
        )
        .await
    };

    std::fs::create_dir_all(code_folder.join("build")).unwrap();
    let build_folder = code_folder.join("build").canonicalize().unwrap();

    ProjectConfig {
        code_folder,
        build_folder,
        project_info,
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

async fn process_build(
    cli_config: &CliConfig,
    project_config: &ProjectConfig,
    args: &ArgMatches<'_>,
) {
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
        build(cli_config, project_config, build_info).await;
    }
}

async fn process_run(
    cli_config: &CliConfig,
    project_config: &ProjectConfig,
    _args: &ArgMatches<'_>,
) {
    let info = BuildInfo {
        target_triple: default_target(),
        code_folder: project_config.code_folder.clone(),
        build_folder: project_config.build_folder.clone(),
    };
    run(cli_config, project_config, info).await;
}

async fn build(cli_config: &CliConfig, _project_config: &ProjectConfig, build_info: BuildInfo) {
    cli_config
        .compiler_location
        .invoke_quillc(
            cli_config.verbose,
            &QuillcInvocation {
                build_info,
                zig_compiler: cli_config.compiler_location.zig_compiler(),
            },
        )
        .await;
}

async fn run(cli_config: &CliConfig, project_config: &ProjectConfig, build_info: BuildInfo) {
    build(cli_config, project_config, build_info.clone()).await;
    let mut command = Command::new(build_info.executable(&project_config.project_info.name));
    command.current_dir(build_info.code_folder);
    let status = command.status().await.unwrap();
    if !status.success() {
        if let Some(code) = status.code() {
            error(format!("program terminated with error code {}", code))
        } else {
            error("program terminated with error")
        }
    }
}
