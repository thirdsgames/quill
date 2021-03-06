use std::collections::HashMap;

use quill_common::name::QualifiedName;
use quill_mir::{
    mir::{DefinitionBodyM, DefinitionM, Statement, StatementKind},
    ProjectMIR,
};

/// Converts curried functions and partial application of functions into more LLVM-friendly representations.
pub fn convert_func_objects(project: &mut ProjectMIR) {
    // First, cache the arities of each function.
    // We need to know exactly how many arguments a function "really" has, and how many are coalesced into the result type of the function.
    let mut arities = HashMap::new();
    for (fname, file) in &project.files {
        for (def_name, def) in &file.definitions {
            arities.insert(
                QualifiedName {
                    source_file: fname.clone(),
                    name: def_name.clone(),
                    range: def.range,
                },
                def.arity,
            );
        }
    }

    for file in project.files.values_mut() {
        for def in file.definitions.values_mut() {
            convert_def(def, &arities);
        }
    }
}

fn convert_def(def: &mut DefinitionM, arities: &HashMap<QualifiedName, u64>) {
    if let DefinitionBodyM::PatternMatch(cfg) = &mut def.body {
        for block in cfg.basic_blocks.values_mut() {
            block.statements = block
                .statements
                .drain(..)
                .map(|stmt| convert_stmt(stmt, arities))
                .flatten()
                .collect();
        }
    }
}

/// Converts functional statements (InstanceSymbol, Apply) to imperative statements (ConstructFunctionObject, InvokeFunction, etc.)
/// In this step, all function objects are considered to be unary. A later optimisation step will construct n-ary functions.
fn convert_stmt(stmt: Statement, arities: &HashMap<QualifiedName, u64>) -> Vec<Statement> {
    match stmt.kind {
        StatementKind::InstanceSymbol {
            name,
            type_variables,
            target,
        } => {
            let arity = *arities.get(&name).expect("function did not exist");
            // Store the fact that the target is a function object.
            // new_infos.insert(target, v)

            if arity == 0 {
                // If the function is a nullary function, i.e. a (possibly polymorphic) constant, a function with no arguments,
                // then we can call it right away.
                vec![Statement {
                    range: stmt.range,
                    kind: StatementKind::InvokeFunction {
                        name,
                        type_variables,
                        target,
                        arguments: Vec::new(),
                    },
                }]
            } else {
                // We need to create a new "dummy" function that applies one argument to this function.
                vec![Statement {
                    range: stmt.range,
                    kind: StatementKind::ConstructFunctionObject {
                        name,
                        type_variables,
                        target,
                        curry_steps: std::iter::repeat(1).take(arity as usize).collect(),
                        curried_arguments: Vec::new(),
                    },
                }]
            }
        }
        StatementKind::Apply {
            argument,
            function,
            target,
            return_type,
            argument_type,
        } => {
            // In this step, the function is always going to be a function object.
            // We should now execute it with the InvokeFunctionObject instruction, because at this point of compilation,
            // all function objects are unary.
            // This instruction may return another function object.
            vec![Statement {
                range: stmt.range,
                kind: StatementKind::InvokeFunctionObject {
                    func_object: *function,
                    target,
                    additional_arguments: vec![*argument],
                    return_type,
                    additional_argument_types: vec![argument_type],
                },
            }]
        }
        StatementKind::DropIfAlive { .. } => {
            unreachable!("the borrow checker should have deleted these statements already")
        }
        kind => vec![Statement {
            range: stmt.range,
            kind,
        }],
    }
}
