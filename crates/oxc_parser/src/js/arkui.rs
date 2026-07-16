//! ArkUI parsing functions
//!
//! This module contains parsing logic for HarmonyOS ArkUI syntax including:
//! - Struct declarations (`struct ComponentName { ... }`)
//! - Annotation declarations (`annotation MyAnnotation { ... }`)
//! - ArkUI component expressions (`Column() { ... }`)

use oxc_allocator::{Box, Vec};
use oxc_ast::ast::*;
use oxc_span::Span;

use crate::{
    Context, ParserConfig as Config, ParserImpl, StatementContext, diagnostics,
    lexer::Kind,
    modifiers::{ModifierKind, ModifierKinds, Modifiers},
};

use super::FunctionKind;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArkUIArgumentContext {
    None,
    SkipFirst,
    All,
}

impl<'a, C: Config> ParserImpl<'a, C> {
    pub(crate) fn at_arkts_annotation_declaration(&mut self) -> bool {
        self.source_type.is_arkui()
            && self.arkts_options.as_ref().is_none_or(|options| options.annotations)
            && self.at(Kind::At)
            && self.lexer.peek_token().kind() == Kind::Interface
    }

    pub(crate) fn is_in_arkui_dsl_context(&self) -> bool {
        self.state.arkui_dsl_depth > 0
    }

    pub(crate) fn is_builtin_arkui_component(&self, expression: &Expression<'a>) -> bool {
        let Expression::Identifier(identifier) = expression else { return false };
        if let Some(options) = &self.arkts_options {
            return options.components.iter().any(|name| name == identifier.name.as_str());
        }
        matches!(
            identifier.name.as_str(),
            "AbilityComponent"
                | "AlphabetIndexer"
                | "Animator"
                | "Badge"
                | "Blank"
                | "Button"
                | "Calendar"
                | "CalendarPicker"
                | "Camera"
                | "Canvas"
                | "Checkbox"
                | "CheckboxGroup"
                | "Circle"
                | "ColorPicker"
                | "ColorPickerDialog"
                | "Column"
                | "ColumnSplit"
                | "Counter"
                | "DataPanel"
                | "DatePicker"
                | "Divider"
                | "Ellipse"
                | "Flex"
                | "FlowItem"
                | "FolderStack"
                | "FormComponent"
                | "FormLink"
                | "Gauge"
                | "GeometryView"
                | "Grid"
                | "GridCol"
                | "GridContainer"
                | "GridItem"
                | "GridRow"
                | "Hyperlink"
                | "Image"
                | "ImageAnimator"
                | "ImageSpan"
                | "Line"
                | "List"
                | "ListItem"
                | "ListItemGroup"
                | "LoadingProgress"
                | "Marquee"
                | "Menu"
                | "MenuItem"
                | "MenuItemGroup"
                | "NavDestination"
                | "NavRouter"
                | "Navigation"
                | "Navigator"
                | "NodeContainer"
                | "Option"
                | "PageTransitionEnter"
                | "PageTransitionExit"
                | "Panel"
                | "Path"
                | "PatternLock"
                | "PluginComponent"
                | "Polygon"
                | "Polyline"
                | "Progress"
                | "QRCode"
                | "Radio"
                | "Rating"
                | "Rect"
                | "RelativeContainer"
                | "Refresh"
                | "RemoteWindow"
                | "RichEditor"
                | "RichText"
                | "Row"
                | "RowSplit"
                | "Scroll"
                | "ScrollBar"
                | "Search"
                | "Section"
                | "Select"
                | "Shape"
                | "Sheet"
                | "SideBarContainer"
                | "Slider"
                | "Span"
                | "Stack"
                | "Stepper"
                | "StepperItem"
                | "Swiper"
                | "SymbolGlyph"
                | "SymbolSpan"
                | "Tabs"
                | "TabContent"
                | "Text"
                | "TextArea"
                | "TextClock"
                | "TextInput"
                | "TextPicker"
                | "TextTimer"
                | "TimePicker"
                | "Toggle"
                | "Video"
                | "WaterFlow"
                | "Web"
                | "XComponent"
        )
    }

    pub(crate) fn in_arkui_dsl_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        self.state.arkui_dsl_depth += 1;
        let result = f(self);
        self.state.arkui_dsl_depth -= 1;
        result
    }

    pub(crate) fn without_arkui_dsl_context<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let depth = self.state.arkui_dsl_depth;
        self.state.arkui_dsl_depth = 0;
        let result = f(self);
        self.state.arkui_dsl_depth = depth;
        result
    }

    pub(crate) fn take_next_arkui_dsl_function(&mut self) -> bool {
        std::mem::take(&mut self.state.arkui_dsl_next_function)
    }

    pub(crate) fn next_function_in_arkui_dsl<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.state.arkui_dsl_next_function;
        self.state.arkui_dsl_next_function = true;
        let result = f(self);
        self.state.arkui_dsl_next_function = previous;
        result
    }

    pub(crate) fn is_arkui_render_method(&self, name: &str) -> bool {
        self.arkts_options.as_ref().map_or_else(
            || matches!(name, "build" | "pageTransition"),
            |options| options.render_methods.iter().any(|method| method == name),
        )
    }

    pub(crate) fn decorators_enable_arkui_dsl(&self, decorators: &[Decorator<'a>]) -> bool {
        decorators.iter().any(|decorator| self.is_arkui_dsl_decorator(&decorator.expression))
    }

    fn is_arkui_dsl_decorator(&self, expression: &Expression<'a>) -> bool {
        match expression {
            Expression::Identifier(ident) => self.arkts_options.as_ref().map_or_else(
                || matches!(ident.name.as_str(), "Builder" | "LocalBuilder" | "Styles"),
                |options| {
                    options.render_decorators.iter().any(|name| name == ident.name.as_str())
                        || options
                            .styles_decorator
                            .as_deref()
                            .is_some_and(|name| name == ident.name.as_str())
                },
            ),
            Expression::CallExpression(call) => {
                let Expression::Identifier(ident) = &call.callee else { return false };
                self.arkts_options.as_ref().map_or_else(
                    || matches!(ident.name.as_str(), "Extend" | "AnimatableExtend"),
                    |options| {
                        options.extend_decorators.iter().any(|name| name == ident.name.as_str())
                    },
                )
            }
            _ => false,
        }
    }

    pub(crate) fn arkui_argument_context(
        &self,
        expression: &Expression<'a>,
    ) -> ArkUIArgumentContext {
        if let Expression::Identifier(identifier) = expression {
            let is_parameter_callback = self.arkts_options.as_ref().map_or_else(
                || matches!(identifier.name.as_str(), "ForEach" | "LazyForEach"),
                |options| {
                    options
                        .parameter_ui_callbacks
                        .iter()
                        .any(|name| name == identifier.name.as_str())
                },
            );
            if is_parameter_callback {
                return ArkUIArgumentContext::SkipFirst;
            }
        }

        let Expression::StaticMemberExpression(member) = expression else {
            return ArkUIArgumentContext::None;
        };
        let attribute = member.property.name.as_str();
        let component = match &member.object {
            Expression::Identifier(identifier) => Some(identifier.name.as_str()),
            Expression::CallExpression(call) => match &call.callee {
                Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                _ => None,
            },
            Expression::ArkUIComponentExpression(component) => match &component.callee {
                Expression::Identifier(identifier) => Some(identifier.name.as_str()),
                _ => None,
            },
            _ => None,
        };
        let Some(component) = component else { return ArkUIArgumentContext::None };
        let is_attribute_callback = self.arkts_options.as_ref().map_or_else(
            || component == "Repeat" && matches!(attribute, "each" | "template"),
            |options| {
                options.attribute_ui_callbacks.iter().any(|callback| {
                    callback.component == component
                        && callback.attributes.iter().any(|name| name == attribute)
                })
            },
        );
        if is_attribute_callback { ArkUIArgumentContext::All } else { ArkUIArgumentContext::None }
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
        let (extends, implements) = self.parse_heritage_clause();
        let mut super_class = None;
        let mut super_type_arguments = None;
        if let Some(mut extends) = extends
            && !extends.is_empty()
        {
            let first_extends = extends.remove(0);
            super_class = Some(first_extends.expression);
            super_type_arguments = first_extends.type_arguments;
            for extend in extends {
                self.error(diagnostics::classes_can_only_extend_single_class(extend.span));
            }
        }
        let body = self.parse_struct_body();

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare, ModifierKind::Abstract]),
            true,
            diagnostics::modifier_cannot_be_used_here,
        );

        let span = self.end_span(start_span);

        let r#abstract = modifiers.contains(ModifierKind::Abstract);
        let declare = modifiers.contains(ModifierKind::Declare);

        self.ast.alloc_struct_statement(
            span,
            decorators,
            id,
            type_parameters,
            super_class,
            super_type_arguments,
            implements.map_or_else(|| self.ast.vec(), |(_, implements)| implements),
            body,
            r#abstract,
            declare,
        )
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
        let at_span = self.cur_token().span();
        self.expect(Kind::At);
        let interface_span = self.cur_token().span();
        self.expect(Kind::Interface);
        if at_span.end != interface_span.start {
            self.error(diagnostics::whitespace_in_annotation_declaration(Span::new(
                at_span.end,
                interface_span.start,
            )));
        }

        if !self.ctx.has_top_level() {
            self.error(diagnostics::annotation_declaration_not_top_level(Span::new(
                start_span,
                interface_span.end,
            )));
        }

        let id = if self.at(Kind::Await) {
            let name = self.parse_identifier_name();
            self.ast.binding_identifier(name.span, name.name)
        } else if self.cur_kind().is_binding_identifier() {
            self.parse_binding_identifier()
        } else {
            self.unexpected::<BindingIdentifier<'a>>()
        };

        let body = self.parse_annotation_body();

        self.verify_modifiers(
            modifiers,
            ModifierKinds::new([ModifierKind::Declare]),
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
                if p.at(Kind::Semicolon) {
                    let span = p.cur_token().span();
                    p.bump_any();
                    p.error(diagnostics::invalid_annotation_member(span));
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
            /* stop_on_start_of_class_static_block */ true,
        );

        if !decorators.is_empty()
            || modifiers.kinds() != ModifierKinds::none()
            || self.cur_kind() != Kind::Ident
        {
            self.error(diagnostics::invalid_annotation_member(Span::new(
                span,
                self.cur_token().span().end,
            )));
        }

        self.verify_modifiers(
            &modifiers,
            ModifierKinds::none(),
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
            /* stop_on_start_of_class_static_block */ true,
        );

        if self.at(Kind::Static) && self.lexer.peek_token().kind() == Kind::LCurly {
            for decorator in decorators {
                self.error(diagnostics::decorators_are_not_valid_here(decorator.span));
            }
            self.verify_modifiers(
                &modifiers,
                ModifierKinds::none(),
                false,
                diagnostics::modifiers_cannot_appear_here,
            );
            let ClassElement::StaticBlock(block) = self.parse_class_static_block(span) else {
                unreachable!();
            };
            return StructElement::StaticBlock(block);
        }

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

        if matches!(self.cur_kind(), Kind::Constructor | Kind::Str)
            && !modifiers.contains(ModifierKind::Static)
            && let Some(name) = self.parse_constructor_name()
        {
            let ClassElement::MethodDefinition(method) =
                self.parse_constructor_declaration(span, r#type, name, &modifiers, decorators)
            else {
                unreachable!();
            };
            return StructElement::MethodDefinition(method);
        }

        if self.is_index_signature() {
            for decorator in decorators {
                self.error(diagnostics::decorators_are_not_valid_here(decorator.span));
            }
            self.verify_modifiers(
                &modifiers,
                ModifierKinds::new([ModifierKind::Readonly, ModifierKind::Static]),
                true,
                diagnostics::cannot_appear_on_an_index_signature,
            );
            return StructElement::TSIndexSignature(
                self.parse_index_signature_declaration(span, &modifiers),
            );
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
        let definite_token_start = self.at(Kind::Bang).then(|| self.start_span());
        let definite = self.eat(Kind::Bang);

        if definite && let Some(optional_span) = optional_span {
            self.error(diagnostics::optional_definite_property(optional_span.expand_right(1)));
        }

        if modifiers.contains(ModifierKind::Accessor) {
            let ClassElement::AccessorProperty(property) = self.parse_class_accessor_property(
                span,
                name,
                computed,
                definite_token_start,
                modifiers,
                decorators,
            ) else {
                unreachable!();
            };
            return StructElement::AccessorProperty(property);
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
                    .is_some_and(|name| self.is_arkui_render_method(name.as_ref())))
                || self.decorators_enable_arkui_dsl(decorators.as_slice()));
        let value = if is_arkui_dsl_method {
            self.next_function_in_arkui_dsl(|p| {
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

    /// Parse an ArkUI component expression after arguments have been parsed
    pub(crate) fn parse_arkui_component_expression_after_args(
        &mut self,
        span: u32,
        callee: Expression<'a>,
        type_arguments: Option<Box<'a, TSTypeParameterInstantiation<'a>>>,
        arguments: Vec<'a, Argument<'a>>,
    ) -> Expression<'a> {
        // Parse children block
        let (children, has_children) = self.parse_arkui_component_children();

        let component_span = self.end_span(span);
        let chain_expressions = self.parse_arkui_component_chain_expressions();
        Expression::ArkUIComponentExpression(self.ast.alloc_ark_ui_component_expression(
            component_span,
            callee,
            type_arguments,
            arguments,
            children,
            has_children,
            chain_expressions,
        ))
    }

    fn parse_arkui_component_chain_expressions(&mut self) -> Vec<'a, CallExpression<'a>> {
        let mut chain_expressions = self.ast.vec();
        while self.at(Kind::Dot) {
            let checkpoint = self.checkpoint();
            self.bump_any();
            if !self.cur_kind().is_identifier_or_keyword() {
                self.rewind(checkpoint);
                break;
            }
            let ident_span = self.start_span();
            let ident = self.parse_identifier_name();
            let type_arguments = if self.is_ts { self.try_parse_type_arguments() } else { None };
            if !self.at(Kind::LParen) {
                self.rewind(checkpoint);
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
                type_arguments,
                call_args,
                false,
            ));
        }
        chain_expressions
    }

    fn parse_arkui_component_children(&mut self) -> (Vec<'a, ArkUIChild<'a>>, bool) {
        if !self.eat(Kind::LCurly) {
            return (self.ast.vec(), false);
        }
        let children = self.in_arkui_dsl_context(|p| {
            let mut children = p.ast.vec();
            while !p.at(Kind::RCurly) && !p.has_fatal_error() {
                children.push(p.parse_arkui_child());
            }
            p.expect(Kind::RCurly);
            children
        });
        (children, true)
    }

    /// Parse an ArkUI child element
    fn parse_arkui_child(&mut self) -> ArkUIChild<'a> {
        let statement = self.parse_statement_list_item(StatementContext::StatementList);
        match statement {
            Statement::ExpressionStatement(statement) => match statement.unbox().expression {
                Expression::ArkUIComponentExpression(component) => ArkUIChild::Component(component),
                expression => ArkUIChild::Expression(self.alloc(expression)),
            },
            statement => ArkUIChild::Statement(self.alloc(statement)),
        }
    }
}
