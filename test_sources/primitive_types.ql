pub enum Either[T, U] = Left { value: T } | Right { value: U }

pub def create_either[T]: T -> Either[T, unit] {
    create_either t = Left { value = t }
    //create_either a = Either::Right { value = a }
}

pub enum Option[T] = Some { value: T } | None {}

pub def unwrap_or[T]: T -> Option[T] -> T {
    unwrap_or _ Some { value } = value
    unwrap_or t None {} = t
}

pub def block: Option[Option[unit]] {
    block = (
        let inner = unit
        let next = Some { value = inner }
        Some { value = next }
    )
}

pub enum Bool = True {} | False {}

// Removed lambdas for now. We'll add them back in once they're correctly translatable into MIR.
pub def or: Bool -> Bool -> Bool {
    // or True {} = \a -> a
    // or _ = \a -> False {}
    or True {} a = a
    or _ _ = False {}
}

pub def one: int {
    one = 1
}

pub def or_options: Option[Bool] -> Option[Bool] -> Option[Bool] {
    or_options Some { value = left } Some { value = right } = Some { value = or left right }
    or_options _ _ = None {},
}

pub def test_let: unit {
    test_let = let a = 1,
}

def main: unit {
    main = (
        unwrap_or Some { value = unit } block
        create_either 104
        test_let
        unwrap_or False {} (or_options Some { value = True {} } Some { value = False {} })
        unit
    )
}