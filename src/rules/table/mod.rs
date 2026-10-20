//! The diagnostic rule table: one `&[Rule]` per category, walked in a fixed order.
use super::{Rule, Severity};

mod front_end;
mod tooling;
mod type_check;

/// The parts in table order. Lookup is by name, so the order is the listing order only.
const PARTS: &[&[Rule]] = &[front_end::RULES, type_check::RULES, tooling::RULES];

/// Every rule, in table order.
pub(super) fn rules() -> impl Iterator<Item = &'static Rule> {
    PARTS.iter().flat_map(|part| part.iter())
}
