use crate::transpiler::{ImportMap, SystemJsTranspiler};
use oxc::allocator::{CloneIn, TakeIn};
use oxc::ast::ast::{
    Argument, ArrayExpressionElement, AssignmentOperator, AssignmentTarget,
    AssignmentTargetMaybeDefault, BindingIdentifier, Class, ClassType,
    Declaration, ExportAllDeclaration, Expression, FormalParameterKind, FunctionType,
    ImportDeclaration, Program, PropertyKind, Statement, VariableDeclarationKind,
    VariableDeclarator,
};
use oxc::ast::{ast, match_declaration, AstType, NONE};
use oxc::ast_visit::{walk_mut, Visit, VisitMut};
use oxc::codegen::Codegen;
use oxc::parser::Parser;
use oxc::semantic::{ScopeFlags, ScopeId};
use oxc::span::{Atom, SourceType, SPAN};
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
        println!("Visiting program with options: {:?}", self.options);
        walk_mut::walk_program(self, it);
        let targets = self
            .builder
            .vec_from_iter(self.top_level_vars.iter().map(|var| {
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
        let mut return_object_items = self.builder.vec();
        return_object_items.push(self.builder.object_property_kind_object_property(
            SPAN,
            PropertyKind::Init,
            self.builder.property_key_static_identifier(SPAN, "setter"),
            self.builder.expression_array(SPAN, self.builder.vec()),
            false,
            false,
            false,
        ));
        let mut new_body = it.body.take_in(self.allocator);
        new_body.retain(|s| !matches!(s, Statement::EmptyStatement(_) | Statement::DebuggerStatement(_)));
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
        println!("Visiting expression: {:?}", expr);
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

    fn visit_variable_declarator(&mut self, it: &mut VariableDeclarator<'a>) {
        if self.is_valid_tla_scope() {
            self.top_level_vars
                .extend(self.extract_variable_symbols(it.id.clone_in(self.allocator)));
        }
        walk_mut::walk_variable_declarator(self, it);
    }

    fn visit_class(&mut self, it: &mut Class<'a>) {
        if self.is_valid_tla_scope() {
            if let Some(id) = it.id.clone() {
                self.top_level_classes.push(id);
            }
        }
        walk_mut::walk_class(self, it);
    }

    fn visit_statement(&mut self, it: &mut Statement<'a>) {
        walk_mut::walk_statement(self, it);
        match it {
            decl @ match_declaration!(Statement) => {
                let declaration = decl.take_in(self.allocator).into_declaration();
                let converted = self.convert_declaration(declaration);
                if let Some(new_code) = converted {
                    *decl = self.builder.statement_expression(SPAN, new_code)
                }
            }
            Statement::ImportDeclaration(_) => *it = self.builder.statement_empty(SPAN),
            Statement::ExportAllDeclaration(_) => *it = self.builder.statement_empty(SPAN),
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = export.declaration.take() {
                    let names = self.extract_declared_names(&decl);
                    let mut converted = self.convert_declaration(decl);
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
                            let mut expr = seq.expressions.pop().unwrap();
                            if let Expression::AssignmentExpression(mut assignment) = expr {
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
                            }
                        }
                        None => {}
                        _ => {
                            println!("Unsupported statement declaration: {converted:?}");
                            unimplemented!()
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn visit_import_declaration(&mut self, it: &mut ImportDeclaration<'a>) {
        self.importee.push(it.source.clone_in(self.allocator));
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

    pub fn extract_variable_symbols(
        &self,
        decl: ast::BindingPattern<'a>,
    ) -> Vec<BindingIdentifier<'a>> {
        match decl.kind {
            ast::BindingPatternKind::BindingIdentifier(mut ident) => {
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

    pub fn convert_binding_identifier_to_assignment(
        &self,
        ident: &mut BindingIdentifier<'a>,
    ) -> AssignmentTarget<'a> {
        AssignmentTarget::AssignmentTargetIdentifier(
            self.builder
                .alloc(self.builder.identifier_reference(SPAN, ident.name.clone())),
        )
    }

    pub fn convert_to_assignment(&mut self, left: ast::BindingPattern<'a>) -> AssignmentTarget<'a> {
        match left.kind {
            ast::BindingPatternKind::BindingIdentifier(mut ident) => {
                self.convert_binding_identifier_to_assignment(&mut ident)
            }
            ast::BindingPatternKind::ArrayPattern(mut array) => {
                let elements = std::mem::replace(&mut array.elements, self.builder.vec());
                let mut targets = self.builder.vec();
                for elem in elements {
                    targets.push(elem.map(|mut e| {
                        AssignmentTargetMaybeDefault::from(self.convert_to_assignment(e))
                    }));
                }
                AssignmentTarget::ArrayAssignmentTarget(
                    self.builder
                        .alloc(self.builder.array_assignment_target(SPAN, targets, None)),
                )
            }
            _ => unreachable!(),
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
            Declaration::VariableDeclaration(mut it) if self.is_valid_tla_scope() => {
                Some(self.hacked_var_decl_to_assignment(&mut it))
            }
            Declaration::FunctionDeclaration(mut function) if self.is_valid_tla_scope() => {
                self.top_level_function_decls
                    .push(function.take_in(self.allocator));
                None
            }
            Declaration::ClassDeclaration(mut decl) if self.is_valid_tla_scope() => Some(
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
            ),
            _ if self.is_valid_tla_scope() => {
                unreachable!()
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
}
