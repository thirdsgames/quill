use codegen::CodeGenContext;
use func::compile_function;
use inkwell::targets::{InitializationConfig, Target};
use inkwell::{context::Context, targets::FileType};
use inkwell::{module::Module, OptimizationLevel};
use inkwell::{
    passes::PassManager,
    targets::{CodeModel, RelocMode, TargetTriple},
};
use quill_index::ProjectIndex;
use quill_mir::ProjectMIR;
use repr::{Monomorphisation, MonomorphisationParameters, MonomorphisedFunction, Representations};
use std::{
    error::Error,
    fmt::{Debug, Display},
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    process::{Command, Output},
};

mod codegen;
mod func;
mod repr;

struct ExecutionError {
    program: String,
    output: Output,
}

impl Error for ExecutionError {}

impl Debug for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} failed with {}", self.program, self.output.status)?;
        writeln!(f, "Output:")?;
        writeln!(f, "{}", std::str::from_utf8(&self.output.stdout).unwrap())?;
        writeln!(f, "Error Output:")?;
        writeln!(f, "{}", std::str::from_utf8(&self.output.stderr).unwrap())?;
        Ok(())
    }
}

impl Display for ExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self, f)
    }
}

/// Builds an LLVM module for the given input source file.
pub fn build(dir: &Path, project_name: &str, mir: &ProjectMIR, index: &ProjectIndex) {
    // println!("Building module...");

    let host_triple = guess_host_triple::guess_host_triple().unwrap();

    let build_folder = dir.join("target").join(project_name).join(host_triple);
    let _ = std::fs::create_dir_all(build_folder.join("bin"));
    let path = Path::new("out.o");

    // Output the MIR.
    {
        use std::io::Write;
        let mir_path = build_folder.join(path.with_extension("mir"));
        let f = File::create(mir_path).unwrap();
        let mut f = BufWriter::new(f);
        writeln!(f, "{}", mir).unwrap();
    }

    let context = Context::create();
    let module = context.create_module(project_name);
    module.set_triple(&TargetTriple::create(host_triple));
    let codegen = CodeGenContext::new(&context, module);

    let mono = Monomorphisation::new(mir);
    let mut reprs = Representations::new(&codegen, index, mono.types);
    // Now that we've computed data type representations we can actually compile the functions.
    // First, declare them all.
    for func in &mono.functions {
        func.add_llvm_type(&codegen, &mut reprs, mir);
    }
    for func in &mono.functions {
        compile_function(&codegen, &reprs, index, mir, func.clone());
    }

    // Now introduce the main function.
    let main_func =
        codegen
            .module
            .add_function("main", codegen.context.i32_type().fn_type(&[], false), None);
    let main_block = codegen.context.append_basic_block(main_func, "entry");
    codegen.builder.position_at_end(main_block);
    codegen.builder.build_call(
        codegen
            .module
            .get_function(
                &MonomorphisedFunction {
                    func: mir.entry_point.clone(),
                    curry_steps: Vec::new(),
                    mono: MonomorphisationParameters {
                        type_parameters: Vec::new(),
                    },
                    direct: true,
                }
                .to_string(),
            )
            .unwrap(),
        &[],
        "call_main",
    );
    codegen
        .builder
        .build_return(Some(&codegen.context.i32_type().const_int(0, false)));

    let pm = PassManager::<Module>::create(&());
    pm.add_verifier_pass();
    println!("Verifying...");
    pm.run_on(&codegen.module);
    println!("Done.");

    // println!("Compiling to target machine...");

    Target::initialize_all(&InitializationConfig::default());

    let object_path = build_folder.join(path);
    let asm_path = build_folder.join(path.with_extension("asm"));
    let bc_path = build_folder.join(path.with_extension("bc"));
    let bc_opt_path = build_folder.join(path.with_extension("opt.bc"));

    let target = Target::from_name("x86-64").unwrap();
    let target_machine = target
        .create_target_machine(
            &TargetTriple::create(host_triple),
            "x86-64",
            "+avx2",
            OptimizationLevel::None,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();

    // Output the LLVM bitcode, and decompile it if we have `llvm-dis` on the system.
    codegen.module.write_bitcode_to_path(&bc_path);
    let _ = Command::new("llvm-dis")
        .arg(bc_path.to_str().unwrap())
        .status();

    let opt = PassManager::<Module>::create(&());
    opt.add_jump_threading_pass();
    opt.add_promote_memory_to_register_pass();
    opt.add_memcpy_optimize_pass();
    println!("Optimising...");
    opt.run_on(&codegen.module);
    println!("Writing bitcode, assembly, and object file...");

    codegen.module.write_bitcode_to_path(&bc_opt_path);
    let _ = Command::new("llvm-dis")
        .arg(bc_opt_path.to_str().unwrap())
        .status();

    assert!(target_machine
        .write_to_file(&codegen.module, FileType::Assembly, &asm_path)
        .is_ok());
    assert!(target_machine
        .write_to_file(&codegen.module, FileType::Object, &object_path)
        .is_ok());

    // Create a "glue" file to force CMake to use the C linker and actually link libc, instead of whatever it normally does.
    File::create(build_folder.join("glue.c")).unwrap();

    println!("Configuring CMake...");

    {
        use std::io::Write;
        // Output the CMakeLists.txt file.
        let mut cmakelists =
            BufWriter::new(std::fs::File::create(build_folder.join("CMakeLists.txt")).unwrap());
        writeln!(cmakelists, "cmake_minimum_required (VERSION 3.10)").unwrap();
        writeln!(cmakelists, "project (TEST)").unwrap();
        writeln!(cmakelists, "add_executable (test out.o glue.c)").unwrap();
    }

    let cmake_configure = Command::new("cmake")
        .arg("..")
        .current_dir(build_folder.join("bin"))
        .output()
        .unwrap();
    if !cmake_configure.status.success() {
        panic!(
            "Errored: {}",
            ExecutionError {
                program: "cmake configure".to_owned(),
                output: cmake_configure,
            }
        );
    }

    println!("Linking...");

    let cmake_build = Command::new("cmake")
        .arg("--build")
        .arg(".")
        .current_dir(build_folder.join("bin"))
        .output()
        .unwrap();
    if !cmake_build.status.success() {
        panic!(
            "Errored: {}",
            ExecutionError {
                program: "cmake build".to_owned(),
                output: cmake_build,
            }
        );
    }

    println!("Running executable...");
    assert!(run_executable(build_folder.join("bin"))
        .status()
        .unwrap()
        .success());
    println!("Done!");
}

fn run_executable(dir: PathBuf) -> Command {
    let file = {
        #[cfg(windows)]
        {
            std::fs::canonicalize(dir.join("Debug").join("test.exe")).unwrap()
        }
        #[cfg(unix)]
        {
            std::fs::canonicalize(dir.join("test")).unwrap()
        }
    };

    let mut command = Command::new(file);
    command.current_dir(dir);
    command
}
