use super::*;

#[derive(Clone)]

pub(crate) struct IrType {
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
    pub(crate) includes: Vec<String>,
    pub(crate) variants: Vec<IrVariant>,
    pub(crate) members: Vec<IrEnumMember>,
    // Source location of the type declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone)]
pub(crate) struct IrBinding {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) mutable: bool,
    pub(crate) type_: String,
    pub(crate) value: Option<IrValue>,
    // Source location of the binding declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone)]
pub(crate) struct IrField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: String,
    // Source location of the field declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone)]
pub(crate) struct IrVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
    // Source location of the variant declaration.
    pub(crate) loc: IrSourceLoc,
}
#[derive(Clone)]

pub(crate) struct IrEnumMember {
    pub(crate) name: String,
}

#[derive(Clone)]

pub(crate) struct IrParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) default: Option<IrValue>,
    // Source location of the parameter declaration.
    pub(crate) loc: IrSourceLoc,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct IrSourceLoc {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone)]
pub(crate) struct IrRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: IrValue,
}

#[derive(Clone)]
pub struct ExternalFunctionParam {
    pub name: String,
    pub type_: String,
}
