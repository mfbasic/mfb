//! `canvas::groupCount` / `canvas::groupBytes` — the group table's two stats readings.
//!
//! Internal-only. `__canvas_presentSurface` writes them into the `MFB_CANVAS_STATS`
//! line, which `.ai/canvas-threading.md` §11 records as the only window a test has onto
//! worker-owned state — and the group table is worker-owned state living in a
//! process-global block no MFBASIC expression can reach.

use super::gen_group::{emit_group_bytes, emit_group_count, emit_group_items, emit_group_reclaim, emit_group_resolve, emit_group_revision, emit_group_slots, emit_next_reclaimable_group, emit_retired_items};
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "groupCount",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_count),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "groupBytes",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_bytes),
        }],
    });
    // The resolver trio. `present` uses these to turn every `canvas::Group` node's name
    // into a slot index once, on the worker, so the graphics thread never does a string
    // lookup (section 4.4).
    pkg.add_function(RegistryFunction {
        name: "groupResolve",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "name",
                desc: "",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_resolve),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "groupRevision",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "slot",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_revision),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "groupItems",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "slot",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::named("DrawItem")),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(emit_group_items),
        }],
    });
    // plan-116-J: the table's slot bound, so the close helper can scan every slot's LIVE
    // items without a `256` spelled in MFBASIC source that nothing keeps in step.
    pkg.add_function(RegistryFunction {
        name: "groupSlots",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_group_slots),
        }],
    });
    // plan-116-J: the retired buffer, for the free path's close. `groupItems`' twin --
    // same copy-out, the retired word instead of the live one.
    pkg.add_function(RegistryFunction {
        name: "retiredItems",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "slot",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::named("DrawItem")),
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_function(emit_retired_items),
        }],
    });
    // The drain gate. Called from `__canvas_present` on EVERY present and before the
    // content comparison, not on the publish path -- see the function's own comment for
    // why the scene ring's placement would not do (G7).
    //
    // plan-116-J split it in two: this one only FINDS a due slot, and the free below
    // takes the slot it names. The split exists so the resources a retired buffer owns
    // can be closed by an MFBASIC `MATCH` between the two -- open-coding the `DrawItem`
    // union's layout in codegen is the alternative, and a `MATCH` a new variant must
    // handle is a compile error where a hand-written tag offset is a hope (J13).
    pkg.add_function(RegistryFunction {
        name: "nextReclaimableGroup",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(emit_next_reclaimable_group),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "groupReclaim",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "slot",
                desc: "",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(emit_group_reclaim),
        }],
    });
}
