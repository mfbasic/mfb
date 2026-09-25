//! Mouse input: the SGR decoder, the per-thread event ring, and the monotonic
//! stamp they share (plan-94-B).
//!
//! Split out of `term`/`canvas` rather than owned by either, because both
//! packages poll the same ring and every app backend feeds the same decoder. The
//! surface each package exposes differs only in the coordinate unit it reads the
//! slots as — cells for `term::`, pixels for `canvas::` — and the unit is a
//! property of the mouse MODE, not of the slot (plan-94-A §4.2).

pub(crate) mod clock;
pub(crate) mod decode;
pub(crate) mod ring;
pub(crate) mod sgr;
