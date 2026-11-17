use oxc_allocator::{ArenaBox, ArenaVec};
use oxc_ast::ast::*;
use oxc_span::GetSpan;
use oxc_str::Ident;

use crate::{
    Context, ParserConfig as Config, ParserImpl, diagnostics,
    lexer::Kind,
    modifiers::{ModifierKind, ModifierKinds, Modifiers},
    state::CoverInitializedName,
};

use super::FunctionKind;

impl<'a, C: Config> ParserImpl<'a, C> {
    /// [Object Expression](https://tc39.es/ecma262/#sec-object-initializer)
    /// `ObjectLiteral`[Yield, Await] :
    ///     { }
    ///     { `PropertyDefinitionList`[?Yield, ?Await] }
    ///     { `PropertyDefinitionList`[?Yield, ?Await] , }
    pub(crate) fn parse_object_expression(&mut self) -> ArenaBox<'a, ObjectExpression<'a>> {
        debug_assert!(self.at(Kind::LCurly));
        let start = self.cur_start();
        let opening_span = self.cur_token().span();
        self.advance(Kind::LCurly);
        // Check if this is an ArkUI object literal with expression statements (e.g., { .backgroundColor(...) })
        // In ArkUI, object literals can contain expression statements starting with dots
        if self.supports_arkui_dsl() && self.is_in_arkui_dsl_context() && self.at(Kind::Dot) {
            return self.parse_arkui_object_expression_with_statements(start, opening_span);
        }
        let (object_expression_properties, comma_start) = self.context_add(Context::In, |p| {
            p.parse_delimited_list(
                Kind::RCurly,
                Kind::Comma,
                opening_span,
                Self::parse_object_expression_property,
            )
        });
        if let Some(comma_start) = comma_start
            && matches!(
                object_expression_properties.last(),
                Some(ObjectPropertyKind::SpreadProperty(_))
            )
        {
            self.state.trailing_commas.insert(start, self.end_span(comma_start));
        }
        self.expect(Kind::RCurly);
        ObjectExpression::boxed(self.end_span(start), object_expression_properties, self)
    }

    /// Parse ArkUI object literal with expression statements
    /// Example: { .backgroundColor('#ffffeef0') }
    /// In ArkUI, object literals can contain expression statements starting with dots
    fn parse_arkui_object_expression_with_statements(
        &mut self,
        span: u32,
        _opening_span: Span,
    ) -> ArenaBox<'a, ObjectExpression<'a>> {
        use crate::lexer::Kind;
        let mut properties = ArenaVec::new_in(self);

        // Parse expression statements until closing brace
        while !self.at(Kind::RCurly) && !self.has_fatal_error() {
            if self.at(Kind::Dot) {
                // Parse expression statement starting with dot as LeadingDotExpression
                let expr_span = self.cur_start();
                let expr = self.parse_leading_dot_expression();
                let expr_end_span = self.end_span(expr_span);

                // Create a property with the expression as value
                // Use a synthetic key for the expression statement
                let key = PropertyKey::StaticIdentifier(IdentifierName::boxed(
                    expr_end_span,
                    Ident::from(""),
                    self,
                ));
                let property = ObjectProperty::boxed(
                    expr_end_span,
                    PropertyKind::Init,
                    key,
                    expr,
                    false, // not a method
                    false, // not shorthand
                    false, // not computed
                    self,
                );
                properties.push(ObjectPropertyKind::ObjectProperty(property));

                // Optional semicolon or comma (both are allowed after expression statements)
                if self.eat(Kind::Semicolon) {
                    // Semicolon consumed
                } else if self.at(Kind::Comma) {
                    // Comma after expression statement (e.g., .method1().method2(),)
                    self.expect(Kind::Comma);
                }
                // If neither semicolon nor comma, continue to next token (could be closing brace or next property)
            } else {
                // Parse as normal object property
                let property = self.parse_object_expression_property();
                properties.push(property);

                // Expect comma or closing brace
                if !self.at(Kind::RCurly) {
                    self.expect(Kind::Comma);
                }
            }
        }

        self.expect(Kind::RCurly);
        ObjectExpression::boxed(self.end_span(span), properties, self)
    }

    fn parse_object_expression_property(&mut self) -> ObjectPropertyKind<'a> {
        match self.cur_kind() {
            Kind::Ident => ObjectPropertyKind::ObjectProperty(
                self.parse_object_literal_element_with_identifier_key(),
            ),
            Kind::Dot3 => ObjectPropertyKind::SpreadProperty(self.parse_spread_element()),
            _ => ObjectPropertyKind::ObjectProperty(self.parse_object_literal_element()),
        }
    }

    /// Parse a property whose key is an identifier that is not a keyword in any context,
    /// so it is not a modifier, `get` or `set`: `a`, `a: b`, `a = b` or `a() {}`.
    fn parse_object_literal_element_with_identifier_key(
        &mut self,
    ) -> ArenaBox<'a, ObjectProperty<'a>> {
        let start = self.cur_start();
        let (span, name) = self.parse_identifier_kind(Kind::Ident);
        let key = PropertyKey::StaticIdentifier(IdentifierName::boxed(span, name, self));

        let kind = self.cur_kind();
        let (property_kind, value, method, shorthand) = if kind == Kind::Colon {
            self.bump_any(); // `:`
            (PropertyKind::Init, self.parse_assignment_expression_or_higher(), false, false)
        } else if matches!(kind, Kind::LParen | Kind::LAngle) {
            let method =
                self.parse_method(/* async */ false, None, FunctionKind::ObjectMethod);
            (PropertyKind::Init, Expression::FunctionExpression(method), true, false)
        } else if self.source_type.is_ets_static() && kind == Kind::Eq {
            // Static ETS annotation argument: `{ value = "ok" }`
            self.bump_any(); // `=`
            (PropertyKind::EtsEquals, self.parse_assignment_expression_or_higher(), false, false)
        } else {
            // CoverInitializedName ({ foo = bar })
            if kind == Kind::Eq {
                self.bump_any(); // `=`
                let initializer = self.parse_assignment_expression_or_higher();
                let cover = CoverInitializedName { span: self.end_span(start), initializer };
                self.state.cover_initialized_name.insert(start, cover);
            }
            (PropertyKind::Init, Expression::new_identifier(span, name, self), false, true)
        };
        ObjectProperty::boxed(
            self.end_span(start),
            property_kind,
            key,
            value,
            method,
            shorthand,
            /* computed */ false,
            self,
        )
    }

    /// `PropertyDefinition`[Yield, Await]
    fn parse_object_literal_element(&mut self) -> ArenaBox<'a, ObjectProperty<'a>> {
        let start = self.cur_start();

        let modifiers = self.parse_modifiers(
            /* permit_const_as_modifier */ false,
            /* stop_on_start_of_class_static_block */ false,
        );

        if self.parse_contextual_modifier(Kind::Get) {
            if self.source_type.is_ets_static() {
                self.error(diagnostics::ets_unsupported_syntax(
                    "Methods in object literals",
                    self.cur_token().span(),
                ));
            }
            return self.parse_method_getter_setter(start, PropertyKind::Get, &modifiers);
        }

        if self.parse_contextual_modifier(Kind::Set) {
            if self.source_type.is_ets_static() {
                self.error(diagnostics::ets_unsupported_syntax(
                    "Methods in object literals",
                    self.cur_token().span(),
                ));
            }
            return self.parse_method_getter_setter(start, PropertyKind::Set, &modifiers);
        }

        let asterisk_token = self.eat(Kind::Star).then_some(self.prev_token_end - 1);
        let key_kind = self.cur_kind();
        let token_is_identifier = key_kind
            .is_identifier_reference(self.ctx.has_yield(), self.ctx.has_await())
            // In a list that may be the parameters of an arrow function, `({ await }) => {}`,
            // a reserved `await` or `yield` is reported as the binding it would be.
            || (matches!(key_kind, Kind::Await | Kind::Yield) && self.state.cover_depth > 0);
        let (key, computed) = self.parse_property_name();

        if asterisk_token.is_some() || matches!(self.cur_kind(), Kind::LParen | Kind::LAngle) {
            self.verify_modifiers(
                &modifiers,
                ModifierKinds::new([ModifierKind::Async]),
                true,
                diagnostics::modifier_cannot_be_used_here,
            );
            let method = self.parse_method(
                modifiers.contains_async(),
                asterisk_token,
                FunctionKind::ObjectMethod,
            );
            return ObjectProperty::boxed(
                self.end_span(start),
                PropertyKind::Init,
                key,
                Expression::FunctionExpression(method),
                /* method */ true,
                /* shorthand */ false,
                computed,
                self,
            );
        }

        self.verify_modifiers(
            &modifiers,
            ModifierKinds::none(),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        if self.source_type.is_ets_static() && computed {
            self.error(diagnostics::ets_unsupported_syntax(
                "Computed properties in object literals",
                key.span(),
            ));
        }

        if self.source_type.is_ets_static() && token_is_identifier && self.eat(Kind::Eq) {
            let value = self.parse_assignment_expression_or_higher();
            return ObjectProperty::boxed(
                self.end_span(start),
                PropertyKind::EtsEquals,
                key,
                value,
                /* method */ false,
                /* shorthand */ false,
                computed,
                self,
            );
        }

        let shorthand = token_is_identifier && !self.at(Kind::Colon);
        if !shorthand {
            return self.parse_property_definition_assignment(start, key, computed);
        }

        let PropertyKey::StaticIdentifier(identifier_name) = &key else {
            return self.unexpected();
        };
        if matches!(key_kind, Kind::Await | Kind::Yield) {
            self.check_identifier_with_span(key_kind, self.ctx, identifier_name.span);
        }
        // CoverInitializedName ({ foo = bar })
        if self.eat(Kind::Eq) {
            let initializer = self.parse_assignment_expression_or_higher();
            let cover = CoverInitializedName { span: self.end_span(start), initializer };
            self.state.cover_initialized_name.insert(start, cover);
        }
        let value = Expression::new_identifier(identifier_name.span, identifier_name.name, self);
        ObjectProperty::boxed(
            self.end_span(start),
            PropertyKind::Init,
            key,
            value,
            /* method */ false,
            /* shorthand */ true,
            computed,
            self,
        )
    }

    /// `PropertyDefinition`[Yield, Await] :
    ///   ... `AssignmentExpression`[+In, ?Yield, ?Await]
    pub(crate) fn parse_spread_element(&mut self) -> ArenaBox<'a, SpreadElement<'a>> {
        let start = self.cur_start();
        self.bump_any(); // advance `...`
        let argument = self.parse_assignment_expression_or_higher();
        SpreadElement::boxed(self.end_span(start), argument, self)
    }

    /// `PropertyDefinition`[Yield, Await] :
    ///   `PropertyName`[?Yield, ?Await] : `AssignmentExpression`[+In, ?Yield, ?Await]
    fn parse_property_definition_assignment(
        &mut self,
        start: u32,
        key: PropertyKey<'a>,
        computed: bool,
    ) -> ArenaBox<'a, ObjectProperty<'a>> {
        self.expect(Kind::Colon);
        let value =
            if self.supports_arkui_dsl() && self.is_in_arkui_dsl_context() && self.at(Kind::LCurly)
            {
                let obj_span = self.cur_start();
                let opening_span = self.cur_token().span();
                self.expect(Kind::LCurly);

                let object = if self.at(Kind::Dot) {
                    self.parse_arkui_object_expression_with_statements(obj_span, opening_span)
                } else {
                    let (properties, comma_span) = self.context_add(Context::In, |p| {
                        p.parse_delimited_list(
                            Kind::RCurly,
                            Kind::Comma,
                            opening_span,
                            Self::parse_object_expression_property,
                        )
                    });
                    if let Some(comma_span) = comma_span {
                        self.state.trailing_commas.insert(obj_span, self.end_span(comma_span));
                    }
                    self.expect(Kind::RCurly);
                    ObjectExpression::boxed(self.end_span(obj_span), properties, self)
                };

                self.parse_type_assertion_if_present(Expression::ObjectExpression(object))
            } else {
                self.parse_assignment_expression_or_higher()
            };
        ObjectProperty::boxed(
            self.end_span(start),
            PropertyKind::Init,
            key,
            value,
            /* method */ false,
            /* shorthand */ false,
            /* computed */ computed,
            self,
        )
    }

    /// `PropertyName`[Yield, Await] :
    ///    `LiteralPropertyName`
    ///    `ComputedPropertyName`[?Yield, ?Await]
    pub(crate) fn parse_property_name(&mut self) -> (PropertyKey<'a>, bool) {
        let mut computed = false;
        let key = match self.cur_kind() {
            Kind::Ident => {
                let ident = self.parse_known_identifier_name();
                PropertyKey::StaticIdentifier(self.alloc(ident))
            }
            Kind::Str => {
                let literal = self.parse_known_literal_string();
                PropertyKey::StringLiteral(self.alloc(literal))
            }
            kind if kind.is_number() => PropertyKey::from(self.parse_literal_expression()),
            // { [foo]() {} }
            Kind::LBrack => {
                computed = true;
                PropertyKey::from(self.parse_computed_property_name())
            }
            Kind::PrivateIdentifier => {
                let private_ident = self.parse_private_identifier();
                self.error(diagnostics::private_identifier_in_property_name(
                    private_ident.name.as_str(),
                    private_ident.span,
                ));
                PropertyKey::PrivateIdentifier(self.alloc(private_ident))
            }
            _ => {
                let ident = self.parse_identifier_name();
                PropertyKey::StaticIdentifier(self.alloc(ident))
            }
        };
        (key, computed)
    }

    /// `ComputedPropertyName`[Yield, Await] : [ `AssignmentExpression`[+In, ?Yield, ?Await] ]
    pub(crate) fn parse_computed_property_name(&mut self) -> Expression<'a> {
        self.bump_any(); // advance `[`

        let expression = self.context_add(Context::In, Self::parse_assignment_expression_or_higher);

        self.expect(Kind::RBrack);
        expression
    }

    /// Parse type assertion (as or satisfies) if present after an expression
    /// Returns the expression wrapped in type assertion if found, otherwise returns the original expression
    fn parse_type_assertion_if_present(&mut self, expr: Expression<'a>) -> Expression<'a> {
        let kind = self.cur_kind();
        if matches!(kind, Kind::As | Kind::Satisfies) {
            if self.cur_token().is_on_new_line() {
                expr
            } else {
                let lhs_span = self.cur_start();
                self.bump_any();
                let type_annotation = self.parse_ts_type();
                let span = self.end_span(lhs_span);
                if kind == Kind::As {
                    if !self.is_ts {
                        self.error(diagnostics::as_in_ts(span));
                    }
                    Expression::new_ts_as_expression(span, expr, type_annotation, self)
                } else {
                    if !self.is_ts {
                        self.error(diagnostics::satisfies_in_ts(span));
                    }
                    Expression::new_ts_satisfies_expression(span, expr, type_annotation, self)
                }
            }
        } else {
            expr
        }
    }

    /// `MethodDefinition`[Yield, Await] :
    ///   get `ClassElementName`[?Yield, ?Await] ( ) { `FunctionBody`[~Yield, ~Await] }
    ///   set `ClassElementName`[?Yield, ?Await] ( `PropertySetParameterList` ) { `FunctionBody`[~Yield, ~Await] }
    fn parse_method_getter_setter(
        &mut self,
        start: u32,
        kind: PropertyKind,
        modifiers: &Modifiers,
    ) -> ArenaBox<'a, ObjectProperty<'a>> {
        let (key, computed) = self.parse_property_name();
        let function = self.parse_method(false, None, FunctionKind::ObjectMethod);
        match kind {
            PropertyKind::Get => self.check_getter(&function),
            PropertyKind::Set => self.check_setter(&function),
            PropertyKind::Init | PropertyKind::EtsEquals => {}
        }
        self.verify_modifiers(
            modifiers,
            ModifierKinds::none(),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );
        ObjectProperty::boxed(
            self.end_span(start),
            kind,
            key,
            Expression::FunctionExpression(function),
            /* method */ false,
            /* shorthand */ false,
            /* computed */ computed,
            self,
        )
    }
}
