//! `collections::values` — descriptor entry + target-generic lowering (plan-96).

use super::{custom, req};
use crate::codegen::registry::BuiltinFunction;
use crate::target::shared::code::type_utils::map_type_parts;
use crate::target::shared::code::{CodeBuilder, ValueResult};
use crate::target::shared::nir::NirValue;

const INTO_VALUES: &str = "Return a map's values as a list.";
const DESC_VALUES: &str = r#"`collections::values` builds a new `List OF V` holding the value of every entry
in `value`. It walks the map's lookup-entry table front to back, copying each
entry's value payload into a freshly allocated list block. The source map is not
mutated and its own storage is not aliased by the result — the returned list is
an independent, owned collection.

The result has exactly one item per map entry, so its length equals
`len(value)`. An empty map yields an empty list. Unlike the key projection, the
result may contain duplicates, because distinct keys may store equal values.

**Ordering.** The projection walks the lookup-entry array directly, and that
array is maintained in insertion order; the hash bucket index is separate
derived metadata that does not reorder it. `collections::values` and
`collections::keys` are the same traversal over the same entries and differ only
in which payload field of each entry they copy, so the two results are
index-aligned: item `i` of `collections::values(m)` is the value of the entry
whose key is item `i` of `collections::keys(m)`. The language specification
describes map iteration order as implementation-defined but stable for a given
unchanged map, so treat insertion order as the current implementation's behavior
rather than a guarantee to rely on across versions.

`collections::values` raises no trappable domain error, so an inline `TRAP` on a
`values` call has a dead handler. Building the result list does allocate, and an
allocation failure is not a trappable domain error in this language."#;

pub(crate) const VALUES: BuiltinFunction = BuiltinFunction::native(
    "collections.values",
    "values",
    INTO_VALUES,
    DESC_VALUES,
    &[],
    &[custom(&[req("value", &["map"], "Map OF K TO V")])],
    lower_values,
);

/// `collections::values(Map OF K TO V) AS List OF V`: project each entry's value.
pub(crate) fn lower_values(
    builder: &mut CodeBuilder,
    args: &[NirValue],
) -> Result<ValueResult, String> {
    let collection = builder.lower_value(&args[0])?;
    let Some((_, value_type)) = map_type_parts(&collection.type_) else {
        return Err(format!(
            "native collection values does not accept {}",
            collection.type_
        ));
    };
    builder.lower_map_projection(&collection, &value_type, false)
}
