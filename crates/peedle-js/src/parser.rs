use crate::ast::*;
use crate::error::JsError;
use crate::lexer::{Lexer, Token, TokenKind};
use std::rc::Rc;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    cur: Token,
    last_token_end: usize,
    strict: bool,
    is_module: bool,
    in_async: bool,
    in_generator: bool,
    in_function: bool,
    in_class: bool,
}

#[derive(Clone, Default)]
pub struct EvalValidationCtx {
    pub allow_super_call: bool,
    pub allow_super_prop: bool,
    pub allow_new_target: bool,
    pub allow_private: bool,
    pub forbid_arguments: bool,
    pub declared_privates: std::collections::HashSet<String>,
}

impl<'a> Parser<'a> {
    pub fn parse_script(src: &'a str) -> Result<Program, JsError> {
        Self::parse_inner(src, false, false, EvalValidationCtx::default())
    }

    pub fn parse_module(src: &'a str) -> Result<Program, JsError> {
        Self::parse_inner(src, true, true, EvalValidationCtx::default())
    }

    pub fn parse_script_with_strict(src: &'a str, strict: bool) -> Result<Program, JsError> {
        Self::parse_inner(src, false, strict, EvalValidationCtx::default())
    }

    pub fn parse_eval(
        src: &'a str,
        strict: bool,
        eval_ctx: EvalValidationCtx,
    ) -> Result<Program, JsError> {
        Self::parse_inner(src, false, strict, eval_ctx)
    }

    fn parse_inner(
        src: &'a str,
        is_module: bool,
        initial_strict: bool,
        eval_ctx: EvalValidationCtx,
    ) -> Result<Program, JsError> {
        let mut lexer = Lexer::new_with_mode(src, is_module);
        let cur = lexer.next_token(true)?;
        let mut parser = Parser {
            lexer,
            cur,
            last_token_end: 0,
            strict: initial_strict || is_module,
            is_module,
            in_async: is_module,
            in_generator: false,
            in_function: false,
            in_class: false,
        };
        let mut body = Vec::new();
        let mut checking_directives = true;
        let mut prologue_strict_err: Option<&'static str> = None;
        while parser.cur.kind != TokenKind::Eof {
            if checking_directives {
                if let TokenKind::String(ref s, has_escape) = parser.cur.kind {
                    let cur_strict_err = parser.cur.strict_error;
                    if s == "use strict" && !has_escape {
                        let stmt = parser.parse_module_item()?;
                        if matches!(stmt, Stmt::Expr(Expr::String(ref st)) if st == "use strict") {
                            if let Some(err) = prologue_strict_err {
                                return Err(JsError::syntax(err));
                            }
                            parser.strict = true;
                        } else {
                            checking_directives = false;
                        }
                        body.push(stmt);
                        continue;
                    }
                    if prologue_strict_err.is_none() {
                        prologue_strict_err = cur_strict_err;
                    }
                }
            }
            if parser.strict {
                if let Some(err) = parser.cur.strict_error {
                    return Err(JsError::syntax(err));
                }
            }
            let stmt = parser.parse_module_item()?;
            if checking_directives && !matches!(stmt, Stmt::Expr(Expr::String(_))) {
                checking_directives = false;
            }
            body.push(stmt);
        }
        if !is_module {
            for stmt in &body {
                if matches!(
                    stmt,
                    Stmt::VarDecl {
                        kind: VarKind::Using | VarKind::AwaitUsing,
                        ..
                    }
                ) {
                    return Err(JsError::syntax(
                        "using declaration is not allowed at the top level of a script",
                    ));
                }
            }
        }
        let program = Program {
            body,
            is_module,
            strict: parser.strict,
        };
        if is_module {
            validate_module_early_errors(&program)?;
        }
        validate_program_early_errors(&program, eval_ctx)?;
        Ok(program)
    }

    fn advance(&mut self, allow_regexp: bool) -> Result<Token, JsError> {
        if self.strict {
            if let Some(err) = self.cur.strict_error {
                return Err(JsError::syntax(err));
            }
        }
        let prev = std::mem::replace(&mut self.cur, self.lexer.next_token(allow_regexp)?);
        self.last_token_end = prev.end;
        Ok(prev)
    }

    fn expect(&mut self, kind: &TokenKind, allow_regexp: bool) -> Result<Token, JsError> {
        if &self.cur.kind == kind {
            self.advance(allow_regexp)
        } else {
            Err(JsError::syntax(format!(
                "Expected {:?}, found {:?} at {}:{}",
                kind, self.cur.kind, self.cur.line, self.cur.col
            )))
        }
    }

    fn eat(&mut self, kind: &TokenKind, allow_regexp: bool) -> Result<bool, JsError> {
        if &self.cur.kind == kind {
            self.advance(allow_regexp)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn eat_semi(&mut self) -> Result<(), JsError> {
        if self.cur.kind == TokenKind::Semi {
            self.advance(true)?;
            return Ok(());
        }
        if self.cur.preceded_by_newline
            || matches!(self.cur.kind, TokenKind::RBrace | TokenKind::Eof)
        {
            return Ok(());
        }
        Err(JsError::syntax(format!(
            "Expected semicolon, found {:?} at {}:{}",
            self.cur.kind, self.cur.line, self.cur.col
        )))
    }

    fn is_ident_word(&self, word: &str) -> bool {
        matches!(&self.cur.kind, TokenKind::Ident(s, false) if s == word)
    }

    fn peek_next(&self, allow_regexp: bool) -> Result<Token, JsError> {
        let mut cloned = self.lexer.clone();
        cloned.next_token(allow_regexp)
    }

    fn parse_module_item(&mut self) -> Result<Stmt, JsError> {
        if self.is_module {
            match self.cur.kind {
                TokenKind::KwImport => {
                    let next = self.peek_next(true)?;
                    if !matches!(next.kind, TokenKind::LParen | TokenKind::Dot) {
                        return self.parse_import();
                    }
                }
                TokenKind::KwExport => return self.parse_export(),
                _ => {}
            }
        }
        self.parse_stmt()
    }

    fn parse_import_attributes(&mut self) -> Result<Vec<(String, String)>, JsError> {
        if self.cur.preceded_by_newline || self.cur.kind != TokenKind::KwWith {
            return Ok(Vec::new());
        }
        self.advance(true)?;
        self.expect(&TokenKind::LBrace, true)?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            let key = match self.cur.kind.clone() {
                TokenKind::String(s, _) => {
                    self.advance(true)?;
                    s
                }
                _ => self.parse_ident_name()?,
            };
            self.expect(&TokenKind::Colon, true)?;
            let val = match self.cur.kind.clone() {
                TokenKind::String(s, _) => {
                    self.advance(true)?;
                    s
                }
                _ => return Err(JsError::syntax("Import attribute value must be a string literal")),
            };
            if attrs.iter().any(|(k, _)| k == &key) {
                return Err(JsError::syntax(format!(
                    "Duplicate import attribute key '{}'",
                    key
                )));
            }
            attrs.push((key, val));
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, false)?;
        Ok(attrs)
    }

    fn parse_import(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let mut specifiers = Vec::new();
        let mut phase = ImportPhase::Evaluation;
        if let TokenKind::String(src, _) = self.cur.kind.clone() {
            self.advance(false)?;
            let attributes = self.parse_import_attributes()?;
            self.eat_semi()?;
            return Ok(Stmt::Import(ImportDecl {
                specifiers,
                source: src,
                phase,
                attributes,
            }));
        }
        if matches!(&self.cur.kind, TokenKind::Ident(s, false) if s == "defer") {
            let next = self.peek_next(true)?;
            if !next.preceded_by_newline && next.kind == TokenKind::Star {
                self.advance(true)?;
                phase = ImportPhase::Defer;
                self.parse_import_clause_tail(&mut specifiers)?;
            } else {
                let local = self.parse_binding_ident()?;
                specifiers.push(ImportSpecifier::Default(local));
                if self.eat(&TokenKind::Comma, true)? {
                    self.parse_import_clause_tail(&mut specifiers)?;
                }
            }
        } else if matches!(&self.cur.kind, TokenKind::Ident(s, false) if s == "source") {
            let next = self.peek_next(true)?;
            if !next.preceded_by_newline
                && !matches!(next.kind, TokenKind::Comma)
                && !matches!(&next.kind, TokenKind::Ident(w, false) if w == "from")
            {
                self.advance(true)?;
                phase = ImportPhase::Source;
                let local = self.parse_binding_ident()?;
                specifiers.push(ImportSpecifier::Default(local));
            } else {
                let local = self.parse_binding_ident()?;
                specifiers.push(ImportSpecifier::Default(local));
                if self.eat(&TokenKind::Comma, true)? {
                    self.parse_import_clause_tail(&mut specifiers)?;
                }
            }
        } else if !matches!(self.cur.kind, TokenKind::Star | TokenKind::LBrace) {
            let local = self.parse_binding_ident()?;
            specifiers.push(ImportSpecifier::Default(local));
            if self.eat(&TokenKind::Comma, true)? {
                self.parse_import_clause_tail(&mut specifiers)?;
            }
        } else {
            self.parse_import_clause_tail(&mut specifiers)?;
        }
        if !self.is_ident_word("from") {
            return Err(JsError::syntax("Expected 'from' in import declaration"));
        }
        self.advance(true)?;
        let source = match self.cur.kind.clone() {
            TokenKind::String(s, _) => {
                self.advance(false)?;
                s
            }
            _ => return Err(JsError::syntax("Expected module specifier string")),
        };
        let attributes = self.parse_import_attributes()?;
        self.eat_semi()?;
        Ok(Stmt::Import(ImportDecl {
            specifiers,
            source,
            phase,
            attributes,
        }))
    }

    fn parse_import_clause_tail(
        &mut self,
        specifiers: &mut Vec<ImportSpecifier>,
    ) -> Result<(), JsError> {
        if self.eat(&TokenKind::Star, true)? {
            if !self.is_ident_word("as") {
                return Err(JsError::syntax("Expected 'as' after '*' in import"));
            }
            self.advance(true)?;
            let local = self.parse_binding_ident()?;
            specifiers.push(ImportSpecifier::Namespace(local));
        } else if self.eat(&TokenKind::LBrace, true)? {
            while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
                let (imported, imported_is_str) = if let TokenKind::String(s, _) = self.cur.kind.clone() {
                    if self.cur.has_lone_surrogate {
                        return Err(JsError::syntax(
                            "Module import name must be well-formed Unicode",
                        ));
                    }
                    self.advance(true)?;
                    (s, true)
                } else {
                    (self.parse_ident_name()?, false)
                };
                let local = if self.is_ident_word("as") {
                    self.advance(true)?;
                    self.parse_binding_ident()?
                } else {
                    if imported_is_str
                        || is_reserved_word(&imported, true)
                        || imported == "await"
                        || imported == "eval"
                        || imported == "arguments"
                    {
                        return Err(JsError::syntax("Invalid imported binding name"));
                    }
                    imported.clone()
                };
                specifiers.push(ImportSpecifier::Named { imported, local });
                if !self.eat(&TokenKind::Comma, true)? {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace, true)?;
        }
        Ok(())
    }

    fn parse_module_export_name(&mut self) -> Result<String, JsError> {
        if let TokenKind::String(s, _) = self.cur.kind.clone() {
            if self.cur.has_lone_surrogate {
                return Err(JsError::syntax(
                    "Module export name must be well-formed Unicode",
                ));
            }
            self.advance(true)?;
            Ok(s)
        } else {
            self.parse_ident_name()
        }
    }

    fn parse_export(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        if self.eat(&TokenKind::KwDefault, true)? {
            if self.cur.kind == TokenKind::KwFunction
                || (self.is_ident_word("async")
                    && !self.peek_next(true)?.preceded_by_newline
                    && self.peek_next(true)?.kind == TokenKind::KwFunction)
            {
                let func = self.parse_function(false, true)?;
                return Ok(Stmt::Export(ExportDecl::DefaultFunc(func)));
            }
            if self.cur.kind == TokenKind::KwClass {
                let cls = self.parse_class(false)?;
                return Ok(Stmt::Export(ExportDecl::DefaultClass(cls)));
            }
            let expr = self.parse_assign_expr(true)?;
            self.eat_semi()?;
            return Ok(Stmt::Export(ExportDecl::DefaultExpr(expr)));
        }
        if self.eat(&TokenKind::Star, true)? {
            let exported = if self.is_ident_word("as") {
                self.advance(true)?;
                Some(self.parse_module_export_name()?)
            } else {
                None
            };
            if !self.is_ident_word("from") {
                return Err(JsError::syntax("Expected 'from' in export *"));
            }
            self.advance(true)?;
            let source = match self.cur.kind.clone() {
                TokenKind::String(s, _) => {
                    self.advance(false)?;
                    s
                }
                _ => return Err(JsError::syntax("Expected string after 'from'")),
            };
            let attributes = self.parse_import_attributes()?;
            self.eat_semi()?;
            return Ok(Stmt::Export(ExportDecl::All {
                exported,
                source,
                attributes,
            }));
        }
        if self.eat(&TokenKind::LBrace, true)? {
            let mut specifiers = Vec::new();
            while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
                let (local, local_is_string) = if let TokenKind::String(s, _) = self.cur.kind.clone() {
                    if self.cur.has_lone_surrogate {
                        return Err(JsError::syntax(
                            "Module export name must be well-formed Unicode",
                        ));
                    }
                    self.advance(true)?;
                    (s, true)
                } else {
                    (self.parse_ident_name()?, false)
                };
                let exported = if self.is_ident_word("as") {
                    self.advance(true)?;
                    self.parse_module_export_name()?
                } else {
                    local.clone()
                };
                specifiers.push(ExportSpecifier {
                    local,
                    exported,
                    local_is_string,
                });
                if !self.eat(&TokenKind::Comma, true)? {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace, true)?;
            let (source, attributes) = if self.is_ident_word("from") {
                self.advance(true)?;
                let s = match self.cur.kind.clone() {
                    TokenKind::String(s, _) => {
                        self.advance(false)?;
                        s
                    }
                    _ => return Err(JsError::syntax("Expected string after 'from'")),
                };
                let attrs = self.parse_import_attributes()?;
                (Some(s), attrs)
            } else {
                for sp in &specifiers {
                    if sp.local_is_string {
                        return Err(JsError::syntax(
                            "String literal export name requires 'from' clause",
                        ));
                    }
                }
                (None, Vec::new())
            };
            self.eat_semi()?;
            return Ok(Stmt::Export(ExportDecl::Named {
                specifiers,
                source,
                attributes,
            }));
        }
        let decl = match &self.cur.kind {
            TokenKind::KwVar => self.parse_var_decl(VarKind::Var, true)?,
            TokenKind::KwConst => self.parse_var_decl(VarKind::Const, true)?,
            TokenKind::Ident(s, false) if s == "let" => self.parse_var_decl(VarKind::Let, true)?,
            TokenKind::KwFunction => Stmt::FuncDecl(self.parse_function(true, true)?),
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(false)?;
                if !next.preceded_by_newline && next.kind == TokenKind::KwFunction {
                    Stmt::FuncDecl(self.parse_function(true, true)?)
                } else {
                    return Err(JsError::syntax("Invalid export declaration"));
                }
            }
            TokenKind::KwClass => Stmt::ClassDecl(self.parse_class(true)?),
            _ => return Err(JsError::syntax("Invalid export declaration")),
        };
        Ok(Stmt::Export(ExportDecl::Decl(Box::new(decl))))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, JsError> {
        self.parse_stmt_inner(true)
    }

    fn parse_stmt_inner(&mut self, allow_lexical: bool) -> Result<Stmt, JsError> {
        match &self.cur.kind {
            TokenKind::LBrace => self.parse_block(),
            TokenKind::Semi => {
                self.advance(true)?;
                Ok(Stmt::Empty)
            }
            TokenKind::KwVar => self.parse_var_decl(VarKind::Var, true),
            TokenKind::KwConst => self.parse_var_decl(VarKind::Const, true),
            TokenKind::Ident(s, false) if s == "let" => {
                if allow_lexical && self.is_let_decl_start()? {
                    self.parse_var_decl(VarKind::Let, true)
                } else {
                    self.parse_expr_or_label_stmt()
                }
            }
            TokenKind::Ident(s, false) if s == "using" => {
                if allow_lexical && self.is_using_decl_start()? {
                    self.parse_var_decl(VarKind::Using, true)
                } else {
                    self.parse_expr_or_label_stmt()
                }
            }
            TokenKind::KwAwait if self.in_async => {
                if allow_lexical && self.is_await_using_decl_start()? {
                    self.advance(true)?;
                    self.parse_var_decl(VarKind::AwaitUsing, true)
                } else {
                    self.parse_expr_or_label_stmt()
                }
            }
            TokenKind::KwFunction => {
                let func = self.parse_function(true, true)?;
                Ok(Stmt::FuncDecl(func))
            }
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(false)?;
                if !next.preceded_by_newline && next.kind == TokenKind::KwFunction {
                    let func = self.parse_function(true, true)?;
                    Ok(Stmt::FuncDecl(func))
                } else {
                    self.parse_expr_or_label_stmt()
                }
            }
            TokenKind::KwClass => {
                let cls = self.parse_class(true)?;
                Ok(Stmt::ClassDecl(cls))
            }
            TokenKind::KwIf => self.parse_if(),
            TokenKind::KwWhile => self.parse_while(),
            TokenKind::KwDo => self.parse_do_while(),
            TokenKind::KwFor => self.parse_for(),
            TokenKind::KwSwitch => self.parse_switch(),
            TokenKind::KwTry => self.parse_try(),
            TokenKind::KwThrow => self.parse_throw(),
            TokenKind::KwReturn => self.parse_return(),
            TokenKind::KwBreak => self.parse_break(),
            TokenKind::KwContinue => self.parse_continue(),
            TokenKind::KwWith => self.parse_with(),
            TokenKind::KwDebugger => {
                self.advance(false)?;
                self.eat_semi()?;
                Ok(Stmt::Debugger)
            }
            _ => self.parse_expr_or_label_stmt(),
        }
    }

    fn parse_sub_stmt(&mut self, allow_annex_b_func: bool) -> Result<Stmt, JsError> {
        match &self.cur.kind {
            TokenKind::KwConst | TokenKind::KwClass => {
                return Err(JsError::syntax(
                    "Lexical declaration cannot appear in a single-statement context",
                ));
            }
            TokenKind::Ident(s, false) if s == "let" => {
                let next = self.peek_next(false)?;
                let forbidden = next.kind == TokenKind::LBracket
                    || (!next.preceded_by_newline
                        && matches!(
                            next.kind,
                            TokenKind::Ident(_, _)
                                | TokenKind::LBrace
                                | TokenKind::KwYield
                                | TokenKind::KwAwait
                        ));
                if forbidden {
                    return Err(JsError::syntax(
                        "Lexical declaration cannot appear in a single-statement context",
                    ));
                }
            }
            TokenKind::Ident(s, false) if s == "using" && self.is_using_decl_start()? => {
                return Err(JsError::syntax(
                    "Lexical declaration cannot appear in a single-statement context",
                ));
            }
            TokenKind::KwAwait if self.in_async && self.is_await_using_decl_start()? => {
                return Err(JsError::syntax(
                    "Lexical declaration cannot appear in a single-statement context",
                ));
            }
            TokenKind::KwFunction => {
                let next = self.peek_next(true)?;
                let is_gen = next.kind == TokenKind::Star;
                if !allow_annex_b_func || is_gen {
                    return Err(JsError::syntax(
                        "Function declaration cannot appear in a single-statement context",
                    ));
                }
            }
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(false)?;
                if !next.preceded_by_newline && next.kind == TokenKind::KwFunction {
                    return Err(JsError::syntax(
                        "Async function declaration cannot appear in a single-statement context",
                    ));
                }
            }
            _ => {}
        }
        let stmt = self.parse_stmt_inner(false)?;
        if is_labeled_func_decl(&stmt) {
            return Err(JsError::syntax(
                "Labelled function declaration cannot appear in a single-statement context",
            ));
        }
        Ok(stmt)
    }

    fn is_using_decl_start(&self) -> Result<bool, JsError> {
        let next = self.peek_next(false)?;
        if next.preceded_by_newline {
            return Ok(false);
        }
        Ok(matches!(
            next.kind,
            TokenKind::Ident(_, _) | TokenKind::KwYield | TokenKind::KwAwait
        ))
    }

    fn is_for_using_decl_start(&self) -> Result<bool, JsError> {
        let mut lex = self.lexer.clone();
        let t1 = lex.next_token(false)?;
        if t1.preceded_by_newline {
            return Ok(false);
        }
        if matches!(&t1.kind, TokenKind::Ident(s, false) if s == "of") {
            let t2 = lex.next_token(true)?;
            return Ok(matches!(t2.kind, TokenKind::Eq));
        }
        Ok(matches!(
            t1.kind,
            TokenKind::Ident(_, _) | TokenKind::KwYield | TokenKind::KwAwait
        ))
    }

    fn is_await_using_decl_start(&self) -> Result<bool, JsError> {
        let mut lex = self.lexer.clone();
        let t1 = lex.next_token(true)?;
        if t1.preceded_by_newline {
            return Ok(false);
        }
        if !matches!(&t1.kind, TokenKind::Ident(s, false) if s == "using") {
            return Ok(false);
        }
        let t2 = lex.next_token(true)?;
        if t2.preceded_by_newline {
            return Ok(false);
        }
        Ok(matches!(
            t2.kind,
            TokenKind::Ident(_, _) | TokenKind::KwYield | TokenKind::KwAwait
        ))
    }

    fn is_let_decl_start(&self) -> Result<bool, JsError> {
        let next = self.peek_next(false)?;
        if next.preceded_by_newline {
            if self.in_async && matches!(next.kind, TokenKind::KwAwait) {
                return Ok(true);
            }
            if self.in_generator && matches!(next.kind, TokenKind::KwYield) {
                return Ok(true);
            }
            if matches!(&next.kind, TokenKind::Ident(s, false) if s == "let") {
                return Ok(true);
            }
            return Ok(matches!(
                next.kind,
                TokenKind::LBrace | TokenKind::LBracket
            ));
        }
        Ok(matches!(
            next.kind,
            TokenKind::Ident(_, _)
                | TokenKind::LBrace
                | TokenKind::LBracket
                | TokenKind::KwYield
                | TokenKind::KwAwait
        ))
    }

    fn parse_block(&mut self) -> Result<Stmt, JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut stmts = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, true)?;
        Ok(Stmt::Block(stmts))
    }

    fn parse_var_decl(&mut self, kind: VarKind, allow_in: bool) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let mut decls = Vec::new();
        loop {
            let pattern = self.parse_pattern()?;
            if matches!(kind, VarKind::Using | VarKind::AwaitUsing)
                && !matches!(pattern, Pattern::Ident(_))
            {
                return Err(JsError::syntax(
                    "using declaration requires a binding identifier",
                ));
            }
            let init = if self.eat(&TokenKind::Eq, true)? {
                Some(self.parse_assign_expr(allow_in)?)
            } else {
                None
            };
            if init.is_none()
                && (matches!(kind, VarKind::Const | VarKind::Using | VarKind::AwaitUsing)
                    || !matches!(pattern, Pattern::Ident(_)))
            {
                return Err(JsError::syntax(
                    "Missing initializer in const or destructuring declaration",
                ));
            }
            decls.push(VarDeclarator { pattern, init });
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        if allow_in {
            self.eat_semi()?;
        }
        Ok(Stmt::VarDecl { kind, decls })
    }

    fn parse_if(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        self.expect(&TokenKind::LParen, true)?;
        let test = self.parse_expr(true)?;
        self.expect(&TokenKind::RParen, true)?;
        let allow_annex = !self.strict;
        let consequent = Box::new(self.parse_sub_stmt(allow_annex)?);
        let alternate = if self.eat(&TokenKind::KwElse, true)? {
            Some(Box::new(self.parse_sub_stmt(allow_annex)?))
        } else {
            None
        };
        Ok(Stmt::If {
            test,
            consequent,
            alternate,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        self.expect(&TokenKind::LParen, true)?;
        let test = self.parse_expr(true)?;
        self.expect(&TokenKind::RParen, true)?;
        let body = Box::new(self.parse_sub_stmt(false)?);
        Ok(Stmt::While { test, body })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let body = Box::new(self.parse_sub_stmt(false)?);
        self.expect(&TokenKind::KwWhile, true)?;
        self.expect(&TokenKind::LParen, true)?;
        let test = self.parse_expr(true)?;
        self.expect(&TokenKind::RParen, false)?;
        let _ = self.eat(&TokenKind::Semi, true);
        Ok(Stmt::DoWhile { body, test })
    }

    fn parse_for(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let is_await = self.eat(&TokenKind::KwAwait, true)?;
        self.expect(&TokenKind::LParen, true)?;

        if self.cur.kind == TokenKind::Semi {
            if is_await {
                return Err(JsError::syntax("for await requires of"));
            }
            self.advance(true)?;
            return self.parse_for_rest(None);
        }

        let var_kind = match &self.cur.kind {
            TokenKind::KwVar => Some(VarKind::Var),
            TokenKind::KwConst => Some(VarKind::Const),
            TokenKind::Ident(s, false) if s == "let" && self.is_let_decl_start()? => {
                Some(VarKind::Let)
            }
            TokenKind::Ident(s, false) if s == "using" && self.is_for_using_decl_start()? => {
                Some(VarKind::Using)
            }
            TokenKind::KwAwait if self.in_async && self.is_await_using_decl_start()? => {
                self.advance(true)?;
                Some(VarKind::AwaitUsing)
            }
            _ => None,
        };

        if let Some(kind) = var_kind {
            self.advance(true)?;
            let first_pat = self.parse_pattern()?;
            let is_using = matches!(kind, VarKind::Using | VarKind::AwaitUsing);
            if is_using && !matches!(first_pat, Pattern::Ident(_)) {
                return Err(JsError::syntax(
                    "using declaration requires a binding identifier",
                ));
            }
            if self.cur.kind == TokenKind::KwIn {
                if is_await {
                    return Err(JsError::syntax("for await cannot be used with in"));
                }
                if is_using {
                    return Err(JsError::syntax("using declaration not allowed in for-in"));
                }
                self.advance(true)?;
                let right = self.parse_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_sub_stmt(false)?);
                return Ok(Stmt::ForIn {
                    left: ForInOfLeft::VarDecl {
                        kind,
                        pattern: first_pat,
                    },
                    right,
                    body,
                });
            }
            if self.is_ident_word("of") {
                if kind == VarKind::Using && matches!(&first_pat, Pattern::Ident(n) if n == "of") {
                    return Err(JsError::syntax("using of is not allowed in for-of"));
                }
                self.advance(true)?;
                let right = self.parse_assign_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_sub_stmt(false)?);
                return Ok(Stmt::ForOf {
                    left: ForInOfLeft::VarDecl {
                        kind,
                        pattern: first_pat,
                    },
                    right,
                    body,
                    is_await,
                });
            }
            let first_init = if self.eat(&TokenKind::Eq, true)? {
                Some(self.parse_assign_expr(false)?)
            } else {
                None
            };
            if is_using && first_init.is_none() {
                return Err(JsError::syntax("Missing initializer in using declaration"));
            }
            let mut decls = vec![VarDeclarator {
                pattern: first_pat,
                init: first_init,
            }];
            while self.eat(&TokenKind::Comma, true)? {
                let pat = self.parse_pattern()?;
                if is_using && !matches!(pat, Pattern::Ident(_)) {
                    return Err(JsError::syntax(
                        "using declaration requires a binding identifier",
                    ));
                }
                let init = if self.eat(&TokenKind::Eq, true)? {
                    Some(self.parse_assign_expr(false)?)
                } else {
                    None
                };
                if is_using && init.is_none() {
                    return Err(JsError::syntax("Missing initializer in using declaration"));
                }
                decls.push(VarDeclarator { pattern: pat, init });
            }
            if self.cur.kind == TokenKind::KwIn && decls.len() == 1 {
                if kind != VarKind::Var
                    || self.strict
                    || !matches!(decls[0].pattern, Pattern::Ident(_))
                {
                    return Err(JsError::syntax(
                        "Invalid initializer in for-in loop declaration",
                    ));
                }
                self.advance(true)?;
                let right = self.parse_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_sub_stmt(false)?);
                let decl = decls.pop().unwrap();
                return Ok(Stmt::ForIn {
                    left: ForInOfLeft::VarDecl {
                        kind,
                        pattern: decl.pattern,
                    },
                    right,
                    body,
                });
            }
            self.expect(&TokenKind::Semi, true)?;
            return self.parse_for_rest(Some(ForInit::VarDecl { kind, decls }));
        }

        let expr = self.parse_expr(false)?;
        if self.cur.kind == TokenKind::KwIn {
            if is_await {
                return Err(JsError::syntax("for await cannot be used with in"));
            }
            self.advance(true)?;
            let left_pat = expr_to_pattern(expr)?;
            let right = self.parse_expr(true)?;
            self.expect(&TokenKind::RParen, true)?;
            let body = Box::new(self.parse_sub_stmt(false)?);
            return Ok(Stmt::ForIn {
                left: ForInOfLeft::Pattern(left_pat),
                right,
                body,
            });
        }
        if self.is_ident_word("of") {
            self.advance(true)?;
            let left_pat = expr_to_pattern(expr)?;
            let right = self.parse_assign_expr(true)?;
            self.expect(&TokenKind::RParen, true)?;
            let body = Box::new(self.parse_sub_stmt(false)?);
            return Ok(Stmt::ForOf {
                left: ForInOfLeft::Pattern(left_pat),
                right,
                body,
                is_await,
            });
        }
        if is_await {
            return Err(JsError::syntax("for await requires 'of'"));
        }
        self.expect(&TokenKind::Semi, true)?;
        self.parse_for_rest(Some(ForInit::Expr(expr)))
    }

    fn parse_for_rest(&mut self, init: Option<ForInit>) -> Result<Stmt, JsError> {
        let test = if self.cur.kind == TokenKind::Semi {
            None
        } else {
            Some(self.parse_expr(true)?)
        };
        self.expect(&TokenKind::Semi, true)?;
        let update = if self.cur.kind == TokenKind::RParen {
            None
        } else {
            Some(self.parse_expr(true)?)
        };
        self.expect(&TokenKind::RParen, true)?;
        let body = Box::new(self.parse_sub_stmt(false)?);
        Ok(Stmt::For {
            init,
            test,
            update,
            body,
        })
    }

    fn parse_switch(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        self.expect(&TokenKind::LParen, true)?;
        let discriminant = self.parse_expr(true)?;
        self.expect(&TokenKind::RParen, true)?;
        self.expect(&TokenKind::LBrace, true)?;
        let mut cases = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            let test = if self.eat(&TokenKind::KwCase, true)? {
                let expr = self.parse_expr(true)?;
                self.expect(&TokenKind::Colon, true)?;
                Some(expr)
            } else if self.eat(&TokenKind::KwDefault, true)? {
                self.expect(&TokenKind::Colon, true)?;
                None
            } else {
                return Err(JsError::syntax("Expected 'case' or 'default' in switch"));
            };
            let mut consequent = Vec::new();
            while !matches!(
                self.cur.kind,
                TokenKind::KwCase | TokenKind::KwDefault | TokenKind::RBrace | TokenKind::Eof
            ) {
                let s = self.parse_stmt()?;
                if matches!(
                    s,
                    Stmt::VarDecl {
                        kind: VarKind::Using | VarKind::AwaitUsing,
                        ..
                    }
                ) {
                    return Err(JsError::syntax(
                        "using declaration is not allowed directly in a switch clause",
                    ));
                }
                consequent.push(s);
            }
            cases.push(SwitchCase { test, consequent });
        }
        self.expect(&TokenKind::RBrace, true)?;
        Ok(Stmt::Switch {
            discriminant,
            cases,
        })
    }

    fn parse_try(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let block = self.parse_block_stmts()?;
        let handler = if self.eat(&TokenKind::KwCatch, true)? {
            let param = if self.eat(&TokenKind::LParen, true)? {
                let pat = self.parse_pattern()?;
                self.expect(&TokenKind::RParen, true)?;
                Some(pat)
            } else {
                None
            };
            let body = self.parse_block_stmts()?;
            Some(CatchClause { param, body })
        } else {
            None
        };
        let finalizer = if self.eat(&TokenKind::KwFinally, true)? {
            Some(self.parse_block_stmts()?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return Err(JsError::syntax("try statement requires catch or finally"));
        }
        Ok(Stmt::Try {
            block,
            handler,
            finalizer,
        })
    }

    fn parse_block_stmts(&mut self) -> Result<Vec<Stmt>, JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut stmts = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace, true)?;
        Ok(stmts)
    }

    fn parse_throw(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        if self.cur.preceded_by_newline {
            return Err(JsError::syntax("Illegal newline after throw"));
        }
        let expr = self.parse_expr(true)?;
        self.eat_semi()?;
        Ok(Stmt::Throw(expr))
    }

    fn parse_return(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        if self.cur.preceded_by_newline
            || matches!(
                self.cur.kind,
                TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
            )
        {
            let _ = self.eat(&TokenKind::Semi, true);
            return Ok(Stmt::Return(None));
        }
        let expr = self.parse_expr(true)?;
        self.eat_semi()?;
        Ok(Stmt::Return(Some(expr)))
    }

    fn parse_break(&mut self) -> Result<Stmt, JsError> {
        self.advance(false)?;
        if self.cur.preceded_by_newline
            || matches!(
                self.cur.kind,
                TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
            )
        {
            let _ = self.eat(&TokenKind::Semi, true);
            return Ok(Stmt::Break(None));
        }
        let label = self.parse_binding_ident()?;
        self.eat_semi()?;
        Ok(Stmt::Break(Some(label)))
    }

    fn parse_continue(&mut self) -> Result<Stmt, JsError> {
        self.advance(false)?;
        if self.cur.preceded_by_newline
            || matches!(
                self.cur.kind,
                TokenKind::Semi | TokenKind::RBrace | TokenKind::Eof
            )
        {
            let _ = self.eat(&TokenKind::Semi, true);
            return Ok(Stmt::Continue(None));
        }
        let label = self.parse_binding_ident()?;
        self.eat_semi()?;
        Ok(Stmt::Continue(Some(label)))
    }

    fn parse_with(&mut self) -> Result<Stmt, JsError> {
        if self.strict {
            return Err(JsError::syntax("with statement is not allowed in strict mode"));
        }
        self.advance(true)?;
        self.expect(&TokenKind::LParen, true)?;
        let object = self.parse_expr(true)?;
        self.expect(&TokenKind::RParen, true)?;
        let body = Box::new(self.parse_sub_stmt(false)?);
        Ok(Stmt::With { object, body })
    }

    fn parse_expr_or_label_stmt(&mut self) -> Result<Stmt, JsError> {
        if let Some(label) = self.peek_label_ident()? {
            if (label == "await" && self.in_async)
                || (label == "yield" && (self.in_generator || self.strict))
                || is_reserved_word(&label, self.strict)
            {
                return Err(JsError::syntax(format!(
                    "Invalid label identifier '{}'",
                    label
                )));
            }
            self.advance(true)?;
            self.advance(true)?;
            let allow_annex = !self.strict;
            let body = Box::new(self.parse_sub_stmt(allow_annex)?);
            return Ok(Stmt::Labeled { label, body });
        }
        let expr = self.parse_expr(true)?;
        self.eat_semi()?;
        Ok(Stmt::Expr(expr))
    }

    fn peek_label_ident(&self) -> Result<Option<String>, JsError> {
        let name = match &self.cur.kind {
            TokenKind::Ident(s, _) => s.clone(),
            TokenKind::KwYield if !self.in_generator && !self.strict => "yield".to_string(),
            TokenKind::KwAwait if !self.in_async => "await".to_string(),
            _ => return Ok(None),
        };
        let next = self.peek_next(false)?;
        if next.kind == TokenKind::Colon {
            Ok(Some(name))
        } else {
            Ok(None)
        }
    }

    fn parse_function(&mut self, require_name: bool, is_decl: bool) -> Result<Rc<FuncNode>, JsError> {
        let func_start = self.cur.start;
        let is_async = if self.is_ident_word("async") {
            self.advance(true)?;
            true
        } else {
            false
        };
        self.expect(&TokenKind::KwFunction, true)?;
        let is_generator = self.eat(&TokenKind::Star, true)?;
        let name = if self.cur.kind != TokenKind::LParen {
            Some(self.parse_binding_ident()?)
        } else if require_name {
            return Err(JsError::syntax("Function declaration requires a name"));
        } else {
            None
        };
        let prev_async = self.in_async;
        let prev_gen = self.in_generator;
        let prev_fn = self.in_function;
        let prev_strict = self.strict;
        self.in_async = is_async;
        self.in_generator = is_generator;
        self.in_function = true;

        self.expect(&TokenKind::LParen, true)?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::RParen, true)?;
        let (body, strict, has_use_strict) = self.parse_func_body_block_info(is_decl)?;
        validate_func_params(&params, strict, has_use_strict, false)?;
        let func_end = self.last_token_end.min(self.lexer.source().len());

        self.in_async = prev_async;
        self.in_generator = prev_gen;
        self.in_function = prev_fn;
        self.strict = prev_strict;

        Ok(Rc::new(FuncNode {
            name,
            params,
            body: FuncBody::Block(body),
            is_async,
            is_generator,
            is_arrow: false,
            is_decl,
            strict,
            source: self.lexer.source()[func_start..func_end].trim().to_string(),
        }))
    }

    fn parse_func_body_block_info(
        &mut self,
        allow_regexp_after: bool,
    ) -> Result<(Vec<Stmt>, bool, bool), JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut stmts = Vec::new();
        let mut checking = true;
        let mut strict = self.strict;
        let mut has_use_strict = false;
        let prev_strict = self.strict;
        let mut prologue_strict_err: Option<&'static str> = None;
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            if checking {
                if let TokenKind::String(ref s, has_escape) = self.cur.kind {
                    let cur_strict_err = self.cur.strict_error;
                    if s == "use strict" && !has_escape {
                        let st = self.parse_stmt()?;
                        if matches!(st, Stmt::Expr(Expr::String(ref v)) if v == "use strict") {
                            if let Some(err) = prologue_strict_err {
                                return Err(JsError::syntax(err));
                            }
                            strict = true;
                            has_use_strict = true;
                            self.strict = true;
                        } else {
                            checking = false;
                        }
                        stmts.push(st);
                        continue;
                    }
                    if prologue_strict_err.is_none() {
                        prologue_strict_err = cur_strict_err;
                    }
                }
            }
            let st = self.parse_stmt()?;
            if checking && !matches!(st, Stmt::Expr(Expr::String(_))) {
                checking = false;
            }
            stmts.push(st);
        }
        self.expect(&TokenKind::RBrace, allow_regexp_after)?;
        self.strict = prev_strict;
        Ok((stmts, strict, has_use_strict))
    }

    fn parse_param_list(&mut self) -> Result<Vec<Param>, JsError> {
        let mut params = Vec::new();
        while self.cur.kind != TokenKind::RParen && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Ellipsis, true)? {
                let pattern = self.parse_pattern()?;
                params.push(Param {
                    pattern,
                    rest: true,
                });
                break;
            }
            let pat = self.parse_pattern()?;
            let pattern = if self.eat(&TokenKind::Eq, true)? {
                let default_expr = self.parse_assign_expr(true)?;
                Pattern::Assign {
                    left: Box::new(pat),
                    right: Box::new(default_expr),
                }
            } else {
                pat
            };
            params.push(Param {
                pattern,
                rest: false,
            });
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        Ok(params)
    }

    fn parse_class(&mut self, require_name: bool) -> Result<Rc<ClassNode>, JsError> {
        self.expect(&TokenKind::KwClass, true)?;
        let prev_strict = self.strict;
        let prev_class = self.in_class;
        self.strict = true;
        self.in_class = true;

        let name = if !matches!(self.cur.kind, TokenKind::KwExtends | TokenKind::LBrace) {
            Some(self.parse_binding_ident()?)
        } else if require_name {
            self.strict = prev_strict;
            self.in_class = prev_class;
            return Err(JsError::syntax("Class declaration requires a name"));
        } else {
            None
        };

        let super_class = if self.eat(&TokenKind::KwExtends, true)? {
            Some(Box::new(self.parse_lhs_expr(true)?))
        } else {
            None
        };

        self.expect(&TokenKind::LBrace, true)?;
        let mut elements = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Semi, true)? {
                continue;
            }
            elements.push(self.parse_class_element()?);
        }
        self.expect(&TokenKind::RBrace, true)?;

        self.strict = prev_strict;
        self.in_class = prev_class;

        Ok(Rc::new(ClassNode {
            name,
            super_class,
            elements,
        }))
    }

    fn parse_class_element(&mut self) -> Result<ClassElement, JsError> {
        let mut is_static = false;
        if self.is_ident_word("static") {
            let next = self.peek_next(true)?;
            if next.kind == TokenKind::LBrace {
                self.advance(true)?;
                let stmts = self.parse_block_stmts()?;
                return Ok(ClassElement::StaticBlock(stmts));
            }
            if !matches!(
                next.kind,
                TokenKind::LParen | TokenKind::Eq | TokenKind::Semi | TokenKind::RBrace
            ) {
                self.advance(true)?;
                is_static = true;
            }
        }

        let method_start = self.cur.start;
        let mut is_async = false;
        let mut is_generator = false;
        let mut kind = MethodKind::Method;

        if self.is_ident_word("async") {
            let next = self.peek_next(true)?;
            if !next.preceded_by_newline
                && !matches!(
                    next.kind,
                    TokenKind::LParen
                        | TokenKind::Eq
                        | TokenKind::Semi
                        | TokenKind::RBrace
                )
            {
                self.advance(true)?;
                is_async = true;
            }
        }

        if self.eat(&TokenKind::Star, true)? {
            is_generator = true;
        } else if !is_async && (self.is_ident_word("get") || self.is_ident_word("set")) {
            let is_get = self.is_ident_word("get");
            let next = self.peek_next(true)?;
            if !matches!(
                next.kind,
                TokenKind::LParen | TokenKind::Eq | TokenKind::Semi | TokenKind::RBrace
            ) {
                self.advance(true)?;
                kind = if is_get {
                    MethodKind::Get
                } else {
                    MethodKind::Set
                };
            }
        }

        let (key, is_private) = self.parse_class_prop_key()?;

        if self.cur.kind == TokenKind::LParen {
            if !is_static && !is_private && kind == MethodKind::Method {
                if matches!(&key, PropKey::Ident(s) | PropKey::String(s) if s == "constructor") {
                    kind = MethodKind::Constructor;
                }
            }
            let func = self.parse_method_func(method_start, is_async, is_generator, None)?;
            return Ok(ClassElement::Method {
                key,
                kind,
                func,
                is_static,
                is_private,
            });
        }

        let init = if self.eat(&TokenKind::Eq, true)? {
            Some(self.parse_assign_expr(true)?)
        } else {
            None
        };
        self.eat_semi()?;
        Ok(ClassElement::Field {
            key,
            init,
            is_static,
            is_private,
        })
    }

    fn parse_class_prop_key(&mut self) -> Result<(PropKey, bool), JsError> {
        if let TokenKind::PrivateIdent(name) = self.cur.kind.clone() {
            self.advance(true)?;
            return Ok((PropKey::Private(name), true));
        }
        Ok((self.parse_prop_key()?, false))
    }

    fn parse_prop_key(&mut self) -> Result<PropKey, JsError> {
        match self.cur.kind.clone() {
            TokenKind::LBracket => {
                self.advance(true)?;
                let expr = self.parse_assign_expr(true)?;
                self.expect(&TokenKind::RBracket, true)?;
                Ok(PropKey::Computed(Box::new(expr)))
            }
            TokenKind::String(s, _) => {
                self.advance(true)?;
                Ok(PropKey::String(s))
            }
            TokenKind::Number(n) => {
                self.advance(true)?;
                Ok(PropKey::Number(n))
            }
            TokenKind::BigInt(s) => {
                self.advance(true)?;
                Ok(PropKey::String(s))
            }
            _ => {
                let name = self.parse_ident_name()?;
                Ok(PropKey::Ident(name))
            }
        }
    }

    fn parse_method_func(
        &mut self,
        start: usize,
        is_async: bool,
        is_generator: bool,
        name: Option<String>,
    ) -> Result<Rc<FuncNode>, JsError> {
        let prev_async = self.in_async;
        let prev_gen = self.in_generator;
        let prev_fn = self.in_function;
        self.in_async = is_async;
        self.in_generator = is_generator;
        self.in_function = true;

        self.expect(&TokenKind::LParen, true)?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::RParen, true)?;
        let (body, strict, has_use_strict) = self.parse_func_body_block_info(true)?;
        validate_func_params(&params, strict, has_use_strict, false)?;
        let end = self.last_token_end.min(self.lexer.source().len());

        self.in_async = prev_async;
        self.in_generator = prev_gen;
        self.in_function = prev_fn;

        Ok(Rc::new(FuncNode {
            name,
            params,
            body: FuncBody::Block(body),
            is_async,
            is_generator,
            is_arrow: false,
            is_decl: false,
            strict,
            source: self.lexer.source()[start..end].trim().to_string(),
        }))
    }

    fn parse_pattern(&mut self) -> Result<Pattern, JsError> {
        match self.cur.kind {
            TokenKind::LBracket => self.parse_array_pattern(),
            TokenKind::LBrace => self.parse_object_pattern(),
            _ => Ok(Pattern::Ident(self.parse_binding_ident()?)),
        }
    }

    fn parse_array_pattern(&mut self) -> Result<Pattern, JsError> {
        self.expect(&TokenKind::LBracket, true)?;
        let mut elements = Vec::new();
        let mut rest = None;
        while self.cur.kind != TokenKind::RBracket && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Comma, true)? {
                elements.push(None);
                continue;
            }
            if self.eat(&TokenKind::Ellipsis, true)? {
                let pat = self.parse_pattern()?;
                rest = Some(Box::new(pat));
                break;
            }
            let pat = self.parse_pattern()?;
            let pat = if self.eat(&TokenKind::Eq, true)? {
                let right = self.parse_assign_expr(true)?;
                Pattern::Assign {
                    left: Box::new(pat),
                    right: Box::new(right),
                }
            } else {
                pat
            };
            elements.push(Some(pat));
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        self.expect(&TokenKind::RBracket, true)?;
        Ok(Pattern::Array { elements, rest })
    }

    fn parse_object_pattern(&mut self) -> Result<Pattern, JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut props = Vec::new();
        let mut rest = None;
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Ellipsis, true)? {
                let pat = self.parse_pattern()?;
                rest = Some(Box::new(pat));
                break;
            }
            let key = self.parse_prop_key()?;
            let value = if self.eat(&TokenKind::Colon, true)? {
                let pat = self.parse_pattern()?;
                if self.eat(&TokenKind::Eq, true)? {
                    let right = self.parse_assign_expr(true)?;
                    Pattern::Assign {
                        left: Box::new(pat),
                        right: Box::new(right),
                    }
                } else {
                    pat
                }
            } else if let PropKey::Ident(ref name) = key {
                if is_reserved_word(name, self.strict) {
                    return Err(JsError::syntax(format!(
                        "Unexpected reserved word '{}' in destructuring pattern",
                        name
                    )));
                }
                let base = Pattern::Ident(name.clone());
                if self.eat(&TokenKind::Eq, true)? {
                    let right = self.parse_assign_expr(true)?;
                    Pattern::Assign {
                        left: Box::new(base),
                        right: Box::new(right),
                    }
                } else {
                    base
                }
            } else {
                return Err(JsError::syntax("Expected ':' in object pattern"));
            };
            props.push(ObjectPatProp { key, value });
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, true)?;
        Ok(Pattern::Object { props, rest })
    }

    fn parse_expr(&mut self, allow_in: bool) -> Result<Expr, JsError> {
        let first = self.parse_assign_expr(allow_in)?;
        if self.cur.kind != TokenKind::Comma {
            return Ok(first);
        }
        let mut exprs = vec![first];
        while self.eat(&TokenKind::Comma, true)? {
            exprs.push(self.parse_assign_expr(allow_in)?);
        }
        Ok(Expr::Sequence(exprs))
    }

    fn parse_assign_expr(&mut self, allow_in: bool) -> Result<Expr, JsError> {
        if self.cur.kind == TokenKind::KwYield && self.in_generator {
            return self.parse_yield(allow_in);
        }

        if let Some(arrow) = self.try_parse_arrow(allow_in)? {
            return Ok(arrow);
        }

        let left = self.parse_conditional_expr(allow_in)?;
        if let Some(op) = token_to_assign_op(&self.cur.kind) {
            self.advance(true)?;
            let target = expr_to_assign_target(left, op == AssignOp::Assign)?;
            let right = self.parse_assign_expr(allow_in)?;
            return Ok(Expr::Assign {
                op,
                left: target,
                right: Box::new(right),
            });
        }
        Ok(left)
    }

    fn parse_yield(&mut self, allow_in: bool) -> Result<Expr, JsError> {
        self.advance(true)?;
        if self.cur.preceded_by_newline
            || matches!(
                self.cur.kind,
                TokenKind::Semi
                    | TokenKind::RBrace
                    | TokenKind::RParen
                    | TokenKind::RBracket
                    | TokenKind::Comma
                    | TokenKind::Colon
                    | TokenKind::Eof
            )
        {
            return Ok(Expr::Yield {
                arg: None,
                delegate: false,
            });
        }
        let delegate = self.eat(&TokenKind::Star, true)?;
        let arg = Some(Box::new(self.parse_assign_expr(allow_in)?));
        Ok(Expr::Yield { arg, delegate })
    }

    fn try_parse_arrow(&mut self, allow_in: bool) -> Result<Option<Expr>, JsError> {
        let is_async_arrow = if self.is_ident_word("async") {
            let next = self.peek_next(false)?;
            if !next.preceded_by_newline {
                if matches!(next.kind, TokenKind::LParen) {
                    self.is_paren_followed_by_arrow(true)
                } else if matches!(
                    next.kind,
                    TokenKind::Ident(_, _) | TokenKind::KwYield | TokenKind::KwAwait
                ) {
                    let mut lex2 = self.lexer.clone();
                    let _ = lex2.next_token(true);
                    let after_ident = lex2.next_token(false)?;
                    !after_ident.preceded_by_newline && after_ident.kind == TokenKind::Arrow
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if is_async_arrow {
            let start = self.cur.start;
            self.advance(true)?;
            let params = if self.cur.kind == TokenKind::LParen {
                self.advance(true)?;
                let p = self.parse_param_list()?;
                self.expect(&TokenKind::RParen, false)?;
                p
            } else {
                let name = self.parse_binding_ident()?;
                vec![Param {
                    pattern: Pattern::Ident(name),
                    rest: false,
                }]
            };
            return Ok(Some(self.parse_arrow_body(start, true, params, allow_in)?));
        }

        if matches!(
            self.cur.kind,
            TokenKind::Ident(_, _) | TokenKind::KwYield | TokenKind::KwAwait
        ) {
            let next = self.peek_next(false)?;
            if !next.preceded_by_newline && next.kind == TokenKind::Arrow {
                let start = self.cur.start;
                let name = self.parse_binding_ident()?;
                let params = vec![Param {
                    pattern: Pattern::Ident(name),
                    rest: false,
                }];
                return Ok(Some(self.parse_arrow_body(start, false, params, allow_in)?));
            }
        }

        if self.cur.kind == TokenKind::LParen && self.is_paren_followed_by_arrow(false) {
            let start = self.cur.start;
            self.advance(true)?;
            let params = self.parse_param_list()?;
            self.expect(&TokenKind::RParen, false)?;
            return Ok(Some(self.parse_arrow_body(start, false, params, allow_in)?));
        }

        Ok(None)
    }

    fn is_paren_followed_by_arrow(&self, skip_first: bool) -> bool {
        let mut lex = self.lexer.clone();
        if skip_first {
            if lex.next_token(true).is_err() {
                return false;
            }
        }
        let mut depth = 1usize;
        let mut prev_allow_re = true;
        while depth > 0 {
            let Ok(tok) = lex.next_token(prev_allow_re) else {
                return false;
            };
            match tok.kind {
                TokenKind::Eof => return false,
                TokenKind::LParen => {
                    depth += 1;
                    prev_allow_re = true;
                }
                TokenKind::RParen => {
                    depth -= 1;
                    prev_allow_re = false;
                }
                TokenKind::TemplateHead { .. } | TokenKind::TemplateMiddle { .. } => {
                    if !skip_template_in_lookahead(&mut lex) {
                        return false;
                    }
                    prev_allow_re = false;
                }
                _ => {
                    prev_allow_re = !matches!(
                        tok.kind,
                        TokenKind::Ident(_, _)
                            | TokenKind::Number(_)
                            | TokenKind::String(_, _)
                            | TokenKind::RBracket
                            | TokenKind::RBrace
                            | TokenKind::KwTrue
                            | TokenKind::KwFalse
                            | TokenKind::KwNull
                            | TokenKind::KwThis
                    );
                }
            }
        }
        match lex.next_token(false) {
            Ok(tok) => !tok.preceded_by_newline && tok.kind == TokenKind::Arrow,
            Err(_) => false,
        }
    }

    fn parse_arrow_body(
        &mut self,
        start: usize,
        is_async: bool,
        params: Vec<Param>,
        allow_in: bool,
    ) -> Result<Expr, JsError> {
        if self.cur.preceded_by_newline {
            return Err(JsError::syntax("Illegal newline before '=>'"));
        }
        self.expect(&TokenKind::Arrow, true)?;
        let prev_async = self.in_async;
        let prev_gen = self.in_generator;
        let prev_fn = self.in_function;
        self.in_async = is_async;
        self.in_generator = false;
        self.in_function = true;

        let (body, strict, has_use_strict) = if self.cur.kind == TokenKind::LBrace {
            let (stmts, st, hs) = self.parse_func_body_block_info(false)?;
            (FuncBody::Block(stmts), st, hs)
        } else {
            let expr = self.parse_assign_expr(allow_in)?;
            (FuncBody::Expr(Box::new(expr)), self.strict, false)
        };
        validate_func_params(&params, strict, has_use_strict, true)?;
        let end = self.last_token_end.min(self.lexer.source().len());

        self.in_async = prev_async;
        self.in_generator = prev_gen;
        self.in_function = prev_fn;

        Ok(Expr::Arrow(Rc::new(FuncNode {
            name: None,
            params,
            body,
            is_async,
            is_generator: false,
            is_arrow: true,
            is_decl: false,
            strict,
            source: self.lexer.source()[start..end].trim().to_string(),
        })))
    }

    fn parse_conditional_expr(&mut self, allow_in: bool) -> Result<Expr, JsError> {
        let test = self.parse_binary_expr(1, allow_in)?;
        if self.eat(&TokenKind::Question, true)? {
            let consequent = self.parse_assign_expr(true)?;
            self.expect(&TokenKind::Colon, true)?;
            let alternate = self.parse_assign_expr(allow_in)?;
            return Ok(Expr::Conditional {
                test: Box::new(test),
                consequent: Box::new(consequent),
                alternate: Box::new(alternate),
            });
        }
        Ok(test)
    }

    fn parse_binary_expr(&mut self, min_prec: u8, allow_in: bool) -> Result<Expr, JsError> {
        let left_was_unary = matches!(
            self.cur.kind,
            TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Bang
                | TokenKind::Tilde
                | TokenKind::KwTypeof
                | TokenKind::KwVoid
                | TokenKind::KwDelete
        ) || (self.in_async && self.cur.kind == TokenKind::KwAwait);
        let mut left = if let TokenKind::PrivateIdent(name) = self.cur.kind.clone() {
            self.advance(false)?;
            if !allow_in || self.cur.kind != TokenKind::KwIn {
                return Err(JsError::syntax("Unexpected private identifier"));
            }
            self.advance(true)?;
            let right = self.parse_binary_expr(8, allow_in)?;
            Expr::PrivateIn {
                name,
                right: Box::new(right),
            }
        } else {
            self.parse_unary_expr()?
        };
        if left_was_unary && self.cur.kind == TokenKind::StarStar {
            return Err(JsError::syntax(
                "Unary operator used immediately before exponentiation expression",
            ));
        }

        let mut saw_coalesce = false;
        let mut saw_logical_and_or = false;
        loop {
            let Some((prec, right_assoc)) = binary_prec(&self.cur.kind, allow_in) else {
                break;
            };
            if prec < min_prec {
                break;
            }
            let op_tok = self.advance(true)?;
            match op_tok.kind {
                TokenKind::QuestionQuestion => {
                    if saw_logical_and_or {
                        return Err(JsError::syntax(
                            "Cannot mix '??' with '&&' or '||' without parentheses",
                        ));
                    }
                    saw_coalesce = true;
                }
                TokenKind::AmpAmp | TokenKind::PipePipe => {
                    if saw_coalesce {
                        return Err(JsError::syntax(
                            "Cannot mix '??' with '&&' or '||' without parentheses",
                        ));
                    }
                    saw_logical_and_or = true;
                }
                _ => {}
            }
            let next_min = if op_tok.kind == TokenKind::QuestionQuestion {
                4
            } else if right_assoc {
                prec
            } else {
                prec + 1
            };
            let right = self.parse_binary_expr(next_min, allow_in)?;
            if op_tok.kind == TokenKind::QuestionQuestion {
                if matches!(
                    self.cur.kind,
                    TokenKind::AmpAmp | TokenKind::PipePipe
                ) && min_prec <= 2
                {
                    return Err(JsError::syntax(
                        "Cannot mix '??' with '&&' or '||' without parentheses",
                    ));
                }
            }
            if matches!(op_tok.kind, TokenKind::AmpAmp | TokenKind::PipePipe)
                && self.cur.kind == TokenKind::QuestionQuestion
                && min_prec <= 2
            {
                return Err(JsError::syntax(
                    "Cannot mix '??' with '&&' or '||' without parentheses",
                ));
            }
            left = match op_tok.kind {
                TokenKind::AmpAmp => Expr::Logical {
                    op: LogicalOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                TokenKind::PipePipe => Expr::Logical {
                    op: LogicalOp::Or,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                TokenKind::QuestionQuestion => Expr::Logical {
                    op: LogicalOp::Nullish,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                ref k => Expr::Binary {
                    op: token_to_binary_op(k).unwrap(),
                    left: Box::new(left),
                    right: Box::new(right),
                },
            };
        }
        Ok(left)
    }

    fn parse_unary_expr(&mut self) -> Result<Expr, JsError> {
        match self.cur.kind {
            TokenKind::Plus => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Plus,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::Minus => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Minus,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::Bang => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::Tilde => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::BitNot,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::KwTypeof => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::TypeOf,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::KwVoid => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Void,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::KwDelete => {
                self.advance(true)?;
                Ok(Expr::Unary {
                    op: UnaryOp::Delete,
                    arg: Box::new(self.parse_unary_expr()?),
                })
            }
            TokenKind::KwAwait if self.in_async => {
                self.advance(true)?;
                Ok(Expr::Await(Box::new(self.parse_unary_expr()?)))
            }
            TokenKind::PlusPlus => {
                self.advance(true)?;
                Ok(Expr::Update {
                    op: UpdateOp::Inc,
                    arg: Box::new(self.parse_unary_expr()?),
                    prefix: true,
                })
            }
            TokenKind::MinusMinus => {
                self.advance(true)?;
                Ok(Expr::Update {
                    op: UpdateOp::Dec,
                    arg: Box::new(self.parse_unary_expr()?),
                    prefix: true,
                })
            }
            _ => self.parse_postfix_expr(),
        }
    }

    fn parse_postfix_expr(&mut self) -> Result<Expr, JsError> {
        let expr = self.parse_lhs_expr(true)?;
        if !self.cur.preceded_by_newline {
            if self.eat(&TokenKind::PlusPlus, false)? {
                return Ok(Expr::Update {
                    op: UpdateOp::Inc,
                    arg: Box::new(expr),
                    prefix: false,
                });
            }
            if self.eat(&TokenKind::MinusMinus, false)? {
                return Ok(Expr::Update {
                    op: UpdateOp::Dec,
                    arg: Box::new(expr),
                    prefix: false,
                });
            }
        }
        Ok(expr)
    }

    fn parse_lhs_expr(&mut self, allow_call: bool) -> Result<Expr, JsError> {
        let mut expr = if self.cur.kind == TokenKind::KwNew {
            self.parse_new_expr()?
        } else if self.cur.kind == TokenKind::KwImport {
            self.parse_import_expr(allow_call)?
        } else {
            self.parse_primary_expr()?
        };

        let mut is_optional_chain = false;
        loop {
            match self.cur.kind {
                TokenKind::Dot => {
                    self.advance(true)?;
                    let prop = if let TokenKind::PrivateIdent(name) = self.cur.kind.clone() {
                        self.advance(false)?;
                        MemberProp::Private(name)
                    } else {
                        MemberProp::Ident(self.parse_ident_name_after_dot()?)
                    };
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: prop,
                        optional: false,
                    };
                }
                TokenKind::LBracket => {
                    self.advance(true)?;
                    let prop_expr = self.parse_expr(true)?;
                    self.expect(&TokenKind::RBracket, false)?;
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: MemberProp::Computed(Box::new(prop_expr)),
                        optional: false,
                    };
                }
                TokenKind::QuestionDot => {
                    is_optional_chain = true;
                    self.advance(true)?;
                    if self.cur.kind == TokenKind::LParen {
                        let args = self.parse_call_args()?;
                        expr = Expr::Call {
                            callee: Box::new(expr),
                            args,
                            optional: true,
                        };
                    } else if self.cur.kind == TokenKind::LBracket {
                        self.advance(true)?;
                        let prop_expr = self.parse_expr(true)?;
                        self.expect(&TokenKind::RBracket, false)?;
                        expr = Expr::Member {
                            object: Box::new(expr),
                            property: MemberProp::Computed(Box::new(prop_expr)),
                            optional: true,
                        };
                    } else if let TokenKind::PrivateIdent(name) = self.cur.kind.clone() {
                        self.advance(false)?;
                        expr = Expr::Member {
                            object: Box::new(expr),
                            property: MemberProp::Private(name),
                            optional: true,
                        };
                    } else {
                        let name = self.parse_ident_name_after_dot()?;
                        expr = Expr::Member {
                            object: Box::new(expr),
                            property: MemberProp::Ident(name),
                            optional: true,
                        };
                    }
                }
                TokenKind::LParen if allow_call => {
                    let args = self.parse_call_args()?;
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                        optional: false,
                    };
                }
                TokenKind::NoSubTemplate { .. } | TokenKind::TemplateHead { .. } => {
                    if is_optional_chain {
                        return Err(JsError::syntax(
                            "Invalid tagged template on optional chain",
                        ));
                    }
                    expr = self.parse_template_literal(Some(Box::new(expr)))?;
                }
                _ => break,
            }
        }
        if is_optional_chain {
            Ok(Expr::OptionalChain(Box::new(expr)))
        } else {
            Ok(expr)
        }
    }

    fn parse_import_expr(&mut self, allow_call: bool) -> Result<Expr, JsError> {
        self.advance(true)?;
        let phase = if self.eat(&TokenKind::Dot, true)? {
            match self.cur.kind.clone() {
                TokenKind::Ident(s, false) if s == "meta" => {
                    if !self.is_module {
                        return Err(JsError::syntax("import.meta may only be used in a module"));
                    }
                    self.advance(false)?;
                    return Ok(Expr::ImportMeta);
                }
                TokenKind::Ident(s, false) if s == "defer" => {
                    self.advance(true)?;
                    ImportPhase::Defer
                }
                TokenKind::Ident(s, false) if s == "source" => {
                    self.advance(true)?;
                    ImportPhase::Source
                }
                _ => {
                    return Err(JsError::syntax("Unknown meta property on import"));
                }
            }
        } else {
            ImportPhase::Evaluation
        };
        if !allow_call {
            return Err(JsError::syntax("Cannot use new with import()"));
        }
        self.expect(&TokenKind::LParen, true)?;
        let spec = self.parse_assign_expr(true)?;
        let options = if self.eat(&TokenKind::Comma, true)? {
            if self.cur.kind != TokenKind::RParen {
                let opt = self.parse_assign_expr(true)?;
                let _ = self.eat(&TokenKind::Comma, true);
                Some(Box::new(opt))
            } else {
                None
            }
        } else {
            None
        };
        self.expect(&TokenKind::RParen, false)?;
        Ok(Expr::DynamicImport {
            spec: Box::new(spec),
            options,
            phase,
        })
    }

    fn parse_new_expr(&mut self) -> Result<Expr, JsError> {
        self.advance(true)?;
        if self.eat(&TokenKind::Dot, true)? {
            let name = self.parse_ident_name_after_dot()?;
            if name == "target" {
                return Ok(Expr::NewTarget);
            }
            return Err(JsError::syntax(format!("Unknown meta property new.{}", name)));
        }
        let callee = self.parse_lhs_expr(false)?;
        let args = if self.cur.kind == TokenKind::LParen {
            self.parse_call_args()?
        } else {
            Vec::new()
        };
        Ok(Expr::New {
            callee: Box::new(callee),
            args,
        })
    }

    fn parse_call_args(&mut self) -> Result<Vec<CallArg>, JsError> {
        self.expect(&TokenKind::LParen, true)?;
        let mut args = Vec::new();
        while self.cur.kind != TokenKind::RParen && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Ellipsis, true)? {
                args.push(CallArg::Spread(self.parse_assign_expr(true)?));
            } else {
                args.push(CallArg::Expr(self.parse_assign_expr(true)?));
            }
            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        self.expect(&TokenKind::RParen, false)?;
        Ok(args)
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, JsError> {
        match self.cur.kind.clone() {
            TokenKind::KwNull => {
                self.advance(false)?;
                Ok(Expr::Null)
            }
            TokenKind::KwTrue => {
                self.advance(false)?;
                Ok(Expr::Bool(true))
            }
            TokenKind::KwFalse => {
                self.advance(false)?;
                Ok(Expr::Bool(false))
            }
            TokenKind::Number(n) => {
                self.advance(false)?;
                Ok(Expr::Number(n))
            }
            TokenKind::BigInt(s) => {
                self.advance(false)?;
                Ok(Expr::BigInt(s))
            }
            TokenKind::String(s, _) => {
                self.advance(false)?;
                Ok(Expr::String(s))
            }
            TokenKind::RegExp(pattern, flags) => {
                self.advance(false)?;
                crate::regexp::CompiledRegExp::compile(&pattern, &flags)?;
                Ok(Expr::RegExp { pattern, flags })
            }
            TokenKind::KwThis => {
                self.advance(false)?;
                Ok(Expr::This)
            }
            TokenKind::KwSuper => {
                self.advance(false)?;
                Ok(Expr::Super)
            }
            TokenKind::KwImport => self.parse_import_expr(true),
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(false)?;
                if !next.preceded_by_newline && next.kind == TokenKind::KwFunction {
                    let func = self.parse_function(false, false)?;
                    Ok(Expr::Function(func))
                } else {
                    self.advance(false)?;
                    Ok(Expr::Ident(s))
                }
            }
            TokenKind::Ident(s, escaped) => {
                if is_reserved_word(&s, self.strict)
                    || (escaped && ((self.in_async && s == "await") || (self.in_generator && s == "yield")))
                {
                    return Err(JsError::syntax(format!("Unexpected reserved word '{}'", s)));
                }
                self.advance(false)?;
                Ok(Expr::Ident(s))
            }
            TokenKind::KwYield if !self.in_generator && !self.strict => {
                self.advance(false)?;
                Ok(Expr::Ident("yield".to_string()))
            }
            TokenKind::KwAwait if !self.in_async => {
                self.advance(false)?;
                Ok(Expr::Ident("await".to_string()))
            }
            TokenKind::KwFunction => {
                let func = self.parse_function(false, false)?;
                Ok(Expr::Function(func))
            }
            TokenKind::KwClass => {
                let cls = self.parse_class(false)?;
                Ok(Expr::Class(cls))
            }
            TokenKind::LParen => {
                self.advance(true)?;
                let expr = self.parse_expr(true)?;
                self.expect(&TokenKind::RParen, false)?;
                Ok(expr)
            }
            TokenKind::LBracket => self.parse_array_literal(),
            TokenKind::LBrace => self.parse_object_literal(),
            TokenKind::NoSubTemplate { .. } | TokenKind::TemplateHead { .. } => {
                self.parse_template_literal(None)
            }
            other => Err(JsError::syntax(format!(
                "Unexpected token {:?} at {}:{}",
                other, self.cur.line, self.cur.col
            ))),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, JsError> {
        self.expect(&TokenKind::LBracket, true)?;
        let mut elems = Vec::new();
        while self.cur.kind != TokenKind::RBracket && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Comma, true)? {
                elems.push(ArrayElem::Hole);
                continue;
            }
            let is_spread = self.eat(&TokenKind::Ellipsis, true)?;
            let expr = self.parse_assign_expr(true)?;
            let has_comma = self.eat(&TokenKind::Comma, true)?;
            if is_spread {
                elems.push(ArrayElem::Spread(expr, has_comma));
            } else {
                elems.push(ArrayElem::Expr(expr));
            }
            if !has_comma {
                break;
            }
        }
        self.expect(&TokenKind::RBracket, false)?;
        Ok(Expr::Array(elems))
    }

    fn parse_object_literal(&mut self) -> Result<Expr, JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut props = Vec::new();
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            if self.eat(&TokenKind::Ellipsis, true)? {
                let expr = self.parse_assign_expr(true)?;
                let has_comma = self.eat(&TokenKind::Comma, true)?;
                props.push(ObjectProp::Spread(expr, has_comma));
                if !has_comma {
                    break;
                }
                continue;
            }

            let method_start = self.cur.start;
            let mut is_async = false;
            let mut is_generator = false;
            let mut method_kind = None;

            if self.is_ident_word("async") {
                let next = self.peek_next(true)?;
                if !next.preceded_by_newline
                    && !matches!(
                        next.kind,
                        TokenKind::Colon
                            | TokenKind::Comma
                            | TokenKind::RBrace
                            | TokenKind::LParen
                            | TokenKind::Eq
                    )
                {
                    self.advance(true)?;
                    is_async = true;
                }
            }

            if self.eat(&TokenKind::Star, true)? {
                is_generator = true;
            } else if !is_async && (self.is_ident_word("get") || self.is_ident_word("set")) {
                let is_get = self.is_ident_word("get");
                let next = self.peek_next(true)?;
                if !matches!(
                    next.kind,
                    TokenKind::Colon
                        | TokenKind::Comma
                        | TokenKind::RBrace
                        | TokenKind::LParen
                        | TokenKind::Eq
                ) {
                    self.advance(true)?;
                    method_kind = Some(if is_get {
                        MethodKind::Get
                    } else {
                        MethodKind::Set
                    });
                }
            }

            let key = self.parse_prop_key()?;
            if self.cur.kind == TokenKind::LParen {
                let kind = method_kind.unwrap_or(MethodKind::Method);
                let name = match &key {
                    PropKey::Ident(s) | PropKey::String(s) => Some(s.clone()),
                    _ => None,
                };
                let func = self.parse_method_func(method_start, is_async, is_generator, name)?;
                props.push(ObjectProp::Method { key, kind, func });
            } else if self.eat(&TokenKind::Colon, true)? {
                let value = self.parse_assign_expr(true)?;
                props.push(ObjectProp::Property {
                    key,
                    value,
                    shorthand: false,
                    is_cover_init: false,
                });
            } else if let PropKey::Ident(ref name) = key {
                if is_reserved_word(name, self.strict)
                    || (self.in_generator && name == "yield")
                    || (self.in_async && name == "await")
                {
                    return Err(JsError::syntax(format!(
                        "Unexpected reserved word '{}' in shorthand property",
                        name
                    )));
                }
                let (value, is_cover_init) = if self.eat(&TokenKind::Eq, true)? {
                    let right = self.parse_assign_expr(true)?;
                    (
                        Expr::Assign {
                            op: AssignOp::Assign,
                            left: AssignTarget::Ident(name.clone()),
                            right: Box::new(right),
                        },
                        true,
                    )
                } else {
                    (Expr::Ident(name.clone()), false)
                };
                props.push(ObjectProp::Property {
                    key,
                    value,
                    shorthand: true,
                    is_cover_init,
                });
            } else {
                return Err(JsError::syntax("Expected ':' in object literal property"));
            }

            if !self.eat(&TokenKind::Comma, true)? {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, false)?;
        Ok(Expr::Object(props))
    }

    fn parse_template_literal(&mut self, tag: Option<Box<Expr>>) -> Result<Expr, JsError> {
        let mut quasis = Vec::new();
        let mut exprs = Vec::new();
        match self.cur.kind.clone() {
            TokenKind::NoSubTemplate { raw, cooked } => {
                if tag.is_none() && cooked.is_none() {
                    return Err(JsError::syntax("Invalid escape in template literal"));
                }
                quasis.push(TemplateQuasi { raw, cooked });
                self.advance(false)?;
                return Ok(Expr::Template { tag, quasis, exprs });
            }
            TokenKind::TemplateHead { raw, cooked } => {
                if tag.is_none() && cooked.is_none() {
                    return Err(JsError::syntax("Invalid escape in template literal"));
                }
                quasis.push(TemplateQuasi { raw, cooked });
                self.advance(true)?;
            }
            _ => return Err(JsError::syntax("Expected template literal")),
        }

        loop {
            let expr = self.parse_expr(true)?;
            exprs.push(expr);
            if self.cur.kind != TokenKind::RBrace {
                return Err(JsError::syntax("Expected '}' in template expression"));
            }
            self.cur = self.lexer.next_template_continuation()?;
            match self.cur.kind.clone() {
                TokenKind::TemplateMiddle { raw, cooked } => {
                    if tag.is_none() && cooked.is_none() {
                        return Err(JsError::syntax("Invalid escape in template literal"));
                    }
                    quasis.push(TemplateQuasi { raw, cooked });
                    self.advance(true)?;
                }
                TokenKind::TemplateTail { raw, cooked } => {
                    if tag.is_none() && cooked.is_none() {
                        return Err(JsError::syntax("Invalid escape in template literal"));
                    }
                    quasis.push(TemplateQuasi { raw, cooked });
                    self.advance(false)?;
                    break;
                }
                _ => return Err(JsError::syntax("Unexpected token in template literal")),
            }
        }
        Ok(Expr::Template { tag, quasis, exprs })
    }

    fn parse_binding_ident(&mut self) -> Result<String, JsError> {
        match self.cur.kind.clone() {
            TokenKind::Ident(s, escaped) => {
                if is_reserved_word(&s, self.strict)
                    || (escaped && ((self.in_async && s == "await") || (self.in_generator && s == "yield")))
                {
                    return Err(JsError::syntax(format!("Unexpected reserved word '{}'", s)));
                }
                self.advance(false)?;
                Ok(s)
            }
            TokenKind::KwYield if !self.in_generator && !self.strict => {
                self.advance(false)?;
                Ok("yield".to_string())
            }
            TokenKind::KwAwait if !self.in_async => {
                self.advance(false)?;
                Ok("await".to_string())
            }
            other => Err(JsError::syntax(format!(
                "Expected identifier, found {:?} at {}:{}",
                other, self.cur.line, self.cur.col
            ))),
        }
    }

    fn parse_ident_name(&mut self) -> Result<String, JsError> {
        let name = token_as_ident_name(&self.cur.kind).ok_or_else(|| {
            JsError::syntax(format!(
                "Expected property name, found {:?} at {}:{}",
                self.cur.kind, self.cur.line, self.cur.col
            ))
        })?;
        self.advance(true)?;
        Ok(name)
    }

    fn parse_ident_name_after_dot(&mut self) -> Result<String, JsError> {
        let name = token_as_ident_name(&self.cur.kind).ok_or_else(|| {
            JsError::syntax(format!(
                "Expected property name after '.', found {:?} at {}:{}",
                self.cur.kind, self.cur.line, self.cur.col
            ))
        })?;
        self.advance(false)?;
        Ok(name)
    }
}

fn is_simple_param_list(params: &[Param]) -> bool {
    params
        .iter()
        .all(|p| !p.rest && matches!(p.pattern, Pattern::Ident(_)))
}

fn validate_func_params(
    params: &[Param],
    strict: bool,
    has_use_strict: bool,
    is_arrow: bool,
) -> Result<(), JsError> {
    let simple = is_simple_param_list(params);
    if has_use_strict && !simple {
        return Err(JsError::syntax(
            "Illegal 'use strict' directive in function with non-simple parameter list",
        ));
    }
    if strict || is_arrow || !simple {
        let mut names = Vec::new();
        for p in params {
            crate::compiler::collect_pattern_names(&p.pattern, &mut names, false);
        }
        let mut seen = std::collections::HashSet::new();
        for (n, _) in names {
            if !seen.insert(n.clone()) {
                return Err(JsError::syntax(format!(
                    "Duplicate parameter name '{}' not allowed",
                    n
                )));
            }
        }
    }
    Ok(())
}

fn skip_template_in_lookahead(lex: &mut Lexer<'_>) -> bool {
    let mut brace_depth = 1usize;
    while brace_depth > 0 {
        let Ok(tok) = lex.next_token(true) else {
            return false;
        };
        match tok.kind {
            TokenKind::Eof => return false,
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => {
                brace_depth -= 1;
                if brace_depth == 0 {
                    match lex.next_template_continuation() {
                        Ok(t) => match t.kind {
                            TokenKind::TemplateMiddle { .. } => brace_depth = 1,
                            TokenKind::TemplateTail { .. } => return true,
                            _ => return false,
                        },
                        Err(_) => return false,
                    }
                }
            }
            TokenKind::TemplateHead { .. } => {
                if !skip_template_in_lookahead(lex) {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

fn token_as_ident_name(kind: &TokenKind) -> Option<String> {
    Some(
        match kind {
            TokenKind::Ident(s, _) => return Some(s.clone()),
            TokenKind::KwBreak => "break",
            TokenKind::KwCase => "case",
            TokenKind::KwCatch => "catch",
            TokenKind::KwClass => "class",
            TokenKind::KwConst => "const",
            TokenKind::KwContinue => "continue",
            TokenKind::KwDebugger => "debugger",
            TokenKind::KwDefault => "default",
            TokenKind::KwDelete => "delete",
            TokenKind::KwDo => "do",
            TokenKind::KwElse => "else",
            TokenKind::KwExport => "export",
            TokenKind::KwExtends => "extends",
            TokenKind::KwFalse => "false",
            TokenKind::KwFinally => "finally",
            TokenKind::KwFor => "for",
            TokenKind::KwFunction => "function",
            TokenKind::KwIf => "if",
            TokenKind::KwImport => "import",
            TokenKind::KwIn => "in",
            TokenKind::KwInstanceof => "instanceof",
            TokenKind::KwNew => "new",
            TokenKind::KwNull => "null",
            TokenKind::KwReturn => "return",
            TokenKind::KwSuper => "super",
            TokenKind::KwSwitch => "switch",
            TokenKind::KwThis => "this",
            TokenKind::KwThrow => "throw",
            TokenKind::KwTrue => "true",
            TokenKind::KwTry => "try",
            TokenKind::KwTypeof => "typeof",
            TokenKind::KwVar => "var",
            TokenKind::KwVoid => "void",
            TokenKind::KwWhile => "while",
            TokenKind::KwWith => "with",
            TokenKind::KwYield => "yield",
            TokenKind::KwAwait => "await",
            _ => return None,
        }
        .to_string(),
    )
}

fn token_to_assign_op(kind: &TokenKind) -> Option<AssignOp> {
    Some(match kind {
        TokenKind::Eq => AssignOp::Assign,
        TokenKind::PlusEq => AssignOp::Add,
        TokenKind::MinusEq => AssignOp::Sub,
        TokenKind::StarEq => AssignOp::Mul,
        TokenKind::SlashEq => AssignOp::Div,
        TokenKind::PercentEq => AssignOp::Rem,
        TokenKind::StarStarEq => AssignOp::Exp,
        TokenKind::LtLtEq => AssignOp::Shl,
        TokenKind::GtGtEq => AssignOp::Shr,
        TokenKind::GtGtGtEq => AssignOp::UShr,
        TokenKind::AmpEq => AssignOp::BitAnd,
        TokenKind::PipeEq => AssignOp::BitOr,
        TokenKind::CaretEq => AssignOp::BitXor,
        TokenKind::AmpAmpEq => AssignOp::And,
        TokenKind::PipePipeEq => AssignOp::Or,
        TokenKind::QuestionQuestionEq => AssignOp::Nullish,
        _ => return None,
    })
}

fn binary_prec(kind: &TokenKind, allow_in: bool) -> Option<(u8, bool)> {
    Some(match kind {
        TokenKind::QuestionQuestion => (1, false),
        TokenKind::PipePipe => (2, false),
        TokenKind::AmpAmp => (3, false),
        TokenKind::Pipe => (4, false),
        TokenKind::Caret => (5, false),
        TokenKind::Amp => (6, false),
        TokenKind::EqEq | TokenKind::NotEq | TokenKind::EqEqEq | TokenKind::NotEqEq => (7, false),
        TokenKind::Lt | TokenKind::LtEq | TokenKind::Gt | TokenKind::GtEq | TokenKind::KwInstanceof => {
            (8, false)
        }
        TokenKind::KwIn if allow_in => (8, false),
        TokenKind::LtLt | TokenKind::GtGt | TokenKind::GtGtGt => (9, false),
        TokenKind::Plus | TokenKind::Minus => (10, false),
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => (11, false),
        TokenKind::StarStar => (12, true),
        _ => return None,
    })
}

fn token_to_binary_op(kind: &TokenKind) -> Option<BinaryOp> {
    Some(match kind {
        TokenKind::Plus => BinaryOp::Add,
        TokenKind::Minus => BinaryOp::Sub,
        TokenKind::Star => BinaryOp::Mul,
        TokenKind::Slash => BinaryOp::Div,
        TokenKind::Percent => BinaryOp::Rem,
        TokenKind::StarStar => BinaryOp::Exp,
        TokenKind::EqEq => BinaryOp::Eq,
        TokenKind::NotEq => BinaryOp::Neq,
        TokenKind::EqEqEq => BinaryOp::StrictEq,
        TokenKind::NotEqEq => BinaryOp::StrictNeq,
        TokenKind::Lt => BinaryOp::Lt,
        TokenKind::LtEq => BinaryOp::Lte,
        TokenKind::Gt => BinaryOp::Gt,
        TokenKind::GtEq => BinaryOp::Gte,
        TokenKind::LtLt => BinaryOp::Shl,
        TokenKind::GtGt => BinaryOp::Shr,
        TokenKind::GtGtGt => BinaryOp::UShr,
        TokenKind::Amp => BinaryOp::BitAnd,
        TokenKind::Pipe => BinaryOp::BitOr,
        TokenKind::Caret => BinaryOp::BitXor,
        TokenKind::KwIn => BinaryOp::In,
        TokenKind::KwInstanceof => BinaryOp::InstanceOf,
        _ => return None,
    })
}

fn expr_to_assign_target(expr: Expr, allow_destructure: bool) -> Result<AssignTarget, JsError> {
    match expr {
        Expr::Ident(name) => Ok(AssignTarget::Ident(name)),
        Expr::Member {
            object,
            property,
            optional: false,
        } => Ok(AssignTarget::Member { object, property }),
        Expr::Array(_) | Expr::Object(_) if allow_destructure => {
            Ok(AssignTarget::Pattern(expr_to_pattern(expr)?))
        }
        _ => Err(JsError::syntax("Invalid left-hand side in assignment")),
    }
}

fn expr_to_pattern(expr: Expr) -> Result<Pattern, JsError> {
    match expr {
        Expr::Ident(name) => Ok(Pattern::Ident(name)),
        Expr::Member {
            optional: false, ..
        } => Ok(Pattern::Member(Box::new(expr))),
        Expr::Assign {
            op: AssignOp::Assign,
            left,
            right,
        } => {
            let left_pat = match left {
                AssignTarget::Ident(n) => Pattern::Ident(n),
                AssignTarget::Member { object, property } => Pattern::Member(Box::new(
                    Expr::Member {
                        object,
                        property,
                        optional: false,
                    },
                )),
                AssignTarget::Pattern(p) => p,
            };
            Ok(Pattern::Assign {
                left: Box::new(left_pat),
                right,
            })
        }
        Expr::Array(elems) => {
            let mut elements = Vec::new();
            let mut rest = None;
            let len = elems.len();
            for (i, el) in elems.into_iter().enumerate() {
                match el {
                    ArrayElem::Hole => elements.push(None),
                    ArrayElem::Expr(e) => elements.push(Some(expr_to_pattern(e)?)),
                    ArrayElem::Spread(e, has_comma) => {
                        if i + 1 != len || has_comma {
                            return Err(JsError::syntax(
                                "Rest element must be last in destructuring pattern",
                            ));
                        }
                        if matches!(e, Expr::Assign { .. }) {
                            return Err(JsError::syntax(
                                "Rest element may not have a default initializer",
                            ));
                        }
                        rest = Some(Box::new(expr_to_pattern(e)?));
                    }
                }
            }
            Ok(Pattern::Array { elements, rest })
        }
        Expr::Object(props) => {
            let mut pat_props = Vec::new();
            let mut rest = None;
            let len = props.len();
            for (i, prop) in props.into_iter().enumerate() {
                match prop {
                    ObjectProp::Property { key, value, .. } => {
                        pat_props.push(ObjectPatProp {
                            key,
                            value: expr_to_pattern(value)?,
                        });
                    }
                    ObjectProp::Spread(e, has_comma) => {
                        if i + 1 != len || has_comma {
                            return Err(JsError::syntax(
                                "Rest element must be last in object pattern",
                            ));
                        }
                        if matches!(e, Expr::Assign { .. }) {
                            return Err(JsError::syntax(
                                "Rest element may not have a default initializer",
                            ));
                        }
                        rest = Some(Box::new(expr_to_pattern(e)?));
                    }
                    ObjectProp::Method { .. } => {
                        return Err(JsError::syntax("Method not allowed in destructuring pattern"));
                    }
                }
            }
            Ok(Pattern::Object {
                props: pat_props,
                rest,
            })
        }
        _ => Err(JsError::syntax("Invalid destructuring assignment target")),
    }
}

fn is_reserved_word(s: &str, strict: bool) -> bool {
    matches!(
        s,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
    ) || (strict
        && matches!(
            s,
            "implements"
                | "interface"
                | "let"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
                | "yield"
        ))
}

#[derive(Clone, Copy)]
struct ValidationCtx<'a> {
    strict: bool,
    allow_super_call: bool,
    allow_super_prop: bool,
    in_function: bool,
    allow_new_target: bool,
    allow_private: bool,
    in_static_block: bool,
    forbid_yield_expr: bool,
    forbid_await_expr: bool,
    forbid_arguments: bool,
    privates: &'a std::collections::HashSet<String>,
}

fn is_labeled_func_decl(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Labeled { body, .. } => {
            matches!(body.as_ref(), Stmt::FuncDecl(_)) || is_labeled_func_decl(body)
        }
        _ => false,
    }
}

fn validate_program_early_errors(
    prog: &Program,
    eval_ctx: EvalValidationCtx,
) -> Result<(), JsError> {
    let ctx = ValidationCtx {
        strict: prog.strict,
        allow_super_call: eval_ctx.allow_super_call,
        allow_super_prop: eval_ctx.allow_super_prop,
        in_function: false,
        allow_new_target: eval_ctx.allow_new_target,
        allow_private: eval_ctx.allow_private,
        in_static_block: false,
        forbid_yield_expr: false,
        forbid_await_expr: false,
        forbid_arguments: eval_ctx.forbid_arguments,
        privates: &eval_ctx.declared_privates,
    };
    validate_stmt_list(&prog.body, ctx, true, None)?;
    Ok(())
}

fn collect_pattern_names(pat: &Pattern, out: &mut Vec<String>) {
    match pat {
        Pattern::Ident(s) => out.push(s.clone()),
        Pattern::Array { elements, rest } => {
            for el in elements.iter().flatten() {
                collect_pattern_names(el, out);
            }
            if let Some(r) = rest {
                collect_pattern_names(r, out);
            }
        }
        Pattern::Object { props, rest } => {
            for p in props {
                collect_pattern_names(&p.value, out);
            }
            if let Some(r) = rest {
                collect_pattern_names(r, out);
            }
        }
        Pattern::Assign { left, .. } => collect_pattern_names(left, out),
        Pattern::Member(_) => {}
    }
}

fn collect_var_declared_names(stmt: &Stmt, out: &mut Vec<String>) {
    match stmt {
        Stmt::VarDecl {
            kind: VarKind::Var,
            decls,
        } => {
            for d in decls {
                collect_pattern_names(&d.pattern, out);
            }
        }
        Stmt::Block(stmts) => {
            for s in stmts {
                collect_var_declared_names(s, out);
            }
        }
        Stmt::If {
            consequent,
            alternate,
            ..
        } => {
            collect_var_declared_names(consequent, out);
            if let Some(alt) = alternate {
                collect_var_declared_names(alt, out);
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::With { body, .. }
        | Stmt::Labeled { body, .. } => {
            collect_var_declared_names(body, out);
        }
        Stmt::For { init, body, .. } => {
            if let Some(ForInit::VarDecl {
                kind: VarKind::Var,
                decls,
            }) = init
            {
                for d in decls {
                    collect_pattern_names(&d.pattern, out);
                }
            }
            collect_var_declared_names(body, out);
        }
        Stmt::ForIn { left, body, .. } | Stmt::ForOf { left, body, .. } => {
            if let ForInOfLeft::VarDecl {
                kind: VarKind::Var,
                pattern,
            } = left
            {
                collect_pattern_names(pattern, out);
            }
            collect_var_declared_names(body, out);
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &c.consequent {
                    collect_var_declared_names(s, out);
                }
            }
        }
        Stmt::Try {
            block,
            handler,
            finalizer,
        } => {
            for s in block {
                collect_var_declared_names(s, out);
            }
            if let Some(h) = handler {
                for s in &h.body {
                    collect_var_declared_names(s, out);
                }
            }
            if let Some(f) = finalizer {
                for s in f {
                    collect_var_declared_names(s, out);
                }
            }
        }
        _ => {}
    }
}

fn check_strict_binding_name(name: &str, strict: bool) -> Result<(), JsError> {
    if strict && (name == "eval" || name == "arguments") {
        return Err(JsError::syntax(format!(
            "Unexpected '{}' in strict mode",
            name
        )));
    }
    Ok(())
}

fn validate_stmt_list(
    stmts: &[Stmt],
    ctx: ValidationCtx<'_>,
    is_top_or_fn_body: bool,
    param_names: Option<&std::collections::HashSet<String>>,
) -> Result<(), JsError> {
    let mut lex_names = std::collections::HashSet::<String>::new();
    let mut annex_b_funcs = std::collections::HashSet::<String>::new();
    let mut var_names = Vec::<String>::new();

    for s in stmts {
        match s {
            Stmt::VarDecl { kind, decls } => {
                if *kind != VarKind::Var {
                    for d in decls {
                        let mut names = Vec::new();
                        collect_pattern_names(&d.pattern, &mut names);
                        for n in names {
                            if n == "let" {
                                return Err(JsError::syntax(
                                    "let is disallowed as a lexically bound name",
                                ));
                            }
                            check_strict_binding_name(&n, ctx.strict)?;
                            if !lex_names.insert(n.clone()) || annex_b_funcs.contains(&n) {
                                return Err(JsError::syntax(format!(
                                    "Identifier '{}' has already been declared",
                                    n
                                )));
                            }
                        }
                    }
                } else {
                    collect_var_declared_names(s, &mut var_names);
                }
            }
            Stmt::ClassDecl(cls) => {
                if let Some(ref n) = cls.name {
                    check_strict_binding_name(n, true)?;
                    if !lex_names.insert(n.clone()) || annex_b_funcs.contains(n) {
                        return Err(JsError::syntax(format!(
                            "Identifier '{}' has already been declared",
                            n
                        )));
                    }
                }
            }
            Stmt::FuncDecl(f) => {
                if let Some(ref n) = f.name {
                    let fn_strict = ctx.strict || f.strict;
                    check_strict_binding_name(n, fn_strict)?;
                    if !is_top_or_fn_body {
                        let is_plain = !f.is_async && !f.is_generator;
                        if ctx.strict || !is_plain {
                            if !lex_names.insert(n.clone()) || annex_b_funcs.contains(n) {
                                return Err(JsError::syntax(format!(
                                    "Identifier '{}' has already been declared",
                                    n
                                )));
                            }
                        } else {
                            if lex_names.contains(n) {
                                return Err(JsError::syntax(format!(
                                    "Identifier '{}' has already been declared",
                                    n
                                )));
                            }
                            annex_b_funcs.insert(n.clone());
                        }
                    }
                }
            }
            _ => {
                collect_var_declared_names(s, &mut var_names);
            }
        }
    }

    for vn in &var_names {
        check_strict_binding_name(vn, ctx.strict)?;
        if lex_names.contains(vn) || annex_b_funcs.contains(vn) {
            return Err(JsError::syntax(format!(
                "Identifier '{}' has already been declared",
                vn
            )));
        }
    }

    if let Some(pnames) = param_names {
        for ln in &lex_names {
            if pnames.contains(ln) {
                return Err(JsError::syntax(format!(
                    "Identifier '{}' has already been declared",
                    ln
                )));
            }
        }
        if !is_top_or_fn_body {
            for fn_name in &annex_b_funcs {
                if pnames.contains(fn_name) {
                    return Err(JsError::syntax(format!(
                        "Identifier '{}' has already been declared",
                        fn_name
                    )));
                }
            }
        }
    }

    for s in stmts {
        validate_stmt(s, ctx)?;
    }
    Ok(())
}

fn validate_stmt(stmt: &Stmt, ctx: ValidationCtx<'_>) -> Result<(), JsError> {
    match stmt {
        Stmt::Block(stmts) => validate_stmt_list(stmts, ctx, false, None),
        Stmt::VarDecl { decls, .. } => {
            for d in decls {
                validate_pattern(&d.pattern, ctx)?;
                if let Some(ref init) = d.init {
                    validate_expr(init, ctx)?;
                }
            }
            Ok(())
        }
        Stmt::Expr(e) | Stmt::Throw(e) => validate_expr(e, ctx),
        Stmt::Return(opt_e) => {
            if !ctx.in_function {
                return Err(JsError::syntax("Illegal return statement"));
            }
            if let Some(e) = opt_e {
                validate_expr(e, ctx)?;
            }
            Ok(())
        }
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Debugger | Stmt::Empty => {
            Ok(())
        }
        Stmt::If {
            test,
            consequent,
            alternate,
        } => {
            validate_expr(test, ctx)?;
            validate_stmt(consequent, ctx)?;
            if let Some(alt) = alternate {
                validate_stmt(alt, ctx)?;
            }
            Ok(())
        }
        Stmt::While { test, body } | Stmt::DoWhile { test, body } => {
            validate_expr(test, ctx)?;
            validate_stmt(body, ctx)
        }
        Stmt::With { object, body } => {
            if ctx.strict {
                return Err(JsError::syntax("Strict mode code may not include a with statement"));
            }
            validate_expr(object, ctx)?;
            validate_stmt(body, ctx)
        }
        Stmt::Labeled { body, .. } => validate_stmt(body, ctx),
        Stmt::For {
            init,
            test,
            update,
            body,
        } => {
            let mut head_lex = std::collections::HashSet::new();
            if let Some(fi) = init {
                match fi {
                    ForInit::VarDecl { kind, decls } => {
                        for d in decls {
                            validate_pattern(&d.pattern, ctx)?;
                            if let Some(ref e) = d.init {
                                validate_expr(e, ctx)?;
                            }
                            if *kind != VarKind::Var {
                                let mut names = Vec::new();
                                collect_pattern_names(&d.pattern, &mut names);
                                for n in names {
                                    if n == "let" {
                                        return Err(JsError::syntax(
                                            "let is disallowed as a lexically bound name",
                                        ));
                                    }
                                    check_strict_binding_name(&n, ctx.strict)?;
                                    if !head_lex.insert(n.clone()) {
                                        return Err(JsError::syntax(format!(
                                            "Identifier '{}' has already been declared",
                                            n
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    ForInit::Expr(e) => validate_expr(e, ctx)?,
                }
            }
            if !head_lex.is_empty() {
                let mut body_vars = Vec::new();
                collect_var_declared_names(body, &mut body_vars);
                for vn in body_vars {
                    if head_lex.contains(&vn) {
                        return Err(JsError::syntax(format!(
                            "Identifier '{}' has already been declared",
                            vn
                        )));
                    }
                }
            }
            if let Some(t) = test {
                validate_expr(t, ctx)?;
            }
            if let Some(u) = update {
                validate_expr(u, ctx)?;
            }
            validate_stmt(body, ctx)
        }
        Stmt::ForIn { left, right, body } | Stmt::ForOf { left, right, body, .. } => {
            let mut head_lex = std::collections::HashSet::new();
            match left {
                ForInOfLeft::VarDecl { kind, pattern } => {
                    validate_pattern(pattern, ctx)?;
                    if *kind != VarKind::Var {
                        let mut names = Vec::new();
                        collect_pattern_names(pattern, &mut names);
                        for n in names {
                            if n == "let" {
                                return Err(JsError::syntax(
                                    "let is disallowed as a lexically bound name",
                                ));
                            }
                            check_strict_binding_name(&n, ctx.strict)?;
                            if !head_lex.insert(n.clone()) {
                                return Err(JsError::syntax(format!(
                                    "Identifier '{}' has already been declared",
                                    n
                                )));
                            }
                        }
                    }
                }
                ForInOfLeft::Pattern(p) => validate_pattern(p, ctx)?,
            }
            if !head_lex.is_empty() {
                let mut body_vars = Vec::new();
                collect_var_declared_names(body, &mut body_vars);
                for vn in body_vars {
                    if head_lex.contains(&vn) {
                        return Err(JsError::syntax(format!(
                            "Identifier '{}' has already been declared",
                            vn
                        )));
                    }
                }
            }
            validate_expr(right, ctx)?;
            validate_stmt(body, ctx)
        }
        Stmt::Switch {
            discriminant,
            cases,
        } => {
            validate_expr(discriminant, ctx)?;
            let mut all_stmts = Vec::new();
            let mut default_count = 0usize;
            for c in cases {
                if let Some(ref t) = c.test {
                    validate_expr(t, ctx)?;
                } else {
                    default_count += 1;
                    if default_count > 1 {
                        return Err(JsError::syntax(
                            "More than one default clause in switch statement",
                        ));
                    }
                }
                all_stmts.extend(c.consequent.iter().cloned());
            }
            validate_stmt_list(&all_stmts, ctx, false, None)
        }
        Stmt::Try {
            block,
            handler,
            finalizer,
        } => {
            validate_stmt_list(block, ctx, false, None)?;
            if let Some(h) = handler {
                let mut catch_names = std::collections::HashSet::new();
                let mut is_simple_catch = false;
                if let Some(ref p) = h.param {
                    validate_pattern(p, ctx)?;
                    is_simple_catch = matches!(p, Pattern::Ident(_));
                    let mut names = Vec::new();
                    collect_pattern_names(p, &mut names);
                    for n in names {
                        check_strict_binding_name(&n, ctx.strict)?;
                        if !catch_names.insert(n.clone()) {
                            return Err(JsError::syntax(format!(
                                "Identifier '{}' has already been declared",
                                n
                            )));
                        }
                    }
                }
                if !is_simple_catch && !catch_names.is_empty() {
                    let mut body_vars = Vec::new();
                    for s in &h.body {
                        collect_var_declared_names(s, &mut body_vars);
                    }
                    for vn in body_vars {
                        if catch_names.contains(&vn) {
                            return Err(JsError::syntax(format!(
                                "Identifier '{}' has already been declared",
                                vn
                            )));
                        }
                    }
                }
                validate_stmt_list(
                    &h.body,
                    ctx,
                    false,
                    if catch_names.is_empty() {
                        None
                    } else {
                        Some(&catch_names)
                    },
                )?;
            }
            if let Some(fin) = finalizer {
                validate_stmt_list(fin, ctx, false, None)?;
            }
            Ok(())
        }
        Stmt::FuncDecl(f) => validate_function(f, ctx, false, false),
        Stmt::ClassDecl(c) => validate_class(c, ctx),
        Stmt::Import(_) | Stmt::Export(_) => Ok(()),
    }
}

fn validate_function(
    func: &FuncNode,
    outer_ctx: ValidationCtx<'_>,
    allow_super_call: bool,
    allow_super_prop: bool,
) -> Result<(), JsError> {
    let fn_strict = outer_ctx.strict || func.strict;
    if let Some(ref n) = func.name {
        check_strict_binding_name(n, fn_strict)?;
    }
    let has_simple_params = func
        .params
        .iter()
        .all(|p| !p.rest && matches!(p.pattern, Pattern::Ident(_)));
    let forbid_dupes = fn_strict || func.is_arrow || !has_simple_params || allow_super_prop;

    let mut param_names = std::collections::HashSet::new();
    let param_ctx = ValidationCtx {
        strict: fn_strict,
        allow_super_call: if func.is_arrow {
            outer_ctx.allow_super_call
        } else {
            allow_super_call
        },
        allow_super_prop: if func.is_arrow {
            outer_ctx.allow_super_prop
        } else {
            allow_super_prop
        },
        in_function: true,
        allow_new_target: if func.is_arrow {
            outer_ctx.allow_new_target
        } else {
            true
        },
        allow_private: outer_ctx.allow_private,
        in_static_block: if func.is_arrow {
            outer_ctx.in_static_block
        } else {
            false
        },
        forbid_yield_expr: func.is_generator || (func.is_arrow && outer_ctx.forbid_yield_expr),
        forbid_await_expr: func.is_async
            || (func.is_arrow && (outer_ctx.forbid_await_expr || outer_ctx.in_static_block)),
        forbid_arguments: if func.is_arrow {
            outer_ctx.forbid_arguments
        } else {
            false
        },
        privates: outer_ctx.privates,
    };

    for p in &func.params {
        validate_pattern(&p.pattern, param_ctx)?;
        let mut names = Vec::new();
        collect_pattern_names(&p.pattern, &mut names);
        for n in names {
            check_strict_binding_name(&n, fn_strict)?;
            if func.is_async && n == "await" {
                return Err(JsError::syntax("Cannot use 'await' as parameter name in async function"));
            }
            if func.is_generator && n == "yield" {
                return Err(JsError::syntax("Cannot use 'yield' as parameter name in generator"));
            }
            if forbid_dupes && !param_names.insert(n.clone()) {
                return Err(JsError::syntax(format!(
                    "Duplicate parameter name '{}' not allowed in this context",
                    n
                )));
            }
            param_names.insert(n);
        }
    }

    let body_ctx = ValidationCtx {
        strict: fn_strict,
        allow_super_call: if func.is_arrow {
            outer_ctx.allow_super_call
        } else {
            allow_super_call
        },
        allow_super_prop: if func.is_arrow {
            outer_ctx.allow_super_prop
        } else {
            allow_super_prop
        },
        in_function: true,
        allow_new_target: if func.is_arrow {
            outer_ctx.allow_new_target
        } else {
            true
        },
        allow_private: outer_ctx.allow_private,
        in_static_block: false,
        forbid_yield_expr: false,
        forbid_await_expr: false,
        forbid_arguments: if func.is_arrow {
            outer_ctx.forbid_arguments
        } else {
            false
        },
        privates: outer_ctx.privates,
    };

    match &func.body {
        FuncBody::Block(stmts) => validate_stmt_list(stmts, body_ctx, true, Some(&param_names)),
        FuncBody::Expr(e) => validate_expr(e, body_ctx),
    }
}

fn validate_class(cls: &ClassNode, outer_ctx: ValidationCtx<'_>) -> Result<(), JsError> {
    if let Some(ref n) = cls.name {
        check_strict_binding_name(n, true)?;
    }
    if let Some(ref sup) = cls.super_class {
        validate_expr(sup, ValidationCtx { strict: true, ..outer_ctx })?;
    }
    let has_super = cls.super_class.is_some();
    let mut has_ctor = false;
    let mut private_names: std::collections::HashMap<String, (bool, u8)> =
        std::collections::HashMap::new();
    let mut merged_privates = outer_ctx.privates.clone();
    for el in &cls.elements {
        match el {
            ClassElement::Method {
                key: PropKey::Private(pn),
                is_private: true,
                ..
            }
            | ClassElement::Field {
                key: PropKey::Private(pn),
                is_private: true,
                ..
            } => {
                merged_privates.insert(pn.clone());
            }
            _ => {}
        }
    }
    let cls_ctx = ValidationCtx {
        strict: true,
        allow_private: true,
        privates: &merged_privates,
        ..outer_ctx
    };
    for el in &cls.elements {
        match el {
            ClassElement::Method {
                key,
                kind,
                func,
                is_static,
                is_private,
            } => {
                if *kind == MethodKind::Get && !func.params.is_empty() {
                    return Err(JsError::syntax("Getter must not have any formal parameters"));
                }
                if *kind == MethodKind::Set
                    && (func.params.len() != 1 || func.params[0].rest)
                {
                    return Err(JsError::syntax(
                        "Setter must have exactly one formal parameter",
                    ));
                }
                if *is_private {
                    if let PropKey::Private(pn) = key {
                        if pn == "constructor" {
                            return Err(JsError::syntax(
                                "Classes may not have a private element named #constructor",
                            ));
                        }
                        let mask: u8 = match kind {
                            MethodKind::Get => 2,
                            MethodKind::Set => 4,
                            _ => 1,
                        };
                        if let Some(&(prev_static, prev_mask)) = private_names.get(pn) {
                            let ok_pair = prev_static == *is_static
                                && ((prev_mask == 2 && mask == 4) || (prev_mask == 4 && mask == 2));
                            if !ok_pair {
                                return Err(JsError::syntax(format!(
                                    "Identifier '#{}' has already been declared",
                                    pn
                                )));
                            }
                            private_names.insert(pn.clone(), (prev_static, prev_mask | mask));
                        } else {
                            private_names.insert(pn.clone(), (*is_static, mask));
                        }
                    }
                } else {
                    if *is_static
                        && matches!(key, PropKey::Ident(s) | PropKey::String(s) if s == "prototype")
                    {
                        return Err(JsError::syntax(
                            "Classes may not have a static property named prototype",
                        ));
                    }
                    if !*is_static
                        && matches!(key, PropKey::Ident(s) | PropKey::String(s) if s == "constructor")
                    {
                        if *kind == MethodKind::Get || *kind == MethodKind::Set {
                            return Err(JsError::syntax(
                                "Class constructor may not be an accessor",
                            ));
                        }
                        if func.is_async || func.is_generator {
                            return Err(JsError::syntax(
                                "Class constructor may not be async or generator",
                            ));
                        }
                        if has_ctor {
                            return Err(JsError::syntax(
                                "A class may only have one constructor",
                            ));
                        }
                        has_ctor = true;
                    }
                }
                if let PropKey::Computed(e) = key {
                    validate_expr(e, cls_ctx)?;
                }
                let is_derived_ctor = *kind == MethodKind::Constructor && !*is_static && has_super;
                validate_function(func, cls_ctx, is_derived_ctor, true)?;
            }
            ClassElement::Field {
                key,
                init,
                is_static,
                is_private,
            } => {
                if *is_private {
                    if let PropKey::Private(pn) = key {
                        if pn == "constructor" {
                            return Err(JsError::syntax(
                                "Classes may not have a private element named #constructor",
                            ));
                        }
                        if private_names.contains_key(pn) {
                            return Err(JsError::syntax(format!(
                                "Identifier '#{}' has already been declared",
                                pn
                            )));
                        }
                        private_names.insert(pn.clone(), (*is_static, 1));
                    }
                } else {
                    if matches!(key, PropKey::Ident(s) | PropKey::String(s) if s == "constructor") {
                        return Err(JsError::syntax(
                            "Classes may not have a field named constructor",
                        ));
                    }
                    if *is_static
                        && matches!(key, PropKey::Ident(s) | PropKey::String(s) if s == "prototype")
                    {
                        return Err(JsError::syntax(
                            "Classes may not have a static property named prototype",
                        ));
                    }
                }
                if let PropKey::Computed(e) = key {
                    validate_expr(e, cls_ctx)?;
                }
                if let Some(e) = init {
                    let field_ctx = ValidationCtx {
                        strict: true,
                        allow_super_call: false,
                        allow_super_prop: true,
                        in_function: false,
                        allow_new_target: true,
                        allow_private: true,
                        in_static_block: false,
                        forbid_yield_expr: false,
                        forbid_await_expr: false,
                        forbid_arguments: true,
                        privates: &merged_privates,
                    };
                    validate_expr(e, field_ctx)?;
                }
            }
            ClassElement::StaticBlock(stmts) => {
                let sb_ctx = ValidationCtx {
                    strict: true,
                    allow_super_call: false,
                    allow_super_prop: true,
                    in_function: false,
                    allow_new_target: true,
                    allow_private: true,
                    in_static_block: true,
                    forbid_yield_expr: true,
                    forbid_await_expr: true,
                    forbid_arguments: true,
                    privates: &merged_privates,
                };
                validate_stmt_list(stmts, sb_ctx, true, None)?;
            }
        }
    }
    Ok(())
}

fn validate_pattern(pat: &Pattern, ctx: ValidationCtx<'_>) -> Result<(), JsError> {
    match pat {
        Pattern::Ident(n) => {
            if is_reserved_word(n, ctx.strict) {
                return Err(JsError::syntax(format!("Unexpected reserved word '{}'", n)));
            }
            check_strict_binding_name(n, ctx.strict)?;
            if ctx.in_static_block && n == "await" {
                return Err(JsError::syntax("Cannot use 'await' in class static block"));
            }
            Ok(())
        }
        Pattern::Array { elements, rest } => {
            for el in elements.iter().flatten() {
                validate_pattern(el, ctx)?;
            }
            if let Some(r) = rest {
                validate_pattern(r, ctx)?;
            }
            Ok(())
        }
        Pattern::Object { props, rest } => {
            for p in props {
                if let PropKey::Computed(e) = &p.key {
                    validate_expr(e, ctx)?;
                }
                validate_pattern(&p.value, ctx)?;
            }
            if let Some(r) = rest {
                validate_pattern(r, ctx)?;
            }
            Ok(())
        }
        Pattern::Assign { left, right } => {
            validate_pattern(left, ctx)?;
            validate_expr(right, ctx)
        }
        Pattern::Member(e) => validate_expr(e, ctx),
    }
}

fn validate_expr(expr: &Expr, ctx: ValidationCtx<'_>) -> Result<(), JsError> {
    match expr {
        Expr::Ident(n) => {
            if is_reserved_word(n, ctx.strict) {
                return Err(JsError::syntax(format!("Unexpected reserved word '{}'", n)));
            }
            if ctx.in_static_block && n == "await" {
                return Err(JsError::syntax("Cannot use 'await' in class static block"));
            }
            if ctx.forbid_arguments && n == "arguments" {
                return Err(JsError::syntax(
                    "'arguments' is not allowed in class field initializer or static block",
                ));
            }
            Ok(())
        }
        Expr::Yield { arg, .. } => {
            if ctx.forbid_yield_expr {
                return Err(JsError::syntax("Yield expression not allowed in parameters"));
            }
            if let Some(a) = arg {
                validate_expr(a, ctx)?;
            }
            Ok(())
        }
        Expr::Await(a) => {
            if ctx.forbid_await_expr {
                return Err(JsError::syntax("Await expression not allowed here"));
            }
            validate_expr(a, ctx)
        }
        Expr::Call { callee, args, .. } => {
            if matches!(callee.as_ref(), Expr::Super) {
                if !ctx.allow_super_call {
                    return Err(JsError::syntax("super() call unexpected here"));
                }
            } else {
                validate_expr(callee, ctx)?;
            }
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => validate_expr(e, ctx)?,
                }
            }
            Ok(())
        }
        Expr::Member {
            object, property, ..
        } => {
            if matches!(object.as_ref(), Expr::Super) {
                if !ctx.allow_super_prop {
                    return Err(JsError::syntax("super property access unexpected here"));
                }
            } else {
                validate_expr(object, ctx)?;
            }
            match property {
                MemberProp::Computed(e) => validate_expr(e, ctx)?,
                MemberProp::Private(pn) if !ctx.allow_private || !ctx.privates.contains(pn) => {
                    return Err(JsError::syntax(format!(
                        "Private field '#{}' must be declared in an enclosing class",
                        pn
                    )));
                }
                _ => {}
            }
            Ok(())
        }
        Expr::Super => Err(JsError::syntax("'super' keyword unexpected here")),
        Expr::Function(f) => validate_function(f, ctx, false, false),
        Expr::Arrow(f) => validate_function(f, ctx, ctx.allow_super_call, ctx.allow_super_prop),
        Expr::Class(c) => validate_class(c, ctx),
        Expr::Array(elems) => {
            for el in elems {
                match el {
                    ArrayElem::Hole => {}
                    ArrayElem::Expr(e) | ArrayElem::Spread(e, _) => validate_expr(e, ctx)?,
                }
            }
            Ok(())
        }
        Expr::Object(props) => {
            let mut has_proto = false;
            for p in props {
                match p {
                    ObjectProp::Property {
                        key,
                        value,
                        shorthand,
                        is_cover_init,
                    } => {
                        if *is_cover_init {
                            return Err(JsError::syntax(
                                "Invalid shorthand property initializer",
                            ));
                        }
                        if !*shorthand {
                            let is_proto = match key {
                                PropKey::Ident(s) | PropKey::String(s) => s == "__proto__",
                                _ => false,
                            };
                            if is_proto {
                                if has_proto {
                                    return Err(JsError::syntax(
                                        "Duplicate __proto__ fields are not allowed in object literals",
                                    ));
                                }
                                has_proto = true;
                            }
                        }
                        if let PropKey::Computed(e) = key {
                            validate_expr(e, ctx)?;
                        }
                        validate_expr(value, ctx)?;
                    }
                    ObjectProp::Method { key, kind, func } => {
                        if *kind == MethodKind::Get && !func.params.is_empty() {
                            return Err(JsError::syntax(
                                "Getter must not have any formal parameters",
                            ));
                        }
                        if *kind == MethodKind::Set
                            && (func.params.len() != 1 || func.params[0].rest)
                        {
                            return Err(JsError::syntax(
                                "Setter must have exactly one formal parameter",
                            ));
                        }
                        if let PropKey::Computed(e) = key {
                            validate_expr(e, ctx)?;
                        }
                        validate_function(func, ctx, false, true)?;
                    }
                    ObjectProp::Spread(e, _) => validate_expr(e, ctx)?,
                }
            }
            Ok(())
        }
        Expr::Template { tag, exprs, .. } => {
            if let Some(t) = tag {
                validate_expr(t, ctx)?;
            }
            for e in exprs {
                validate_expr(e, ctx)?;
            }
            Ok(())
        }
        Expr::DynamicImport { spec, options, .. } => {
            validate_expr(spec, ctx)?;
            if let Some(opt) = options {
                validate_expr(opt, ctx)?;
            }
            Ok(())
        }
        Expr::OptionalChain(e)
        | Expr::Unary { arg: e, .. } => validate_expr(e, ctx),
        Expr::Update { arg: e, .. } => {
            if let Expr::Ident(n) = e.as_ref() {
                check_strict_binding_name(n, ctx.strict)?;
            }
            validate_expr(e, ctx)
        }
        Expr::PrivateIn { name, right: e } => {
            if !ctx.allow_private || !ctx.privates.contains(name) {
                return Err(JsError::syntax(format!(
                    "Private field '#{}' must be declared in an enclosing class",
                    name
                )));
            }
            validate_expr(e, ctx)
        }
        Expr::New { callee, args } => {
            validate_expr(callee, ctx)?;
            for a in args {
                match a {
                    CallArg::Expr(e) | CallArg::Spread(e) => validate_expr(e, ctx)?,
                }
            }
            Ok(())
        }
        Expr::Binary { left, right, .. } | Expr::Logical { left, right, .. } => {
            validate_expr(left, ctx)?;
            validate_expr(right, ctx)
        }
        Expr::Conditional {
            test,
            consequent,
            alternate,
        } => {
            validate_expr(test, ctx)?;
            validate_expr(consequent, ctx)?;
            validate_expr(alternate, ctx)
        }
        Expr::Assign { left, right, .. } => {
            match left {
                AssignTarget::Ident(n) => {
                    check_strict_binding_name(n, ctx.strict)?;
                }
                AssignTarget::Member { object, property } => {
                    if matches!(object.as_ref(), Expr::Super) {
                        if !ctx.allow_super_prop {
                            return Err(JsError::syntax("super property access unexpected here"));
                        }
                    } else {
                        validate_expr(object, ctx)?;
                    }
                    match property {
                        MemberProp::Computed(e) => validate_expr(e, ctx)?,
                        MemberProp::Private(pn)
                            if !ctx.allow_private || !ctx.privates.contains(pn) =>
                        {
                            return Err(JsError::syntax(format!(
                                "Private field '#{}' must be declared in an enclosing class",
                                pn
                            )));
                        }
                        _ => {}
                    }
                }
                AssignTarget::Pattern(p) => validate_pattern(p, ctx)?,
            }
            validate_expr(right, ctx)
        }
        Expr::Sequence(es) => {
            for e in es {
                validate_expr(e, ctx)?;
            }
            Ok(())
        }
        Expr::NewTarget => {
            if !ctx.allow_new_target {
                return Err(JsError::syntax("new.target expression is not allowed here"));
            }
            Ok(())
        }
        Expr::Undefined
        | Expr::Null
        | Expr::Bool(_)
        | Expr::Number(_)
        | Expr::BigInt(_)
        | Expr::String(_)
        | Expr::RegExp { .. }
        | Expr::This
        | Expr::ImportMeta => Ok(()),
    }
}

fn validate_module_early_errors(program: &Program) -> Result<(), JsError> {
    let mut lex_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut var_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut exported_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut local_export_refs: Vec<String> = Vec::new();

    let add_lex = |name: &str, lex: &mut std::collections::HashSet<String>| -> Result<(), JsError> {
        if name == "eval" || name == "arguments" {
            return Err(JsError::syntax(format!(
                "Unexpected '{}' binding in module",
                name
            )));
        }
        if !lex.insert(name.to_string()) {
            return Err(JsError::syntax(format!(
                "Identifier '{}' has already been declared",
                name
            )));
        }
        Ok(())
    };

    let add_export =
        |name: &str, exps: &mut std::collections::HashSet<String>| -> Result<(), JsError> {
            if !exps.insert(name.to_string()) {
                return Err(JsError::syntax(format!(
                    "Duplicate export of '{}'",
                    name
                )));
            }
            Ok(())
        };

    for stmt in &program.body {
        match stmt {
            Stmt::Import(imp) => {
                for sp in &imp.specifiers {
                    let local = match sp {
                        ImportSpecifier::Default(n)
                        | ImportSpecifier::Namespace(n)
                        | ImportSpecifier::Named { local: n, .. } => n,
                    };
                    add_lex(local, &mut lex_names)?;
                }
            }
            Stmt::Export(exp) => match exp {
                ExportDecl::DefaultExpr(_) => {
                    add_export("default", &mut exported_names)?;
                }
                ExportDecl::DefaultFunc(f) => {
                    add_export("default", &mut exported_names)?;
                    if let Some(ref n) = f.name {
                        add_lex(n, &mut lex_names)?;
                    }
                }
                ExportDecl::DefaultClass(c) => {
                    add_export("default", &mut exported_names)?;
                    if let Some(ref n) = c.name {
                        add_lex(n, &mut lex_names)?;
                    }
                }
                ExportDecl::Decl(inner) => match inner.as_ref() {
                    Stmt::VarDecl {
                        kind: VarKind::Var,
                        decls,
                    } => {
                        let mut names = Vec::new();
                        for d in decls {
                            collect_pat_bound_names(&d.pattern, &mut names);
                        }
                        for n in names {
                            var_names.insert(n.clone());
                            add_export(&n, &mut exported_names)?;
                        }
                    }
                    Stmt::VarDecl { decls, .. } => {
                        let mut names = Vec::new();
                        for d in decls {
                            collect_pat_bound_names(&d.pattern, &mut names);
                        }
                        for n in names {
                            add_lex(&n, &mut lex_names)?;
                            add_export(&n, &mut exported_names)?;
                        }
                    }
                    Stmt::FuncDecl(f) => {
                        if let Some(ref n) = f.name {
                            add_lex(n, &mut lex_names)?;
                            add_export(n, &mut exported_names)?;
                        }
                    }
                    Stmt::ClassDecl(c) => {
                        if let Some(ref n) = c.name {
                            add_lex(n, &mut lex_names)?;
                            add_export(n, &mut exported_names)?;
                        }
                    }
                    _ => {}
                },
                ExportDecl::Named {
                    specifiers, source, ..
                } => {
                    for sp in specifiers {
                        add_export(&sp.exported, &mut exported_names)?;
                        if source.is_none() {
                            local_export_refs.push(sp.local.clone());
                        }
                    }
                }
                ExportDecl::All {
                    exported: Some(name),
                    ..
                } => {
                    add_export(name, &mut exported_names)?;
                }
                ExportDecl::All { exported: None, .. } => {}
            },
            Stmt::VarDecl {
                kind: VarKind::Var, ..
            } => {
                collect_stmt_var_names(stmt, &mut var_names);
            }
            Stmt::VarDecl { decls, .. } => {
                let mut names = Vec::new();
                for d in decls {
                    collect_pat_bound_names(&d.pattern, &mut names);
                }
                for n in names {
                    add_lex(&n, &mut lex_names)?;
                }
            }
            Stmt::FuncDecl(f) => {
                if let Some(ref n) = f.name {
                    add_lex(n, &mut lex_names)?;
                }
            }
            Stmt::ClassDecl(c) => {
                if let Some(ref n) = c.name {
                    add_lex(n, &mut lex_names)?;
                }
            }
            other => {
                collect_stmt_var_names(other, &mut var_names);
            }
        }
    }

    for vn in &var_names {
        if lex_names.contains(vn) {
            return Err(JsError::syntax(format!(
                "Identifier '{}' has already been declared",
                vn
            )));
        }
    }
    for local in &local_export_refs {
        if !lex_names.contains(local) && !var_names.contains(local) {
            return Err(JsError::syntax(format!(
                "Export '{}' is not defined in module",
                local
            )));
        }
    }
    let mut active_labels = Vec::new();
    for stmt in &program.body {
        check_duplicate_labels(stmt, &mut active_labels)?;
    }
    Ok(())
}

fn check_duplicate_labels(stmt: &Stmt, active: &mut Vec<String>) -> Result<(), JsError> {
    match stmt {
        Stmt::Labeled { label, body } => {
            if active.contains(label) {
                return Err(JsError::syntax(format!(
                    "Duplicate label '{}'",
                    label
                )));
            }
            active.push(label.clone());
            check_duplicate_labels(body, active)?;
            active.pop();
        }
        Stmt::Block(stmts) => {
            for s in stmts {
                check_duplicate_labels(s, active)?;
            }
        }
        Stmt::If {
            consequent,
            alternate,
            ..
        } => {
            check_duplicate_labels(consequent, active)?;
            if let Some(alt) = alternate {
                check_duplicate_labels(alt, active)?;
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::For { body, .. }
        | Stmt::ForIn { body, .. }
        | Stmt::ForOf { body, .. }
        | Stmt::With { body, .. } => check_duplicate_labels(body, active)?,
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &c.consequent {
                    check_duplicate_labels(s, active)?;
                }
            }
        }
        Stmt::Try {
            block,
            handler,
            finalizer,
        } => {
            for s in block {
                check_duplicate_labels(s, active)?;
            }
            if let Some(h) = handler {
                for s in &h.body {
                    check_duplicate_labels(s, active)?;
                }
            }
            if let Some(f) = finalizer {
                for s in f {
                    check_duplicate_labels(s, active)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_pat_bound_names(pat: &Pattern, out: &mut Vec<String>) {
    match pat {
        Pattern::Ident(n) => out.push(n.clone()),
        Pattern::Assign { left, .. } => collect_pat_bound_names(left, out),
        Pattern::Array { elements, rest } => {
            for el in elements.iter().flatten() {
                collect_pat_bound_names(el, out);
            }
            if let Some(r) = rest {
                collect_pat_bound_names(r, out);
            }
        }
        Pattern::Object { props, rest } => {
            for p in props {
                collect_pat_bound_names(&p.value, out);
            }
            if let Some(r) = rest {
                collect_pat_bound_names(r, out);
            }
        }
        Pattern::Member(_) => {}
    }
}

fn collect_stmt_var_names(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match stmt {
        Stmt::VarDecl {
            kind: VarKind::Var,
            decls,
        } => {
            let mut names = Vec::new();
            for d in decls {
                collect_pat_bound_names(&d.pattern, &mut names);
            }
            for n in names {
                out.insert(n);
            }
        }
        Stmt::Block(stmts) => {
            for s in stmts {
                collect_stmt_var_names(s, out);
            }
        }
        Stmt::If {
            consequent,
            alternate,
            ..
        } => {
            collect_stmt_var_names(consequent, out);
            if let Some(alt) = alternate {
                collect_stmt_var_names(alt, out);
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Labeled { body, .. }
        | Stmt::With { body, .. } => collect_stmt_var_names(body, out),
        Stmt::For { init, body, .. } => {
            if let Some(ForInit::VarDecl {
                kind: VarKind::Var,
                decls,
            }) = init
            {
                let mut names = Vec::new();
                for d in decls {
                    collect_pat_bound_names(&d.pattern, &mut names);
                }
                for n in names {
                    out.insert(n);
                }
            }
            collect_stmt_var_names(body, out);
        }
        Stmt::ForIn { left, body, .. } | Stmt::ForOf { left, body, .. } => {
            if let ForInOfLeft::VarDecl {
                kind: VarKind::Var,
                pattern,
            } = left
            {
                let mut names = Vec::new();
                collect_pat_bound_names(pattern, &mut names);
                for n in names {
                    out.insert(n);
                }
            }
            collect_stmt_var_names(body, out);
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &c.consequent {
                    collect_stmt_var_names(s, out);
                }
            }
        }
        Stmt::Try {
            block,
            handler,
            finalizer,
        } => {
            for s in block {
                collect_stmt_var_names(s, out);
            }
            if let Some(h) = handler {
                for s in &h.body {
                    collect_stmt_var_names(s, out);
                }
            }
            if let Some(f) = finalizer {
                for s in f {
                    collect_stmt_var_names(s, out);
                }
            }
        }
        _ => {}
    }
}
