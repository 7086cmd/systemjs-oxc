use crate::transpiler::{ImportMap, SystemJsTranspiler};
use oxc::allocator::Vec as ArenaVec;
use oxc::allocator::{CloneIn, FromIn, TakeIn};
use oxc::ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget,
    AssignmentTargetMaybeDefault, BindingIdentifier, ClassType, Declaration, ExportAllDeclaration,
    ExportDefaultDeclarationKind, Expression, FormalParameterKind, FunctionType,
    IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier, Program, PropertyKind,
    Statement, VariableDeclarationKind,
};
use oxc::ast::{ast, match_declaration, AstType, NONE};
use oxc::ast_visit::{walk_mut, Visit, VisitMut};
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::{ScopeFlags, ScopeId};
use oxc::span::{Atom, SourceType, SPAN};
use oxc::syntax::identifier;
use oxc::syntax::identifier::is_identifier_name;
use std::borrow::Cow;
use std::cell::Cell;

impl<'a> Visit<'a> for SystemJsTranspiler<'a> {}

impl<'a> VisitMut<'a> for SystemJsTranspiler<'a> {
    fn enter_scope(&mut self, flags: ScopeFlags, _scope_id: &Cell<Option<ScopeId>>) {
        self.scope_stack.push(flags);
    }

    fn leave_scope(&mut self) {
        self.scope_stack.pop();
    }

    fn enter_node(&mut self, kind: AstType) {
        self.path_stack.push(kind);
    }

    fn leave_node(&mut self, _kind: AstType) {
        self.path_stack.pop();
    }

    fn visit_program(&mut self, it: &mut Program<'a>) {
        walk_mut::walk_program(self, it);
        // imported_symbols + top_level_classes + top_level_vars
        let mut decl_targets = vec![];
        decl_targets.extend(std::mem::replace(&mut self.imported_symbols, vec![]));
        decl_targets.extend(std::mem::replace(&mut self.top_level_classes, vec![]));
        decl_targets.extend(std::mem::replace(&mut self.top_level_vars, vec![]));
        let targets = self.builder.vec_from_iter(decl_targets.iter().map(|var| {
            self.builder.variable_declarator(
                SPAN,
                VariableDeclarationKind::Var,
                self.builder.binding_pattern(
                    self.builder
                        .binding_pattern_kind_binding_identifier(SPAN, var.name),
                    NONE,
                    false,
                ),
                None,
                false,
            )
        }));
        let declaration =
            self.builder
                .variable_declaration(SPAN, VariableDeclarationKind::Var, targets, false);
        let mut body = self
            .builder
            .vec_from_iter(self.top_level_function_decls.iter_mut().map(|x| {
                Statement::FunctionDeclaration(self.builder.alloc(x.take_in(self.allocator)))
            }));
        body.insert(
            0,
            Statement::VariableDeclaration(self.builder.alloc(declaration)),
        );
        if !self.export_tree.is_empty() {
            body.push(
                self.builder.statement_expression(
                    SPAN,
                    self.builder.expression_call(
                        SPAN,
                        self.builder.expression_identifier(SPAN, "_exports"),
                        NONE,
                        self.builder.vec1(Argument::from(
                            self.builder
                                .expression_object(SPAN, self.export_tree.take_in(self.allocator)),
                        )),
                        false,
                    ),
                ),
            );
        }
        let mut return_object_items = self.builder.vec();
        return_object_items.push(
            self.builder.object_property_kind_object_property(
                SPAN,
                PropertyKind::Init,
                self.builder.property_key_static_identifier(SPAN, "setters"),
                self.builder.expression_array(
                    SPAN,
                    self.builder.vec_from_iter(
                        self.convert_import_map().iter_mut().map(|import| {
                            ArrayExpressionElement::from(import.take_in(self.builder))
                        }),
                    ),
                ),
                false,
                false,
                false,
            ),
        );
        let mut new_body = it.body.take_in(self.allocator);
        new_body.retain(|s| {
            !matches!(
                s,
                Statement::EmptyStatement(_) | Statement::DebuggerStatement(_)
            )
        });
        return_object_items.push(
            self.builder.object_property_kind_object_property(
                SPAN,
                PropertyKind::Init,
                self.builder.property_key_static_identifier(SPAN, "execute"),
                self.builder.expression_function(
                    SPAN,
                    FunctionType::FunctionExpression,
                    None,
                    false,
                    false,
                    false,
                    NONE,
                    NONE,
                    self.builder.formal_parameters(
                        SPAN,
                        FormalParameterKind::FormalParameter,
                        self.builder.vec(),
                        NONE,
                    ),
                    NONE,
                    Some(
                        self.builder
                            .function_body(SPAN, self.builder.vec(), new_body),
                    ),
                ),
                false,
                false,
                false,
            ),
        );
        let return_factory = self.builder.statement_return(
            SPAN,
            Some(self.builder.expression_object(SPAN, return_object_items)),
        );
        body.push(return_factory);
        let factory_function =
            self.builder.expression_function(
                SPAN,
                FunctionType::FunctionExpression,
                None,
                false,
                false,
                false,
                NONE,
                NONE,
                self.builder.formal_parameters(
                    SPAN,
                    FormalParameterKind::FormalParameter,
                    self.builder.vec(),
                    NONE,
                ),
                NONE,
                Some(self.builder.function_body(
                    SPAN,
                    self.builder.vec1(self.builder.directive(
                        SPAN,
                        self.builder.string_literal(
                            SPAN,
                            "use strict",
                            Some(Atom::from("use strict")),
                        ),
                        "use strict",
                    )),
                    body,
                )),
            );
        let mut params = self.builder.vec();
        params.push(Argument::from(
            self.builder.expression_array(
                SPAN,
                self.builder.vec_from_iter(
                    self.importee
                        .take_in(self.allocator)
                        .into_iter()
                        .map(|lit| ArrayExpressionElement::StringLiteral(self.builder.alloc(lit))),
                ),
            ),
        ));
        params.push(Argument::from(factory_function));
        let factory = self.builder.statement_expression(
            SPAN,
            self.builder.expression_call(
                SPAN,
                Expression::StaticMemberExpression(self.builder.alloc(
                    self.builder.static_member_expression(
                        SPAN,
                        self.builder.expression_identifier(SPAN, "System"),
                        self.builder.identifier_name(SPAN, "register"),
                        false,
                    ),
                )),
                NONE,
                params,
                false,
            ),
        );
        it.body = self.builder.vec1(factory)
    }

    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        match expr {
            Expression::ThisExpression(_)
                if !self.options.allow_top_level_this && self.is_valid_tla_scope() =>
            {
                *expr = self.builder.void_0(SPAN)
            }
            _ => {}
        }
        walk_mut::walk_expression(self, expr);
    }

    fn visit_statement(&mut self, it: &mut Statement<'a>) {
        walk_mut::walk_statement(self, it);
        match it {
            decl @ match_declaration!(Statement) => {
                let declaration = decl.clone_in(self.allocator).into_declaration();
                let converted = self.convert_declaration(declaration);
                if let Some(new_code) = converted {
                    *decl = self.builder.statement_expression(SPAN, new_code)
                }
            }
            Statement::ImportDeclaration(_) => *it = self.builder.statement_empty(SPAN),
            Statement::ExportAllDeclaration(_) => *it = self.builder.statement_empty(SPAN),
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = export.declaration.take() {
                    match &decl {
                        Declaration::FunctionDeclaration(func) => {
                            let fn_name = func
                                .id
                                .clone_in(self.allocator)
                                .expect("Should have a name")
                                .name;
                            self.export_tree.push(
                                self.builder.object_property_kind_object_property(
                                    SPAN,
                                    PropertyKind::Init,
                                    self.builder.property_key_static_identifier(
                                        SPAN,
                                        fn_name.clone_in(self.allocator),
                                    ),
                                    self.builder.expression_identifier(SPAN, fn_name),
                                    false,
                                    false,
                                    false,
                                ),
                            );
                        }
                        Declaration::ClassDeclaration(cls) => {
                            let class_name = cls
                                .id
                                .clone_in(self.allocator)
                                .expect("Should have a name")
                                .name;
                            self.export_tree.push(
                                self.builder.object_property_kind_object_property(
                                    SPAN,
                                    PropertyKind::Init,
                                    self.builder.property_key_static_identifier(
                                        SPAN,
                                        class_name.clone_in(self.allocator),
                                    ),
                                    self.builder.void_0(SPAN),
                                    false,
                                    false,
                                    false,
                                ),
                            );
                        }
                        _ => {}
                    }
                    let names = self.extract_declared_names(&decl);
                    let converted = self.convert_declaration(decl);
                    match converted {
                        Some(Expression::SequenceExpression(mut seq))
                            if seq.expressions.len() > 1 =>
                        {
                            let diff = names.iter().map(|name| {
                                let mut args = self.builder.vec();
                                args.push(Argument::from(self.builder.expression_string_literal(
                                    SPAN,
                                    name.name.clone(),
                                    None,
                                )));
                                args.push(Argument::from(
                                    self.builder.expression_identifier(SPAN, name.name.clone()),
                                ));
                                self.builder.expression_call(
                                    SPAN,
                                    self.builder.expression_identifier(SPAN, "_exports"),
                                    NONE,
                                    args,
                                    false,
                                )
                            });
                            seq.expressions.extend(self.builder.vec_from_iter(diff));
                            *it = self.builder.statement_expression(
                                SPAN,
                                self.builder.expression_sequence(
                                    SPAN,
                                    seq.expressions.take_in(self.allocator),
                                ),
                            )
                        }
                        Some(Expression::SequenceExpression(mut seq))
                            if seq.expressions.len() == 1 =>
                        {
                            let expr = seq.expressions.pop().expect("Length must be 1.");
                            if let Expression::AssignmentExpression(mut assignment) = expr {
                                if assignment.left.is_simple_assignment_target() {
                                    let name = assignment
                                        .left
                                        .to_simple_assignment_target()
                                        .get_identifier_name()
                                        .expect("Should have a name.");
                                    let mut args = self.builder.vec();
                                    args.push(Argument::from(
                                        self.builder.expression_string_literal(SPAN, name, None),
                                    ));
                                    args.push(Argument::AssignmentExpression(
                                        assignment.take_in_box(self.allocator),
                                    ));
                                    *it = self.builder.statement_expression(
                                        SPAN,
                                        self.builder.expression_call(
                                            SPAN,
                                            self.builder.expression_identifier(SPAN, "_exports"),
                                            NONE,
                                            args,
                                            false,
                                        ),
                                    )
                                } else {
                                    let symbols = self.extract_assignment_symbols(&assignment.left);
                                    let mut args =
                                        self.builder.vec_from_iter(symbols.iter().map(|name| {
                                            let mut args_exports = self.builder.vec();
                                            args_exports.push(Argument::from(
                                                self.builder.expression_string_literal(
                                                    SPAN,
                                                    name.name.clone(),
                                                    None,
                                                ),
                                            ));
                                            args_exports.push(Argument::from(
                                                self.builder
                                                    .expression_identifier(SPAN, name.name.clone()),
                                            ));
                                            self.builder.expression_call(
                                                SPAN,
                                                self.builder
                                                    .expression_identifier(SPAN, "_exports"),
                                                NONE,
                                                args_exports,
                                                false,
                                            )
                                        }));
                                    args.insert(0, Expression::AssignmentExpression(assignment));
                                    *it = self.builder.statement_expression(
                                        SPAN,
                                        self.builder.expression_sequence(
                                            SPAN,
                                            self.builder.vec_from_iter(args),
                                        ),
                                    );
                                }
                            }
                        }
                        None => {
                            *it = self.builder.statement_empty(SPAN);
                        }
                        _ => {
                            unimplemented!()
                        }
                    }
                } else {
                    for spec in export.specifiers.iter_mut() {
                        let local = spec.local.to_string();
                        let exported = spec.exported.to_string();
                        if is_identifier_name(exported.as_str()) {
                            self.export_tree.push(
                                self.builder.object_property_kind_object_property(
                                    SPAN,
                                    PropertyKind::Init,
                                    if is_identifier_name(exported.as_str()) {
                                        self.builder.property_key_static_identifier(
                                            SPAN,
                                            Atom::from_in(exported.as_str(), self.allocator),
                                        )
                                    } else {
                                        ast::PropertyKey::from(
                                            self.builder.expression_string_literal(
                                                SPAN,
                                                Atom::from_in(exported.as_str(), self.allocator),
                                                None,
                                            ),
                                        )
                                    },
                                    self.builder.expression_identifier(
                                        SPAN,
                                        Atom::from_in(local.as_str(), self.allocator),
                                    ),
                                    false,
                                    false,
                                    false,
                                ),
                            )
                        }
                    }
                    *it = self.builder.statement_empty(SPAN);
                }
            }
            Statement::ExportDefaultDeclaration(export_default) => {
                if export_default.declaration.is_expression() {
                    let expr = export_default
                        .take_in(self.allocator)
                        .declaration
                        .into_expression();
                    let mut args = self.builder.vec();
                    args.push(Argument::from(
                        self.builder
                            .expression_string_literal(SPAN, "default", None),
                    ));
                    args.push(Argument::from(expr));
                    *it = self.builder.statement_expression(
                        SPAN,
                        self.builder.expression_call(
                            SPAN,
                            self.builder.expression_identifier(SPAN, "_exports"),
                            NONE,
                            args,
                            false,
                        ),
                    );
                } else {
                    match export_default.declaration.take_in(self.allocator) {
                        ExportDefaultDeclarationKind::FunctionDeclaration(mut func) => {
                            let fn_name = func
                                .id
                                .clone_in(self.allocator)
                                .expect("Function declarations should have names.")
                                .name;
                            self.export_tree.push(
                                self.builder.object_property_kind_object_property(
                                    SPAN,
                                    PropertyKind::Init,
                                    self.builder.property_key_static_identifier(SPAN, "default"),
                                    self.builder.expression_identifier(SPAN, fn_name),
                                    false,
                                    false,
                                    false,
                                ),
                            );
                            self.top_level_function_decls
                                .push(func.take_in(self.allocator));
                            *it = self.builder.statement_empty(SPAN);
                        }
                        ExportDefaultDeclarationKind::ClassDeclaration(mut class) => {
                            self.top_level_classes.push(
                                class
                                    .id
                                    .clone_in(self.allocator)
                                    .expect("Class declarations should have names."),
                            );
                            self.export_tree.push(
                                self.builder.object_property_kind_object_property(
                                    SPAN,
                                    PropertyKind::Init,
                                    self.builder.property_key_static_identifier(SPAN, "default"),
                                    self.builder.void_0(SPAN),
                                    false,
                                    false,
                                    false,
                                ),
                            );
                            *it = self.builder.statement_expression(
                                SPAN,
                                self.builder.expression_assignment(
                                    SPAN,
                                    AssignmentOperator::Assign,
                                    self.builder
                                        .simple_assignment_target_assignment_target_identifier(
                                            SPAN,
                                            class
                                                .id
                                                .as_ref()
                                                .expect("Class declarations should have names.")
                                                .name,
                                        )
                                        .into(),
                                    self.builder.expression_class(
                                        SPAN,
                                        ClassType::ClassExpression,
                                        class.decorators.take_in(self.allocator),
                                        None,
                                        NONE,
                                        class.super_class.take(),
                                        NONE,
                                        class.implements.take_in(self.allocator),
                                        class.body.take_in(self.allocator),
                                        class.r#abstract,
                                        class.declare,
                                    ),
                                ),
                            );
                        }
                        _ => unreachable!(),
                    };
                }
            }
            _ => {}
        }
    }

    fn visit_import_declaration(&mut self, it: &mut ImportDeclaration<'a>) {
        self.importee.push(it.source.clone_in(self.allocator));
        if let Some(specifiers) = it.specifiers.as_ref() {
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(default) => {
                        self.imported_symbols
                            .push(default.local.clone_in(self.allocator));
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(namespace) => {
                        self.imported_symbols
                            .push(namespace.local.clone_in(self.allocator));
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                        self.imported_symbols
                            .push(specifier.local.clone_in(self.allocator));
                    }
                }
            }
        }
        self.imports
            .push(ImportMap::ImportDeclaration(it.take_in_box(self.allocator)));
        walk_mut::walk_import_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &mut ExportAllDeclaration<'a>) {
        self.importee.push(it.source.clone_in(self.allocator));
        self.imports.push(ImportMap::ExportAllDeclaration(
            it.take_in_box(self.allocator),
        ));
        walk_mut::walk_export_all_declaration(self, it);
    }
}

impl<'a> SystemJsTranspiler<'a> {
    pub fn is_valid_tla_scope(&self) -> bool {
        self.scope_stack
            .iter()
            .rev()
            .all(|flag| flag.is_block() || flag.is_top())
    }

    pub fn is_strict_top_level(&self) -> bool {
        let strict =
            matches!(self.path_stack.first(), Some(AstType::Program)) && self.path_stack.len() == 1;
        strict
    }

    pub fn extract_assignment_symbols(
        &self,
        decl: &AssignmentTarget<'a>,
    ) -> Vec<IdentifierReference<'a>> {
        match decl {
            AssignmentTarget::ArrayAssignmentTarget(arr) => {
                let mut idents = vec![];
                for elem in arr.elements.iter() {
                    if let Some(elem) = elem.clone_in(self.allocator) {
                        match elem {
                            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(_) => {}
                            _ => {
                                idents.extend(
                                    self.extract_assignment_symbols(&elem.into_assignment_target()),
                                );
                            }
                        }
                    }
                }
                if let Some(element) = arr.rest.clone_in(self.allocator) {
                    idents.extend(self.extract_assignment_symbols(&element.target));
                }
                idents
            }
            AssignmentTarget::ObjectAssignmentTarget(obj) => {
                let mut idents = vec![];
                for elem in obj.properties.iter() {
                    match elem {
                        ast::AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(
                            ident,
                        ) => {
                            idents.push(ident.binding.clone_in(self.allocator));
                        }
                        ast::AssignmentTargetProperty::AssignmentTargetPropertyProperty(
                            pattern,
                        ) => match pattern.binding {
                            AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(_) => {}
                            _ => {
                                idents.extend(
                                    self.extract_assignment_symbols(
                                        &pattern
                                            .binding
                                            .clone_in(self.allocator)
                                            .into_assignment_target(),
                                    ),
                                );
                            }
                        },
                    }
                }
                idents
            }
            _ => {
                if let Some(ident) = decl.to_simple_assignment_target().get_identifier_name() {
                    vec![self.builder.identifier_reference(SPAN, ident)]
                } else {
                    vec![]
                }
            }
        }
    }

    pub fn extract_variable_symbols(
        &self,
        decl: ast::BindingPattern<'a>,
    ) -> Vec<BindingIdentifier<'a>> {
        match decl.kind {
            ast::BindingPatternKind::BindingIdentifier(ident) => {
                vec![ident.clone()]
            }
            ast::BindingPatternKind::ArrayPattern(mut pattern) => {
                let mut idents = vec![];
                for elem in pattern.elements.iter_mut() {
                    if let Some(elem) = elem.clone_in(self.allocator) {
                        let incremental = self.extract_variable_symbols(elem);
                        idents.extend(incremental);
                    }
                }
                idents
            }
            ast::BindingPatternKind::ObjectPattern(mut pattern) => {
                let mut idents = vec![];
                for prop in pattern.properties.iter_mut() {
                    idents
                        .extend(self.extract_variable_symbols(prop.value.clone_in(self.allocator)));
                }
                idents
            }
            ast::BindingPatternKind::AssignmentPattern(pattern) => {
                self.extract_variable_symbols(pattern.left.clone_in(self.allocator))
            }
        }
    }

    fn hacked_var_decl_to_assignment(
        &self,
        decl: &mut ast::VariableDeclaration<'a>,
    ) -> Expression<'a> {
        decl.kind = VariableDeclarationKind::Var;
        let mut values = self.builder.vec();
        for decl_item in decl.declarations.iter_mut() {
            decl_item.kind = VariableDeclarationKind::Var;
            if let Some(init_val) = decl_item.init.take() {
                values.push(init_val);
                decl_item.init = Some(self.builder.void_0(SPAN))
            }
        }
        let simple_program = self.builder.program(
            SPAN,
            SourceType::cjs(),
            "",
            self.builder.vec(),
            None,
            self.builder.vec(),
            self.builder.vec1(Statement::VariableDeclaration(
                decl.take_in_box(self.allocator),
            )),
        );
        let generated_code = Codegen::new()
            .build(&simple_program)
            .code
            .strip_prefix("var ")
            .expect("Should include `var`.")
            .to_string();
        let parsed = Parser::new(self.allocator, generated_code.as_str(), SourceType::cjs())
            .parse_expression()
            .expect("Should be parsed.")
            .clone_in(self.allocator);
        let mut seq = match parsed {
            Expression::AssignmentExpression(assignment) => self.builder.sequence_expression(
                SPAN,
                self.builder
                    .vec1(Expression::AssignmentExpression(assignment)),
            ),
            Expression::SequenceExpression(mut seq) => {
                let mut exprs = seq.expressions.take_in(self.allocator);
                exprs.retain(|e| matches!(e, Expression::AssignmentExpression(_)));
                self.builder.sequence_expression(SPAN, exprs)
            }
            _ => self.builder.sequence_expression(SPAN, self.builder.vec()),
        };
        for (item, value) in seq.expressions.iter_mut().zip(values) {
            if let Expression::AssignmentExpression(assignment) = item {
                assignment.right = value
            }
        }
        Expression::SequenceExpression(self.builder.alloc(seq))
    }

    fn convert_declaration(&mut self, it: Declaration<'a>) -> Option<Expression<'a>> {
        match it {
            Declaration::VariableDeclaration(mut it)
                if (self.is_valid_tla_scope()
                    && matches!(it.kind, VariableDeclarationKind::Var))
                    || self.is_strict_top_level() =>
            {
                for pattern in it.declarations.iter() {
                    self.top_level_vars
                        .extend(self.extract_variable_symbols(pattern.id.clone_in(self.allocator)));
                }
                Some(self.hacked_var_decl_to_assignment(&mut it))
            }
            Declaration::FunctionDeclaration(mut function) if self.is_valid_tla_scope() => {
                self.top_level_function_decls
                    .push(function.take_in(self.allocator));
                None
            }
            Declaration::ClassDeclaration(mut decl) if self.is_valid_tla_scope() => {
                if let Some(id) = decl.id.as_ref() {
                    self.top_level_classes.push(id.clone_in(self.allocator));
                }
                Some(
                    self.builder.expression_sequence(
                        SPAN,
                        self.builder.vec1(
                            self.builder.expression_assignment(
                                SPAN,
                                AssignmentOperator::Assign,
                                self.builder
                                    .simple_assignment_target_assignment_target_identifier(
                                        SPAN,
                                        decl.id
                                            .as_ref()
                                            .expect("Should have a name for ClassDeclaration.")
                                            .name,
                                    )
                                    .into(),
                                self.builder.expression_class(
                                    SPAN,
                                    ClassType::ClassExpression,
                                    decl.decorators.take_in(self.allocator),
                                    None,
                                    NONE,
                                    decl.super_class.take(),
                                    NONE,
                                    decl.implements.take_in(self.allocator),
                                    decl.body.take_in(self.allocator),
                                    decl.r#abstract,
                                    decl.declare,
                                ),
                            ),
                        ),
                    ),
                )
            }
            _ => None,
        }
    }

    fn extract_declared_names(&self, it: &Declaration<'a>) -> Vec<BindingIdentifier<'a>> {
        match it {
            Declaration::VariableDeclaration(it) => Vec::from_iter(
                it.declarations
                    .iter()
                    .map(|decl| self.extract_variable_symbols(decl.id.clone_in(self.allocator)))
                    .flatten(),
            ),
            Declaration::FunctionDeclaration(it) => {
                vec![
                    it.id
                        .clone()
                        .expect("Function declarations should have names."),
                ]
            }
            Declaration::ClassDeclaration(it) => {
                vec![
                    it.id
                        .clone()
                        .expect("Class declarations should have names."),
                ]
            }
            _ => unreachable!(),
        }
    }

    /// Ported from https://github.com/rolldown/rolldown/blob/main/crates/rolldown_utils/src/ecmascript.rs#L14-L49
    pub fn legitimize_identifier_name(name: &str) -> Cow<str> {
        let mut legitimized = String::new();
        let mut chars_indices = name.char_indices();

        let mut first_invalid_char_index = None;

        if let Some((idx, first_char)) = chars_indices.next() {
            if !identifier::is_identifier_start(first_char) {
                first_invalid_char_index = Some(idx);
            }
        }

        if first_invalid_char_index.is_none() {
            first_invalid_char_index = chars_indices
                .find(|(_idx, char)| !identifier::is_identifier_part(*char))
                .map(|(idx, _)| idx);
        }

        let Some(first_invalid_char_index) = first_invalid_char_index else {
            return Cow::Borrowed(name);
        };

        let (first_valid_part, rest_part) = name.split_at(first_invalid_char_index);
        legitimized.push_str(first_valid_part);
        if first_invalid_char_index == 0 {
            legitimized.push('_');
        }
        for char in rest_part.chars() {
            if identifier::is_identifier_part(char) {
                legitimized.push(char);
            } else {
                legitimized.push('_');
            }
        }

        Cow::Owned(legitimized)
    }

    fn convert_import_to_function(&self, import: &mut ImportDeclaration<'a>) -> Expression<'a> {
        let name = format!("_{}", Self::legitimize_identifier_name(import.source.value.as_str()).to_string());
        let mut assigns = self.builder.vec();
        if let Some(specifiers) = import.specifiers.take() {
            for specifier in specifiers {
                assigns.push(
                    self.builder.statement_expression(
                        SPAN,
                        self.builder.expression_assignment(
                            SPAN,
                            AssignmentOperator::Assign,
                            AssignmentTarget::from(
                                self.builder
                                    .simple_assignment_target_assignment_target_identifier(
                                        SPAN,
                                        specifier.local().name.clone_in(self.allocator),
                                    ),
                            ),
                            match specifier {
                                ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                                    Expression::from(self.builder.member_expression_static(
                                        SPAN,
                                        self.builder.expression_identifier(
                                            SPAN,
                                            Atom::from_in(name.as_str(), self.allocator),
                                        ),
                                        self.builder.identifier_name(SPAN, "default"),
                                        false,
                                    ))
                                }
                                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                                    self.builder.expression_identifier(
                                        SPAN,
                                        Atom::from_in(name.as_str(), self.allocator),
                                    )
                                }
                                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                                    Expression::from(
                                        self.builder.member_expression_static(
                                            SPAN,
                                            self.builder.expression_identifier(
                                                SPAN,
                                                Atom::from_in(name.as_str(), self.allocator),
                                            ),
                                            self.builder.identifier_name(
                                                SPAN,
                                                specifier
                                                    .imported
                                                    .identifier_name()
                                                    .unwrap_or_else(|| specifier.local.name),
                                            ),
                                            false,
                                        ),
                                    )
                                }
                            },
                        ),
                    ),
                );
            }
        }
        let body = self
            .builder
            .function_body(SPAN, self.builder.vec(), assigns);
        self.builder.expression_function(
            SPAN,
            FunctionType::FunctionExpression,
            None,
            false,
            false,
            false,
            NONE,
            NONE,
            self.builder.formal_parameters(
                SPAN,
                FormalParameterKind::FormalParameter,
                self.builder.vec1(self.builder.formal_parameter(
                    SPAN,
                    self.builder.vec(),
                    self.builder.binding_pattern(
                        self.builder.binding_pattern_kind_binding_identifier(
                            SPAN,
                            Atom::from_in(name, self.allocator),
                        ),
                        NONE,
                        false,
                    ),
                    None,
                    false,
                    false,
                )),
                NONE,
            ),
            NONE,
            Some(body),
        )
    }

    fn convert_export_all_to_function(
        &self,
        export: &mut ExportAllDeclaration<'a>,
    ) -> Expression<'a> {
        let name = format!("_{}", Self::legitimize_identifier_name(export.source.value.as_str()).to_string());
        let body =
            self.builder.function_body(
                SPAN,
                self.builder.vec(),
                if let Some(_) = export.exported {
                    self.builder.vec1(self.builder.statement_expression(
                        SPAN,
                        self.builder.expression_call(
                            SPAN,
                            self.builder.expression_identifier(SPAN, "_exports"),
                            NONE,
                            self.builder.vec1(Argument::from(
                                self.builder.expression_object(SPAN, self.builder.vec()),
                            )),
                            false,
                        ),
                    ))
                } else {
                    let mut stmts = self.builder.vec();
                    // var _exportObj = {};
                    stmts.push(Statement::from(self.builder.declaration_variable(
                        SPAN,
                        VariableDeclarationKind::Var,
                        self.builder.vec1(
                            self.builder.variable_declarator(
                                SPAN,
                                VariableDeclarationKind::Var,
                                self.builder.binding_pattern(
                                    self.builder.binding_pattern_kind_binding_identifier(
                                        SPAN,
                                        "_exportObj",
                                    ),
                                    NONE,
                                    false,
                                ),
                                Some(self.builder.expression_object(SPAN, self.builder.vec())),
                                false,
                            ),
                        ),
                        false,
                    )));
                    //       for (var _key in _e) {
                    //         if (_key !== "default" && _key !== "__esModule") _exportObj[_key] = _e[_key];
                    //       }
                    stmts.push(self.builder.statement_for_in(
                        SPAN,
                        ast::ForStatementLeft::VariableDeclaration(
                            self.builder.alloc_variable_declaration(
                                SPAN,
                                VariableDeclarationKind::Var,
                                self.builder.vec1(self.builder.variable_declarator(
                                    SPAN,
                                    VariableDeclarationKind::Var,
                                    self.builder.binding_pattern(
                                        self.builder.binding_pattern_kind_binding_identifier(
                                            SPAN,
                                            Atom::from_in("_key", self.allocator),
                                        ),
                                        NONE,
                                        false,
                                    ),
                                    None,
                                    false,
                                )),
                                false,
                            ),
                        ),
                        self.builder.expression_identifier(
                            SPAN,
                            Atom::from_in(name.as_str(), self.allocator),
                        ),
                        self.builder.statement_block(
                            SPAN,
                            self.builder.vec1(self.builder.statement_expression(
                                SPAN,
                                self.builder.expression_assignment(
                                    SPAN,
                                    AssignmentOperator::Assign,
                                    AssignmentTarget::from(self.builder.member_expression_static(
                                        SPAN,
                                        self.builder.expression_identifier(SPAN, "_exportObj"),
                                        self.builder.identifier_name(SPAN, "_key"),
                                        false,
                                    )),
                                    Expression::from(self.builder.member_expression_static(
                                        SPAN,
                                        self.builder.expression_identifier(
                                            SPAN,
                                            Atom::from_in(name.as_str(), self.allocator),
                                        ),
                                        self.builder.identifier_name(SPAN, "_key"),
                                        false,
                                    )),
                                ),
                            )),
                        ),
                    ));
                    // _export(_exportObj);
                    stmts.push(
                        self.builder.statement_expression(
                            SPAN,
                            self.builder.expression_call(
                                SPAN,
                                self.builder.expression_identifier(SPAN, "_exports"),
                                NONE,
                                self.builder.vec1(Argument::from(
                                    self.builder.expression_identifier(
                                        SPAN,
                                        Atom::from_in("_exportObj", self.allocator),
                                    ),
                                )),
                                false,
                            ),
                        ),
                    );
                    stmts
                },
            );
        self.builder.expression_function(
            SPAN,
            FunctionType::FunctionExpression,
            None,
            false,
            false,
            false,
            NONE,
            NONE,
            self.builder.formal_parameters(
                SPAN,
                FormalParameterKind::FormalParameter,
                self.builder.vec1(self.builder.formal_parameter(
                    SPAN,
                    self.builder.vec(),
                    self.builder.binding_pattern(
                        self.builder.binding_pattern_kind_binding_identifier(
                            SPAN,
                            Atom::from_in(name, self.allocator),
                        ),
                        NONE,
                        false,
                    ),
                    None,
                    false,
                    false,
                )),
                NONE,
            ),
            NONE,
            Some(body),
        )
    }

    fn convert_import_map(&mut self) -> ArenaVec<'a, Expression<'a>> {
        let mut converted_imports = self.builder.vec();
        for import in self.imports.take_in(self.allocator).iter_mut() {
            match import {
                ImportMap::ImportDeclaration(import_decl) => {
                    converted_imports.push(
                        self.convert_import_to_function(&mut import_decl.take_in(self.allocator)),
                    );
                }
                ImportMap::ExportAllDeclaration(export_all_decl) => {
                    converted_imports.push(self.convert_export_all_to_function(
                        &mut export_all_decl.take_in(self.allocator),
                    ))
                }
            }
        }
        converted_imports
    }
}
