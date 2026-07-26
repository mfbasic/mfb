use super::*;

/// The documentation surface of a project: an optional package-level entry plus
/// one entry per documented exported declaration (plan-09-doc.md §5).
#[derive(Clone, Default)]
pub(crate) struct ProjectDocs {
    pub(crate) package: Option<IrPackageDoc>,
    pub(crate) decls: Vec<IrDocDecl>,
}

#[derive(Clone)]
pub(crate) struct IrPackageDoc {
    pub(crate) name: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind::code`.
    pub(crate) desc: Vec<(u8, String)>,
    /// `Some(message)` when deprecated (message may be empty); `None` otherwise.
    pub(crate) deprecated: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrDocKind {
    Func,
    Sub,
    Type,
    Union,
    Enum,
    Resource,
}

#[derive(Clone)]
pub(crate) struct IrDocDecl {
    pub(crate) kind: IrDocKind,
    pub(crate) name: String,
    pub(crate) signature: String,
    /// `GROUP` name for FUNC/SUB, or empty.
    pub(crate) group: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind::code`.
    pub(crate) desc: Vec<(u8, String)>,
    pub(crate) args: Vec<(String, String)>,
    pub(crate) props: Vec<(String, String)>,
    pub(crate) ret: String,
    pub(crate) errors: Vec<(String, String)>,
    pub(crate) example: String,
    pub(crate) internal: bool,
    /// `Some(message)` when deprecated (message may be empty); `None` otherwise.
    pub(crate) deprecated: Option<String>,
}

fn doc_prose(desc: &[crate::ast::DocProse]) -> Vec<(u8, String)> {
    desc.iter()
        .map(|prose| (prose.kind.code(), prose.text.clone()))
        .collect()
}

/// Collect the documentation surface from a project's `DOC` blocks. Only exported
/// declarations are recorded (plan-09-doc.md §3): a non-exported declaration is
/// documented in source but never persisted into the compiled package. Runs after
/// `DOC` validation, so every block here is well-formed.
pub(crate) fn collect_project_docs(ast: &crate::ast::AstProject) -> ProjectDocs {
    use crate::ast::{DocHeaderKind, Function, FunctionKind, Item, TypeDeclKind, Visibility};

    let mut funcs: HashMap<&str, Vec<&Function>> = HashMap::new();
    let mut types: HashMap<&str, (TypeDeclKind, Visibility, String)> = HashMap::new();
    let mut resources: HashMap<&str, (Visibility, String)> = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    funcs
                        .entry(function.name.as_str())
                        .or_default()
                        .push(function);
                }
                Item::Type(type_decl) => {
                    types.entry(type_decl.name.as_str()).or_insert((
                        type_decl.kind,
                        type_decl.visibility,
                        type_decl.signature_line(),
                    ));
                }
                Item::Resource(resource) => {
                    resources
                        .entry(resource.name.as_str())
                        .or_insert((resource.visibility, resource.signature_line()));
                }
                _ => {}
            }
        }
    }

    // Pick the overload a callable DOC block documents: the one matching the
    // header's parameter types, or the first matching-kind overload otherwise.
    let overload_for = |doc: &crate::ast::DocBlock, want_sub: bool| -> Option<&Function> {
        let list = funcs.get(doc.header_name.as_str())?;
        let matching = list
            .iter()
            .copied()
            .filter(|f| (f.kind == FunctionKind::Sub) == want_sub);
        match &doc.header_params {
            Some(wanted) => matching
                .clone()
                .find(|f| crate::ast::param_types(f) == crate::ast::normalize_types(wanted)),
            None => matching.clone().next(),
        }
    };

    let make_decl = |doc: &crate::ast::DocBlock, kind: IrDocKind, signature: String| IrDocDecl {
        kind,
        name: doc.header_name.clone(),
        signature,
        group: doc
            .groups
            .first()
            .map(|(name, _)| name.clone())
            .unwrap_or_default(),
        desc: doc_prose(&doc.desc),
        args: doc
            .args
            .iter()
            .map(|arg| (arg.name.clone(), arg.desc.clone()))
            .collect(),
        props: doc
            .props
            .iter()
            .map(|prop| (prop.name.clone(), prop.desc.clone()))
            .collect(),
        ret: doc
            .rets
            .first()
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        errors: doc
            .errors
            .iter()
            .map(|error| (error.code.clone(), error.desc.clone()))
            .collect(),
        example: doc
            .examples
            .first()
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        internal: doc
            .attrs
            .iter()
            .any(|attr| attr.eq_ignore_ascii_case("INTERNAL")),
        deprecated: doc.deprecated.first().map(|(message, _)| message.clone()),
    };

    let mut package = None;
    let mut decls = Vec::new();
    for file in &ast.files {
        for item in &file.items {
            let Item::Doc(doc) = item else {
                continue;
            };
            match doc.header_kind {
                DocHeaderKind::Package => {
                    if package.is_none() {
                        package = Some(IrPackageDoc {
                            name: ast.name.clone(),
                            desc: doc_prose(&doc.desc),
                            deprecated: doc.deprecated.first().map(|(message, _)| message.clone()),
                        });
                    }
                }
                DocHeaderKind::Func | DocHeaderKind::Sub => {
                    let want_sub = doc.header_kind == DocHeaderKind::Sub;
                    // Only exported overloads are persisted (plan-09-doc.md §3).
                    let Some(function) = overload_for(doc, want_sub) else {
                        continue;
                    };
                    if function.visibility != Visibility::Export {
                        continue;
                    }
                    let kind = if want_sub {
                        IrDocKind::Sub
                    } else {
                        IrDocKind::Func
                    };
                    decls.push(make_decl(doc, kind, function.signature_line()));
                }
                DocHeaderKind::Type | DocHeaderKind::Union | DocHeaderKind::Enum => {
                    let Some((_, vis, signature)) = types.get(doc.header_name.as_str()) else {
                        continue;
                    };
                    if *vis != Visibility::Export {
                        continue;
                    }
                    let kind = match doc.header_kind {
                        DocHeaderKind::Type => IrDocKind::Type,
                        DocHeaderKind::Union => IrDocKind::Union,
                        _ => IrDocKind::Enum,
                    };
                    decls.push(make_decl(doc, kind, signature.clone()));
                }
                DocHeaderKind::Resource => {
                    let Some((vis, signature)) = resources.get(doc.header_name.as_str()) else {
                        continue;
                    };
                    if *vis != Visibility::Export {
                        continue;
                    }
                    decls.push(make_decl(doc, IrDocKind::Resource, signature.clone()));
                }
            }
        }
    }

    ProjectDocs { package, decls }
}
