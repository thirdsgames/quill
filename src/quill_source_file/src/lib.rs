use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Debug,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

use quill_common::{
    diagnostic::{Diagnostic, ErrorMessage, HelpType, Severity},
    location::{
        ModuleIdentifier, SourceFileIdentifier, SourceFileIdentifierSegment, SourceFileType,
    },
};
use std::{fs::File, io::BufReader, sync::RwLock};

/// If a source file's contents could not be loaded, why was this?
#[derive(Debug)]
pub enum SourceFileLoadError {
    Io(std::io::Error),
}

/// A single file of source code.
pub struct SourceFile {
    contents: String,
    modified_time: SystemTime,
}

impl Debug for SourceFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "source file modified at {:?}", self.modified_time)
    }
}

impl SourceFile {
    pub fn get_contents(&self) -> &str {
        self.contents.as_str()
    }

    pub fn last_modified(&self) -> SystemTime {
        self.modified_time
    }
}

/// A tree of source files and other modules.
#[derive(Debug, Default)]
pub struct Module {
    pub submodules: HashMap<String, Module>,
    pub source_files: HashMap<String, Result<SourceFile, SourceFileLoadError>>,
}

/// Represents the file structure of an entire package on disk.
/// A package is a folder that produces some kind of compiled output file,
/// which could be a binary, a static library, a shared object, a WASM module,
/// or something else of that kind.
pub struct PackageFileSystem {
    /// Maps project names (e.g. `core`) to the path their sources (and the `quill.toml` file) are stored at.
    pub project_directories: HashMap<String, PathBuf>,
    root_module: RwLock<Module>,
}

impl PackageFileSystem {
    /// TODO: Make only one project_directories map in `quill` and send it to `quillc`.
    pub fn new(project_directories: HashMap<String, PathBuf>) -> Self {
        Self {
            project_directories,
            root_module: RwLock::new(Module::default()),
        }
    }

    fn with_module<F, T>(&self, identifier: &ModuleIdentifier, func: F) -> T
    where
        F: FnOnce(&mut Module) -> T,
    {
        let mut guard = self.root_module.write().unwrap();
        let mut module = &mut *guard;
        for SourceFileIdentifierSegment(segment) in &identifier.segments {
            module = module.submodules.entry(segment.clone()).or_default();
        }
        func(&mut module)
    }

    /// Gets a source file stored in memory, or reads it from disk if it isn't loaded yet.
    pub fn with_source_file<F, T>(&self, identifier: &SourceFileIdentifier, func: F) -> T
    where
        F: FnOnce(Result<&SourceFile, &SourceFileLoadError>) -> T,
    {
        // To get around borrowing rules, we put the func in an option, and take it out when we use it.
        let mut func = Some(func);

        let module_identifier = &identifier.module;
        let file_identifier = &identifier.file.0;

        let result = self.with_module(module_identifier, |module| {
            module
                .source_files
                .get(file_identifier)
                .map(|result| func.take().unwrap()(result.as_ref()))
        });

        match result {
            Some(result) => result,
            None => {
                let file = self.load_source_file(identifier.clone());
                // Recreate the borrows here, since they were implicitly destroyed so that we could clone the identifier.
                let module_identifier = &identifier.module;
                let file_identifier = &identifier.file.0;

                self.with_module(module_identifier, |module| {
                    let result = func.take().unwrap()(file.as_ref());
                    module.source_files.insert(file_identifier.clone(), file);
                    result
                })
            }
        }
    }

    /// Removes the cached entry of this source file from memory.
    /// Next time we need this file, it will be reloaded from disk.
    pub fn remove_cache(&self, identifier: &SourceFileIdentifier) {
        let module_identifier = &identifier.module;
        let file_identifier = &identifier.file.0;
        self.with_module(module_identifier, |module| {
            module.source_files.remove(file_identifier)
        });
    }

    /// Overwrites the truth of this source file with new contents.
    pub fn overwrite_source_file(&self, identifier: SourceFileIdentifier, contents: String) {
        // eprintln!("overwriting {}", identifier);
        let module_identifier = &identifier.module;
        let file_identifier = identifier.file.0;
        self.with_module(module_identifier, |module| {
            match module.source_files.entry(file_identifier) {
                Entry::Occupied(mut occupied) => {
                    // eprintln!("source was {:?}", occupied.get());
                    *occupied.get_mut() = Ok(SourceFile {
                        contents,
                        modified_time: SystemTime::now(),
                    });
                    // eprintln!("source now {:?}", occupied.get());
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(Ok(SourceFile {
                        contents,
                        modified_time: SystemTime::now(),
                    }));
                }
            }
        });
    }

    pub fn file_path(&self, identifier: &SourceFileIdentifier) -> PathBuf {
        let directory = self.project_directories[&identifier.module.segments[0].0].clone();
        let directory = identifier
            .module
            .segments
            .iter()
            .skip(1)
            .fold(directory, |dir, segment| dir.join(&segment.0));
        directory
            .join(&identifier.file.0)
            .with_extension(identifier.file_type.file_extension())
    }

    fn load_source_file(
        &self,
        identifier: SourceFileIdentifier,
    ) -> Result<SourceFile, SourceFileLoadError> {
        let file = File::open(self.file_path(&identifier)).map_err(SourceFileLoadError::Io)?;

        let metadata = file.metadata().map_err(SourceFileLoadError::Io)?;
        let modified_time = metadata.modified().map_err(SourceFileLoadError::Io)?;
        let mut contents = Default::default();
        BufReader::new(file)
            .read_to_string(&mut contents)
            .map_err(SourceFileLoadError::Io)?;
        Ok(SourceFile {
            contents,
            modified_time,
        })
    }
}

/// Prints error and warning messages, outputting the relevant lines of source code from the input files.
#[must_use = "error messages must be emitted using the emit_all method"]
pub struct ErrorEmitter<'fs> {
    package_file_system: &'fs PackageFileSystem,
}

impl<'fs> ErrorEmitter<'fs> {
    pub fn new(package_file_system: &'fs PackageFileSystem) -> Self {
        Self {
            package_file_system,
        }
    }

    /// Emits the given message to the screen.
    pub fn emit(&self, message: ErrorMessage) {
        use console::style;

        match message.severity {
            Severity::Error => {
                println!(
                    "{}{} {}",
                    style("error").red().bright(),
                    style(":").white().bright(),
                    style(message.message).white().bright()
                );
                self.print_message(message.diagnostic, |s| style(s).red().bright());
            }
            Severity::Warning => {
                println!(
                    "{}: {}",
                    style("warning").yellow().bright(),
                    message.message
                );
                self.print_message(message.diagnostic, |s| style(s).yellow().bright());
            }
        }

        for help in message.help {
            match help.help_type {
                HelpType::Help => println!(
                    "{} {}",
                    style("help:").white().bright(),
                    style(help.message).white().bright()
                ),
                HelpType::Note => println!(
                    "{} {}",
                    style("note:").white().bright(),
                    style(help.message).white().bright()
                ),
            }
            self.print_message(help.diagnostic, |s| style(s).cyan().bright());
        }
    }

    fn print_message(
        &self,
        diagnostic: Diagnostic,
        style_arrows: impl Fn(String) -> console::StyledObject<String>,
    ) {
        use console::style;

        if let Some(range) = diagnostic.range {
            // We calculate the amount of digits in the line number.
            let line_number_max_digits =
                (range.start.line.max(range.end.line) + 1).to_string().len();

            println!(
                "{}{} {} ({}) @ {}:{}",
                " ".repeat(line_number_max_digits),
                style("-->").cyan().bright(),
                diagnostic.source_file,
                diagnostic.source_file.file_type,
                range.start.line + 1,
                range.start.col + 1
            );

            // Let's get the contents of the offending source code file.
            self.package_file_system
                .with_source_file(&diagnostic.source_file, |source_file| {
                    // We don't need to worry about optimising reads from the source file, since it's cached in memory anyway,
                    // and this is the cold path because we're just handling errors.
                    match source_file {
                        Ok(source_file) => {
                            let lines = source_file
                                .get_contents()
                                .lines()
                                .enumerate()
                                .skip(range.start.line as usize)
                                .take((range.end.line - range.start.line + 1) as usize);

                            // Print out each relevant line of code, starting and finishing with an empty line.

                            // Empty line.
                            println!(
                                "{: >2$} {}",
                                "",
                                style("|").cyan().bright(),
                                line_number_max_digits,
                            );

                            // Relevant lines.
                            for (line_number, line_contents) in lines {
                                let line_length = line_contents.chars().count();

                                // Signal where on the line the error occured if we're on the first line.
                                if line_number == range.start.line as usize {
                                    // If the error was on a single line, we'll just underline where the error occured.
                                    // We don't need an overline.
                                    if range.start.line != range.end.line {
                                        println!(
                                            "{: >4$} {} {: >5$}{}",
                                            "",
                                            style("|").cyan().bright(),
                                            "",
                                            style_arrows(
                                                "v".repeat(line_length - range.start.col as usize)
                                            ),
                                            line_number_max_digits,
                                            range.start.col as usize,
                                        );
                                    }
                                }

                                println!(
                                    "{: >3$} {} {}",
                                    style((line_number + 1).to_string()).cyan().bright(),
                                    style("|").cyan().bright(),
                                    line_contents,
                                    line_number_max_digits,
                                );

                                // Signal where on the line the error occured if we're on the last line.
                                if line_number == range.end.line as usize {
                                    if range.start.line == range.end.line {
                                        // The error was on a single line. We'll just underline where the error occured.
                                        println!(
                                            "{: >4$} {} {: >5$}{}",
                                            "",
                                            style("|").cyan().bright(),
                                            "",
                                            style_arrows("^".repeat(
                                                range.end.col as usize - range.start.col as usize
                                            )),
                                            line_number_max_digits,
                                            range.start.col as usize,
                                        );
                                    } else {
                                        // Underline from the start of the line to the end of the error.
                                        println!(
                                            "{: >3$} {} {}",
                                            "",
                                            style("|").cyan().bright(),
                                            style_arrows("^".repeat(range.end.col as usize)),
                                            line_number_max_digits,
                                        );
                                    }
                                }
                            }

                            // Empty line.
                            println!(
                                "{: >2$} {}",
                                "",
                                style("|").cyan().bright(),
                                line_number_max_digits,
                            );
                        }
                        Err(_) => {
                            println!(
                                "{}",
                                style("could not read file".to_string()).red().bright()
                            );
                        }
                    }
                });
        } else {
            println!(
                "{} {}",
                style("-->").cyan().bright(),
                diagnostic.source_file
            );
        }
    }
}

pub fn find_all_source_files(
    root_module: ModuleIdentifier,
    code_folder: &Path,
) -> Vec<SourceFileIdentifier> {
    let mut result = Vec::new();
    let read_dir = std::fs::read_dir(code_folder).unwrap();
    for entry in read_dir {
        let entry = entry.unwrap();
        let metadata = entry.metadata().unwrap();
        if metadata.is_file() {
            let os_fname = entry.file_name();
            let fname = os_fname.to_string_lossy();
            if let Some(fname) = fname.strip_suffix(".ql") {
                result.push(SourceFileIdentifier {
                    module: root_module.clone(),
                    file: fname.to_string().into(),
                    file_type: SourceFileType::Quill,
                })
            }
        } else if metadata.is_dir() {
            let os_folder_name = entry.file_name();
            let folder_name = os_folder_name.to_string_lossy();
            // TODO: check if this is a valid folder name.
            result.extend(find_all_source_files(
                ModuleIdentifier {
                    segments: root_module
                        .segments
                        .iter()
                        .cloned()
                        .chain(std::iter::once(folder_name.into()))
                        .collect(),
                },
                &entry.path(),
            ));
        }
    }
    result
}
