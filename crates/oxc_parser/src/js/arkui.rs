//! ArkUI parsing functions
//!
//! This module contains parsing logic for HarmonyOS ArkUI syntax including:
//! - Struct declarations (`struct ComponentName { ... }`)
//! - Annotation declarations (`annotation MyAnnotation { ... }`)
//! - ArkUI component expressions (`Column() { ... }`)

use oxc_allocator::{Box, Vec};
use oxc_ast::{NONE, ast::*};
use oxc_span::Span;

use crate::{
    Context, ParserConfig as Config, ParserImpl, StatementContext, diagnostics,
    lexer::Kind,
    modifiers::{ModifierKind, ModifierKinds, Modifiers},
};

use super::FunctionKind;

impl<'a, C: Config> ParserImpl<'a, C> {
    pub(crate) fn is_in_arkui_dsl_context(&self) -> bool {
        self.state.arkui_dsl_depth > 0
    }

    pub(crate) fn in_arkui_dsl_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.state.arkui_dsl_depth += 1;
        let result = f(self);
        self.state.arkui_dsl_depth -= 1;
        result
    }

    pub(crate) fn decorators_enable_arkui_dsl(decorators: &[Decorator<'a>]) -> bool {
        decorators.iter().any(|decorator| Self::is_arkui_dsl_decorator(&decorator.expression))
    }

    fn is_arkui_dsl_decorator(expression: &Expression<'a>) -> bool {
        match expression {
            Expression::Identifier(ident) => matches!(
                ident.name.as_str(),
                "Builder" | "LocalBuilder" | "Extend" | "AnimatableExtend" | "Styles"
            ),
            Expression::CallExpression(call) => Self::is_arkui_dsl_decorator(&call.callee),
            Expression::StaticMemberExpression(member) => {
                member.property.name == "Builder"
                    || member.property.name == "LocalBuilder"
                    || member.property.name == "Extend"
                    || member.property.name == "AnimatableExtend"
                    || member.property.name == "Styles"
            }
            _ => false,
        }
    }

    /// Parse a struct statement
    ///
    /// ## Example
    /// ```arkui
    /// @Component
    /// struct MyComponent {
    ///   @State message: string = 'Hello';
    ///   build() {
    ///     Column() {}
    ///   }
    /// }
    /// ```
    pub(crate) fn parse_struct_statement(
        &mut self,
        start_span: u32,
        stmt_ctx: StatementContext,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Statement<'a> {
        let decl = self.parse_struct_declaration(start_span, modifiers, decorators);
        if stmt_ctx.is_single_statement() {
            self.error(diagnostics::class_declaration(Span::new(
                decl.span.start,
                decl.body.span.start,
            )));
        }
        Statement::StructStatement(decl)
    }

    /// Parse a struct declaration
    pub(crate) fn parse_struct_declaration(
        &mut self,
        start_span: u32,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Box<'a, StructStatement<'a>> {
        self.bump_any(); // advance `struct`

        // Move span start to decorator position if decorators exist
        let mut start_span = start_span;
        if let Some(d) = decorators.first() {
            start_span = d.span.start;
        }

        let id = if self.cur_kind().is_binding_identifier() {
            self.parse_binding_identifier()
        } else {
            self.unexpected::<BindingIdentifier<'a>>()
        };

        let type_parameters = if self.is_ts { self.parse_ts_type_parameters() } else { None };
        let body = self.parse_struct_body();

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare, ModifierKind::Abstract]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        let span = self.end_span(start_span);

        let declare = modifiers.contains(ModifierKind::Declare);

        self.ast.alloc_struct_statement(span, decorators, id, type_parameters, body, declare)
    }

    /// Parse an annotation statement
    ///
    /// ## Example
    /// ```arkts
    /// @interface MyAnnotation {
    ///   value: string;
    ///   count?: number;
    /// }
    /// ```
    pub(crate) fn parse_annotation_statement(
        &mut self,
        start_span: u32,
        stmt_ctx: StatementContext,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Statement<'a> {
        let decl = self.parse_annotation_declaration(start_span, modifiers, decorators);
        if stmt_ctx.is_single_statement() {
            self.error(diagnostics::class_declaration(Span::new(
                decl.span.start,
                decl.body.span.start,
            )));
        }
        Statement::AnnotationDeclaration(decl)
    }

    /// Parse an annotation declaration
    ///
    /// Parses `@interface MyAnnotation { ... }` syntax
    pub(crate) fn parse_annotation_declaration(
        &mut self,
        start_span: u32,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Box<'a, AnnotationDeclaration<'a>> {
        // We should be at `interface` after `@` was consumed
        // Consume `interface` keyword
        if self.at(Kind::Interface) {
            self.bump_any();
        } else {
            // Call unexpected to report error; the return value is () which we intentionally ignore
            #[allow(unused_must_use, unused_variables)]
            let _: () = self.unexpected::<()>();
        }

        // Move span start to @ position
        let start_span = start_span;

        let id = if self.cur_kind().is_binding_identifier() {
            self.parse_binding_identifier()
        } else {
            self.unexpected::<BindingIdentifier<'a>>()
        };

        let body = self.parse_annotation_body();

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare, ModifierKind::Abstract]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        let span = self.end_span(start_span);

        let declare = modifiers.contains(ModifierKind::Declare);

        self.ast.alloc_annotation_declaration(span, decorators, id, body, declare)
    }

    /// Parse annotation body containing properties
    fn parse_annotation_body(&mut self) -> Box<'a, AnnotationBody<'a>> {
        let span = self.start_span();
        let annotation_elements =
            self.parse_normal_list_breakable(Kind::LCurly, Kind::RCurly, |p| {
                // Skip empty annotation element `;`
                if p.eat(Kind::Semicolon) {
                    while p.eat(Kind::Semicolon) {
                        // consume multiple semicolons
                    }
                    if p.at(Kind::RCurly) {
                        return None;
                    }
                }
                Some(Self::parse_annotation_element(p))
            });
        self.ast.alloc_annotation_body(self.end_span(span), annotation_elements)
    }

    /// Parse an annotation element (property)
    fn parse_annotation_element(&mut self) -> AnnotationElement<'a> {
        let span = self.start_span();

        let decorators = self.parse_decorators();
        let modifiers = self.parse_modifiers(
            /* permit_const_as_modifier */ true,
            /* stop_on_start_of_class_static_block */ false,
        );

        self.verify_modifiers(
            &modifiers,
            ModifierKinds::all_except([ModifierKind::Export]),
            false,
            diagnostics::cannot_appear_on_class_elements,
        );

        // Parse property key
        let (name, computed) = self.parse_property_name();

        // Parse optional type annotation
        let type_annotation = if self.is_ts && self.eat(Kind::Colon) {
            let span = self.start_span();
            let ts_type = self.parse_ts_type();
            Some(self.ast.alloc_ts_type_annotation(self.end_span(span), ts_type))
        } else {
            None
        };

        // Parse optional initializer (default value)
        let initializer = self
            .eat(Kind::Eq)
            .then(|| self.context(Context::In, Context::Yield | Context::Await, Self::parse_expr));

        // Semicolon is optional in annotation bodies
        let _ = self.eat(Kind::Semicolon);

        let r#type = PropertyDefinitionType::PropertyDefinition;
        let property_def = self.ast.alloc_property_definition(
            self.end_span(span),
            r#type,
            decorators,
            name,
            type_annotation,
            initializer,
            computed,
            modifiers.contains(ModifierKind::Static),
            false, // declare
            modifiers.contains(ModifierKind::Override),
            false, // optional - not supported
            false, // definite - not supported
            modifiers.contains(ModifierKind::Readonly),
            modifiers.accessibility(),
        );

        AnnotationElement::PropertyDefinition(property_def)
    }

    /// Parse struct body containing properties and methods
    fn parse_struct_body(&mut self) -> Box<'a, StructBody<'a>> {
        let span = self.start_span();
        let struct_elements = self.parse_normal_list_breakable(Kind::LCurly, Kind::RCurly, |p| {
            // Skip empty struct element `;`
            if p.eat(Kind::Semicolon) {
                while p.eat(Kind::Semicolon) {
                    // consume multiple semicolons
                }
                if p.at(Kind::RCurly) {
                    return None;
                }
            }
            Some(Self::parse_struct_element(p))
        });
        self.ast.alloc_struct_body(self.end_span(span), struct_elements)
    }

    /// Parse a struct element (property or method)
    fn parse_struct_element(&mut self) -> StructElement<'a> {
        let span = self.start_span();

        let decorators = self.parse_decorators();
        let modifiers = self.parse_modifiers(
            /* permit_const_as_modifier */ true,
            /* stop_on_start_of_class_static_block */ false,
        );

        self.verify_modifiers(
            &modifiers,
            ModifierKinds::all_except([ModifierKind::Export]),
            false,
            diagnostics::cannot_appear_on_class_elements,
        );

        // Check for get/set accessors (similar to class elements)
        let r#abstract = modifiers.contains(ModifierKind::Abstract);
        let r#type = if r#abstract {
            MethodDefinitionType::TSAbstractMethodDefinition
        } else {
            MethodDefinitionType::MethodDefinition
        };

        if self.parse_contextual_modifier(Kind::Get) {
            return StructElement::MethodDefinition(self.parse_struct_accessor_declaration(
                span,
                r#type,
                MethodDefinitionKind::Get,
                &modifiers,
                decorators,
            ));
        }

        if self.parse_contextual_modifier(Kind::Set) {
            return StructElement::MethodDefinition(self.parse_struct_accessor_declaration(
                span,
                r#type,
                MethodDefinitionKind::Set,
                &modifiers,
                decorators,
            ));
        }

        // Parse property or method declaration (similar to class)
        if self.cur_kind().is_identifier_or_keyword()
            || self.at(Kind::Star)
            || self.at(Kind::LBrack)
        {
            return self.parse_property_or_method_declaration_for_struct(
                span, r#type, &modifiers, decorators,
            );
        }

        // Otherwise parse as property definition
        self.parse_property_definition_for_struct(span, &modifiers, decorators)
    }

    /// Parse property or method declaration for struct (similar to class)
    fn parse_property_or_method_declaration_for_struct(
        &mut self,
        span: u32,
        r#type: MethodDefinitionType,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> StructElement<'a> {
        let generator = self.eat(Kind::Star);
        let (name, computed) = self.parse_property_name();

        // Handle optional ? token (aligned with class parsing)
        let cur_token = self.cur_token();
        let optional_span = (cur_token.kind() == Kind::Question).then(|| {
            let span = cur_token.span();
            self.bump_any();
            span
        });
        let optional = optional_span.is_some();

        // Check if this is a method (generator or has parentheses or type parameters)
        if generator || matches!(self.cur_kind(), Kind::LParen | Kind::LAngle) {
            return StructElement::MethodDefinition(self.parse_method_declaration_for_struct(
                span, r#type, generator, name, computed, optional, modifiers, decorators,
            ));
        }

        // Otherwise parse as property
        let definite = self.eat(Kind::Bang);

        if definite && let Some(optional_span) = optional_span {
            self.error(diagnostics::optional_definite_property(optional_span.expand_right(1)));
        }

        self.parse_property_declaration_for_struct(
            span,
            name,
            computed,
            optional_span,
            definite,
            modifiers,
            decorators,
        )
    }

    /// Parse method declaration for struct (similar to class)
    fn parse_method_declaration_for_struct(
        &mut self,
        span: u32,
        r#type: MethodDefinitionType,
        generator: bool,
        name: PropertyKey<'a>,
        computed: bool,
        optional: bool,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Box<'a, MethodDefinition<'a>> {
        let is_arkui_dsl_method = self.source_type.is_arkui()
            && ((!computed
                && name
                    .static_name()
                    .is_some_and(|name| matches!(name.as_ref(), "build" | "pageTransition")))
                || Self::decorators_enable_arkui_dsl(decorators.as_slice()));
        let value = if is_arkui_dsl_method {
            self.in_arkui_dsl_context(|p| {
                p.parse_method(
                    modifiers.contains(ModifierKind::Async),
                    generator,
                    FunctionKind::ClassMethod,
                )
            })
        } else {
            self.parse_method(
                modifiers.contains(ModifierKind::Async),
                generator,
                FunctionKind::ClassMethod,
            )
        };
        self.ast.alloc_method_definition(
            self.end_span(span),
            r#type,
            decorators,
            name,
            value,
            MethodDefinitionKind::Method,
            computed,
            modifiers.contains(ModifierKind::Static),
            modifiers.contains(ModifierKind::Override),
            optional,
            modifiers.accessibility(),
        )
    }

    /// Parse property declaration for struct (similar to class)
    fn parse_property_declaration_for_struct(
        &mut self,
        span: u32,
        name: PropertyKey<'a>,
        computed: bool,
        optional_span: Option<Span>,
        definite: bool,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> StructElement<'a> {
        let optional = optional_span.is_some();

        // Parse optional type annotation
        let type_annotation = if self.is_ts && self.eat(Kind::Colon) {
            let span = self.start_span();
            let ts_type = self.parse_ts_type();
            Some(self.ast.alloc_ts_type_annotation(self.end_span(span), ts_type))
        } else {
            None
        };

        // Parse optional initializer
        let initializer = self
            .eat(Kind::Eq)
            .then(|| self.context(Context::In, Context::Yield | Context::Await, Self::parse_expr));

        // Semicolon is optional in struct bodies
        let _ = self.eat(Kind::Semicolon);

        let r#type = PropertyDefinitionType::PropertyDefinition;
        let property_def = self.ast.alloc_property_definition(
            self.end_span(span),
            r#type,
            decorators,
            name,
            type_annotation,
            initializer,
            computed,
            modifiers.contains(ModifierKind::Static),
            false, // declare
            modifiers.contains(ModifierKind::Override),
            optional,
            definite,
            modifiers.contains(ModifierKind::Readonly),
            modifiers.accessibility(),
        );

        StructElement::PropertyDefinition(property_def)
    }

    /// Parse an accessor declaration (get/set) for struct
    fn parse_struct_accessor_declaration(
        &mut self,
        span: u32,
        r#type: MethodDefinitionType,
        kind: MethodDefinitionKind,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> Box<'a, MethodDefinition<'a>> {
        let (name, computed) = self.parse_property_name();
        let value = self.parse_method(
            modifiers.contains(ModifierKind::Async),
            false,
            FunctionKind::ClassMethod,
        );
        let method_definition = self.ast.alloc_method_definition(
            self.end_span(span),
            r#type,
            decorators,
            name,
            value,
            kind,
            computed,
            modifiers.contains(ModifierKind::Static),
            modifiers.contains(ModifierKind::Override),
            false,
            modifiers.accessibility(),
        );
        match kind {
            MethodDefinitionKind::Get => self.check_getter(&method_definition.value),
            MethodDefinitionKind::Set => self.check_setter(&method_definition.value),
            _ => {}
        }
        self.verify_modifiers(
            modifiers,
            ModifierKinds::all_except([ModifierKind::Async, ModifierKind::Declare]),
            false,
            diagnostics::modifier_cannot_be_used_here,
        );
        method_definition
    }

    /// Parse a property definition for struct (fallback when not identifier/keyword/star/bracket)
    fn parse_property_definition_for_struct(
        &mut self,
        span: u32,
        modifiers: &Modifiers,
        decorators: Vec<'a, Decorator<'a>>,
    ) -> StructElement<'a> {
        // Parse property key
        let (name, computed) = self.parse_property_name();
        let optional_span = (self.cur_token().kind() == Kind::Question).then(|| {
            let span = self.cur_token().span();
            self.bump_any();
            span
        });
        let definite = self.eat(Kind::Bang);

        if definite && let Some(optional_span) = optional_span {
            self.error(diagnostics::optional_definite_property(optional_span.expand_right(1)));
        }

        self.parse_property_declaration_for_struct(
            span,
            name,
            computed,
            optional_span,
            definite,
            modifiers,
            decorators,
        )
    }

    /// Parse an ArkUI component expression
    ///
    /// ## Example
    /// ```arkui
    /// Column() {
    ///   Text('Hello')
    ///   Button('Click')
    /// }
    /// ```
    pub(crate) fn parse_arkui_component_expression(
        &mut self,
        callee: Expression<'a>,
    ) -> Expression<'a> {
        let span = self.start_span();

        // Parse type arguments if present (for generic components)
        let type_arguments = if self.is_ts { self.try_parse_type_arguments() } else { None };

        // Parse arguments
        let opening_span = self.cur_token().span();
        self.expect(Kind::LParen);
        let (exprs, _) = self.parse_delimited_list(
            Kind::RParen,
            Kind::Comma,
            opening_span,
            Self::parse_assignment_expression_or_higher,
        );
        let mut arguments = self.ast.vec();
        for expr in exprs {
            arguments.push(Argument::from(expr));
        }
        self.expect(Kind::RParen);

        // Parse children block if present
        let children = if self.eat(Kind::LCurly) {
            self.in_arkui_dsl_context(|p| {
                let mut children_vec = p.ast.vec();
                while !p.at(Kind::RCurly) && !p.has_fatal_error() {
                    // Parse child element
                    let child = p.parse_arkui_child();
                    children_vec.push(child);

                    // Optional semicolon between children
                    let _ = p.eat(Kind::Semicolon);
                }
                p.expect(Kind::RCurly);
                children_vec
            })
        } else {
            self.ast.vec()
        };

        let component_span = self.end_span(span);
        let chain_expressions = self.parse_arkui_component_chain_expressions();
        Expression::ArkUIComponentExpression(self.ast.alloc_ark_ui_component_expression(
            component_span,
            callee,
            type_arguments,
            arguments,
            children,
            chain_expressions,
        ))
    }

    /// Parse an ArkUI component expression after arguments have been parsed
    pub(crate) fn parse_arkui_component_expression_after_args(
        &mut self,
        span: u32,
        callee: Expression<'a>,
        type_arguments: Option<Box<'a, TSTypeParameterInstantiation<'a>>>,
        arguments: Vec<'a, Argument<'a>>,
    ) -> Expression<'a> {
        // Parse children block
        let children = if self.eat(Kind::LCurly) {
            self.in_arkui_dsl_context(|p| {
                let mut children_vec = p.ast.vec();
                while !p.at(Kind::RCurly) && !p.has_fatal_error() {
                    // Parse child element
                    let child = p.parse_arkui_child();
                    children_vec.push(child);

                    // Optional semicolon between children
                    let _ = p.eat(Kind::Semicolon);
                }
                p.expect(Kind::RCurly);
                children_vec
            })
        } else {
            self.ast.vec()
        };

        let component_span = self.end_span(span);
        let chain_expressions = self.parse_arkui_component_chain_expressions();
        Expression::ArkUIComponentExpression(self.ast.alloc_ark_ui_component_expression(
            component_span,
            callee,
            type_arguments,
            arguments,
            children,
            chain_expressions,
        ))
    }

    fn parse_arkui_component_chain_expressions(&mut self) -> Vec<'a, CallExpression<'a>> {
        let mut chain_expressions = self.ast.vec();
        while self.eat(Kind::Dot) {
            if !self.cur_kind().is_identifier_or_keyword() {
                break;
            }
            let ident_span = self.start_span();
            let ident = self.parse_identifier_name();
            if !self.at(Kind::LParen) {
                break;
            }
            let member_span = self.end_span(ident_span);
            let member_expr = self.ast.member_expression_static(
                member_span,
                self.ast.expression_this(member_span),
                ident,
                false,
            );
            let call_span = self.start_span();
            let opening_span = self.cur_token().span();
            self.expect(Kind::LParen);
            let (exprs, _) = self.parse_delimited_list(
                Kind::RParen,
                Kind::Comma,
                opening_span,
                Self::parse_assignment_expression_or_higher,
            );
            let mut call_args = self.ast.vec();
            for expr in exprs {
                call_args.push(Argument::from(expr));
            }
            self.expect(Kind::RParen);
            chain_expressions.push(self.ast.call_expression(
                self.end_span(call_span),
                Expression::from(member_expr),
                NONE,
                call_args,
                false,
            ));
        }
        chain_expressions
    }

    /// Parse an ArkUI child element
    fn parse_arkui_child(&mut self) -> ArkUIChild<'a> {
        // Check for control flow statements first (if, for, while, switch, etc.)
        // These are commonly used in ArkUI children for conditional rendering
        match self.cur_kind() {
            Kind::If
            | Kind::For
            | Kind::While
            | Kind::Do
            | Kind::Switch
            | Kind::Try
            | Kind::With
            | Kind::Break
            | Kind::Continue
            | Kind::Return
            | Kind::Throw
            | Kind::Debugger => {
                // Parse as statement
                let stmt = self.parse_statement_list_item(StatementContext::StatementList);
                return ArkUIChild::Statement(self.alloc(stmt));
            }
            _ => {}
        }

        // Check if this is another component expression
        if self.cur_kind().is_identifier_or_keyword() {
            let checkpoint = self.checkpoint();
            let ident_expr = self.parse_identifier_expression();

            if self.at(Kind::LParen) {
                // This is a component expression
                let component_expr = self.parse_arkui_component_expression(ident_expr);
                if let Expression::ArkUIComponentExpression(expr) = component_expr {
                    return ArkUIChild::Component(expr);
                } else {
                    unreachable!(
                        "parse_arkui_component_expression should return ArkUIComponentExpression"
                    );
                }
            } else {
                // Not a component, restore and parse as regular expression
                self.rewind(checkpoint);
            }
        }

        // Parse as regular expression
        let expr = self.parse_assignment_expression_or_higher();
        ArkUIChild::Expression(self.alloc(expr))
    }
}
