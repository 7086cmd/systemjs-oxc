use fxhash::FxHashMap;
use crate::options::SystemJsTranspilerOptions;
use oxc::allocator::{Allocator, Vec as ArenaVec, Box as ArenaBox, CloneIn};
use oxc::ast::ast::{BindingIdentifier, ExportAllDeclaration, Function, IdentifierName, ImportDeclaration, StringLiteral};
use oxc::ast::{AstBuilder, AstType};
use oxc::semantic::{ScopeFlags, Scoping};

pub struct SystemJsTranspiler<'a> {
    pub options: SystemJsTranspilerOptions,
    pub allocator: &'a Allocator,
    pub scope_stack: Vec<ScopeFlags>,
    pub path_stack: Vec<AstType>,
    pub source_code: &'a str,
    pub builder: AstBuilder<'a>,
    pub top_level_vars: Vec<BindingIdentifier<'a>>,
    pub top_level_function_decls: ArenaVec<'a, Function<'a>>,
    pub top_level_classes: Vec<BindingIdentifier<'a>>,
    pub scoping: Scoping,
    pub imports: ArenaVec<'a, ImportMap<'a>>,
    pub importee: ArenaVec<'a, StringLiteral<'a>>,
}

#[derive(Debug)]
pub enum ImportMap<'a> {
    ImportDeclaration(ArenaBox<'a, ImportDeclaration<'a>>),
    ExportAllDeclaration(ArenaBox<'a, ExportAllDeclaration<'a>>),
}

impl<'a> CloneIn<'a> for ImportMap<'a> {
    type Cloned = ImportMap<'a>;
    fn clone_in(&self, allocator: &'a Allocator) -> Self::Cloned {
        match self {
            ImportMap::ImportDeclaration(it) => ImportMap::ImportDeclaration(it.clone_in(allocator)),
            ImportMap::ExportAllDeclaration(it) => ImportMap::ExportAllDeclaration(it.clone_in(allocator)),
        }
    }
}

impl<'a> SystemJsTranspiler<'a> {
    pub fn new(
        options: SystemJsTranspilerOptions,
        allocator: &'a Allocator,
        scoping: Scoping,
        source_code: &'a str,
    ) -> Self {
        let builder = AstBuilder::new(allocator);
        Self {
            options,
            allocator,
            scope_stack: vec![],
            path_stack: vec![],
            source_code,
            top_level_function_decls: builder.vec(),
            builder,
            top_level_vars: vec![],
            top_level_classes: vec![],
            scoping,
            imports: builder.vec(),
            importee: builder.vec(),
        }
    }
}
