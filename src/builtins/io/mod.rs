pub(crate) mod print;

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        print::NAME => Some(print::RETURN_TYPE),
        _ => None,
    }
}
