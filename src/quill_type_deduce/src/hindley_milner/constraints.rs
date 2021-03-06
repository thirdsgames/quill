use quill_common::{location::Range, name::QualifiedName};

use crate::hir::expr::TypeVariable;

/// A list of constraints between types.
#[derive(Debug, Default)]
pub(crate) struct Constraints(pub Vec<(TypeVariable, Constraint)>);

impl Constraints {
    pub(crate) fn new_with(ty: TypeVariable, constraint: Constraint) -> Self {
        Self(vec![(ty, constraint)])
    }

    pub(crate) fn union(mut self, mut other: Self) -> Self {
        self.0.append(&mut other.0);
        self
    }
}

/// A constraint about a variable's type, used by the type checker.
#[derive(Debug)]
pub(crate) enum Constraint {
    /// The given type is exactly equal to this type.
    Equality {
        ty: TypeVariable,
        reason: ConstraintEqualityReason,
    },
}

#[derive(Debug)]
pub(crate) enum ConstraintEqualityReason {
    /// This constraint was generated as a result of applying a function to a variable.
    Apply {
        /// The function we're invoking.
        function_range: Range,
        function_ty: TypeVariable,
        /// The argument we're supplying.
        argument_range: Range,
        argument_ty: TypeVariable,
    },
    /// This constraint was generated as a result of generating a lambda abstraction's type.
    /// These constraints should probably be solved first if possible, since they're likely
    /// to have really bad error messages.
    LambdaType { lambda: Range },
    /// This constraint was generated as a result of a lambda's parameter being used
    /// in the lambda expression body.
    LambdaParameter {
        lambda: Range,
        param_name: String,
        param_range: Range,
    },
    /// This constraint was generated as a result of specifying that a let expression's
    /// type must be equal to the right hand expression's type.
    LetType { let_token: Range, expression: Range },
    /// The expression was defined to be a specific type.
    ByDefinition {
        /// The expression we're type checking.
        expr: Range,
        /// The definition that shows what type it must have.
        definition: Range,
    },
    /// The expression was used as a field in a data constructor, and we know the type of the field.
    Field {
        /// The expression we're type checking.
        expr: Range,
        /// The data type we're constructing.
        data_type: QualifiedName,
        /// The type constructor.
        type_ctor: String,
        /// The field name.
        field: String,
    },
    /// This expression is an instance of the variable bound in a let expression.
    InstanceLet {
        /// The name of the variable.
        variable_name: String,
        /// The variable's type.
        variable_type: TypeVariable,
        /// The expression we're type checking.
        expr: Range,
        /// The token `let` that we're using the variable from.
        let_token: Range,
    },
    /// A variable was borrowed.
    Borrow { expr: Range, borrow_token: Range },
    /// A variable was copied.
    Copy { expr: Range, copy_token: Range },
}
