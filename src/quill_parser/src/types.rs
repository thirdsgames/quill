use quill_common::location::{Range, Ranged};

use crate::{definition::TypeBorrowP, identifier::IdentifierP};

#[derive(Debug)]
pub enum TypeP {
    /// An explicitly named type possibly with type parameters, e.g. `Bool`, `Either[T, U]` or `Foo[T[_]]`.
    Named {
        identifier: IdentifierP,
        params: Vec<TypeP>,
    },
    /// A function `a -> b`.
    /// Functions with more arguments, e.g. `a -> b -> c` are represented as
    /// curried functions, e.g. `a -> (b -> c)`.
    Function(Box<TypeP>, Box<TypeP>),
    /// A borrowed type.
    Borrow { ty: Box<TypeP>, borrow: TypeBorrowP },
    /// An implementation of an aspect for a list of types.
    Impl {
        impl_token: Range,
        aspect: IdentifierP,
        params: Vec<TypeP>,
    },
}

impl Ranged for TypeP {
    fn range(&self) -> Range {
        match self {
            TypeP::Named {
                identifier,
                params: args,
            } => args
                .iter()
                .fold(identifier.range(), |acc, i| acc.union(i.range())),
            TypeP::Function(left, right) => left.range().union(right.range()),
            TypeP::Borrow { ty, borrow } => ty.range().union(borrow.borrow_token),
            TypeP::Impl {
                impl_token, aspect, ..
            } => aspect.range().union(*impl_token),
        }
    }
}
