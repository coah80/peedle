use crate::ast::*;
use crate::error::JsError;
use crate::lexer::{Lexer, Token, TokenKind};
use std::rc::Rc;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    cur: Token,
    strict: bool,
    is_module: bool,
    in_async: bool,
    in_generator: bool,
    in_function: bool,
    in_class: bool,
}

impl<'a> Parser<'a> {
    pub fn parse_script(src: &'a str) -> Result<Program, JsError> {
        Self::parse_inner(src, false, false)
    }

    pub fn parse_module(src: &'a str) -> Result<Program, JsError> {
        Self::parse_inner(src, true, true)
    }

    pub fn parse_script_with_strict(src: &'a str, strict: bool) -> Result<Program, JsError> {
        Self::parse_inner(src, false, strict)
    }

    fn parse_inner(src: &'a str, is_module: bool, initial_strict: bool) -> Result<Program, JsError> {
        let mut lexer = Lexer::new(src);
        let cur = lexer.next_token(true)?;
        let mut parser = Parser {
            lexer,
            cur,
            strict: initial_strict || is_module,
            is_module,
            in_async: is_module,
            in_generator: false,
            in_function: false,
            in_class: false,
        };
        let mut body = Vec::new();
        let mut checking_directives = true;
        while parser.cur.kind != TokenKind::Eof {
            if checking_directives {
                if let TokenKind::String(ref s, has_escape) = parser.cur.kind {
                    if s == "use strict" && !has_escape {
                        let stmt = parser.parse_module_item()?;
                        if matches!(stmt, Stmt::Expr(Expr::String(ref st)) if st == "use strict") {
                            parser.strict = true;
                        } else {
                            checking_directives = false;
                        }
                        body.push(stmt);
                        continue;
                    }
                }
            }
            let stmt = parser.parse_module_item()?;
            if checking_directives && !matches!(stmt, Stmt::Expr(Expr::String(_))) {
                checking_directives = false;
            }
            body.push(stmt);
        }
        Ok(Program {
            body,
            is_module,
            strict: parser.strict,
        })
    }

    fn advance(&mut self, allow_regexp: bool) -> Result<Token, JsError> {
        let prev = std::mem::replace(&mut self.cur, self.lexer.next_token(allow_regexp)?);
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

    fn parse_import(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let mut specifiers = Vec::new();
        if let TokenKind::String(src, _) = self.cur.kind.clone() {
            self.advance(false)?;
            self.eat_semi()?;
            return Ok(Stmt::Import(ImportDecl {
                specifiers,
                source: src,
            }));
        }
        if !matches!(self.cur.kind, TokenKind::Star | TokenKind::LBrace) {
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
        self.eat_semi()?;
        Ok(Stmt::Import(ImportDecl { specifiers, source }))
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
                let imported = self.parse_module_export_name()?;
                let local = if self.is_ident_word("as") {
                    self.advance(true)?;
                    self.parse_binding_ident()?
                } else {
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
            self.eat_semi()?;
            return Ok(Stmt::Export(ExportDecl::All { exported, source }));
        }
        if self.eat(&TokenKind::LBrace, true)? {
            let mut specifiers = Vec::new();
            while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
                let local = self.parse_module_export_name()?;
                let exported = if self.is_ident_word("as") {
                    self.advance(true)?;
                    self.parse_module_export_name()?
                } else {
                    local.clone()
                };
                specifiers.push(ExportSpecifier { local, exported });
                if !self.eat(&TokenKind::Comma, true)? {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace, true)?;
            let source = if self.is_ident_word("from") {
                self.advance(true)?;
                match self.cur.kind.clone() {
                    TokenKind::String(s, _) => {
                        self.advance(false)?;
                        Some(s)
                    }
                    _ => return Err(JsError::syntax("Expected string after 'from'")),
                }
            } else {
                None
            };
            self.eat_semi()?;
            return Ok(Stmt::Export(ExportDecl::Named { specifiers, source }));
        }
        let decl = self.parse_stmt()?;
        Ok(Stmt::Export(ExportDecl::Decl(Box::new(decl))))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, JsError> {
        match &self.cur.kind {
            TokenKind::LBrace => self.parse_block(),
            TokenKind::Semi => {
                self.advance(true)?;
                Ok(Stmt::Empty)
            }
            TokenKind::KwVar => self.parse_var_decl(VarKind::Var, true),
            TokenKind::KwConst => self.parse_var_decl(VarKind::Const, true),
            TokenKind::Ident(s, false) if s == "let" => {
                if self.is_let_decl_start()? {
                    self.parse_var_decl(VarKind::Let, true)
                } else {
                    self.parse_expr_or_label_stmt()
                }
            }
            TokenKind::KwFunction => {
                let func = self.parse_function(true, true)?;
                Ok(Stmt::FuncDecl(func))
            }
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(true)?;
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

    fn is_let_decl_start(&self) -> Result<bool, JsError> {
        let next = self.peek_next(true)?;
        if next.preceded_by_newline {
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
            let init = if self.eat(&TokenKind::Eq, true)? {
                Some(self.parse_assign_expr(allow_in)?)
            } else {
                None
            };
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
        let consequent = Box::new(self.parse_stmt()?);
        let alternate = if self.eat(&TokenKind::KwElse, true)? {
            Some(Box::new(self.parse_stmt()?))
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
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::While { test, body })
    }

    fn parse_do_while(&mut self) -> Result<Stmt, JsError> {
        self.advance(true)?;
        let body = Box::new(self.parse_stmt()?);
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
            _ => None,
        };

        if let Some(kind) = var_kind {
            self.advance(true)?;
            let first_pat = self.parse_pattern()?;
            if self.cur.kind == TokenKind::KwIn {
                if is_await {
                    return Err(JsError::syntax("for await cannot be used with in"));
                }
                self.advance(true)?;
                let right = self.parse_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_stmt()?);
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
                self.advance(true)?;
                let right = self.parse_assign_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_stmt()?);
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
            let mut decls = vec![VarDeclarator {
                pattern: first_pat,
                init: first_init,
            }];
            while self.eat(&TokenKind::Comma, true)? {
                let pat = self.parse_pattern()?;
                let init = if self.eat(&TokenKind::Eq, true)? {
                    Some(self.parse_assign_expr(false)?)
                } else {
                    None
                };
                decls.push(VarDeclarator { pattern: pat, init });
            }
            if self.cur.kind == TokenKind::KwIn && decls.len() == 1 {
                self.advance(true)?;
                let right = self.parse_expr(true)?;
                self.expect(&TokenKind::RParen, true)?;
                let body = Box::new(self.parse_stmt()?);
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
            let body = Box::new(self.parse_stmt()?);
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
            let body = Box::new(self.parse_stmt()?);
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
        let body = Box::new(self.parse_stmt()?);
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
                consequent.push(self.parse_stmt()?);
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
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::With { object, body })
    }

    fn parse_expr_or_label_stmt(&mut self) -> Result<Stmt, JsError> {
        if let Some(label) = self.peek_label_ident()? {
            self.advance(true)?;
            self.advance(true)?;
            let body = Box::new(self.parse_stmt()?);
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
        let next = self.peek_next(true)?;
        if next.kind == TokenKind::Colon {
            Ok(Some(name))
        } else {
            Ok(None)
        }
    }

    fn parse_function(&mut self, require_name: bool, _is_decl: bool) -> Result<Rc<FuncNode>, JsError> {
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
        let (body, strict, has_use_strict) = self.parse_func_body_block_info()?;
        validate_func_params(&params, strict, has_use_strict, false)?;
        let func_end = self.cur.start.min(self.lexer.source().len());

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
            strict,
            source: self.lexer.source()[func_start..func_end].trim().to_string(),
        }))
    }

    fn parse_func_body_block(&mut self) -> Result<(Vec<Stmt>, bool), JsError> {
        let (stmts, strict, _) = self.parse_func_body_block_info()?;
        Ok((stmts, strict))
    }

    fn parse_func_body_block_info(&mut self) -> Result<(Vec<Stmt>, bool, bool), JsError> {
        self.expect(&TokenKind::LBrace, true)?;
        let mut stmts = Vec::new();
        let mut checking = true;
        let mut strict = self.strict;
        let mut has_use_strict = false;
        let prev_strict = self.strict;
        while self.cur.kind != TokenKind::RBrace && self.cur.kind != TokenKind::Eof {
            if checking {
                if let TokenKind::String(ref s, has_escape) = self.cur.kind {
                    if s == "use strict" && !has_escape {
                        let st = self.parse_stmt()?;
                        if matches!(st, Stmt::Expr(Expr::String(ref v)) if v == "use strict") {
                            strict = true;
                            has_use_strict = true;
                            self.strict = true;
                        } else {
                            checking = false;
                        }
                        stmts.push(st);
                        continue;
                    }
                }
            }
            let st = self.parse_stmt()?;
            if checking && !matches!(st, Stmt::Expr(Expr::String(_))) {
                checking = false;
            }
            stmts.push(st);
        }
        self.expect(&TokenKind::RBrace, true)?;
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
            let func = self.parse_method_func(is_async, is_generator, None)?;
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
        is_async: bool,
        is_generator: bool,
        name: Option<String>,
    ) -> Result<Rc<FuncNode>, JsError> {
        let start = self.cur.start;
        let prev_async = self.in_async;
        let prev_gen = self.in_generator;
        let prev_fn = self.in_function;
        self.in_async = is_async;
        self.in_generator = is_generator;
        self.in_function = true;

        self.expect(&TokenKind::LParen, true)?;
        let params = self.parse_param_list()?;
        self.expect(&TokenKind::RParen, true)?;
        let (body, strict) = self.parse_func_body_block()?;
        let end = self.cur.start.min(self.lexer.source().len());

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
            let next = self.peek_next(true)?;
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
            let (stmts, st, hs) = self.parse_func_body_block_info()?;
            (FuncBody::Block(stmts), st, hs)
        } else {
            let expr = self.parse_assign_expr(allow_in)?;
            (FuncBody::Expr(Box::new(expr)), self.strict, false)
        };
        validate_func_params(&params, strict, has_use_strict, true)?;
        let end = self.cur.start.min(self.lexer.source().len());

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

        loop {
            let Some((prec, right_assoc)) = binary_prec(&self.cur.kind, allow_in) else {
                break;
            };
            if prec < min_prec {
                break;
            }
            let op_tok = self.advance(true)?;
            let next_min = if right_assoc { prec } else { prec + 1 };
            let right = self.parse_binary_expr(next_min, allow_in)?;
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
            TokenKind::KwImport => {
                self.advance(true)?;
                if self.eat(&TokenKind::Dot, true)? {
                    let prop = self.parse_ident_name_after_dot()?;
                    if prop == "meta" {
                        return Ok(Expr::ImportMeta);
                    }
                    return Err(JsError::syntax(format!(
                        "Unknown meta property import.{}",
                        prop
                    )));
                }
                self.expect(&TokenKind::LParen, true)?;
                let spec = self.parse_assign_expr(true)?;
                let _ = self.eat(&TokenKind::Comma, true);
                if self.cur.kind != TokenKind::RParen {
                    let _ = self.parse_assign_expr(true)?;
                    let _ = self.eat(&TokenKind::Comma, true);
                }
                self.expect(&TokenKind::RParen, false)?;
                Ok(Expr::DynamicImport(Box::new(spec)))
            }
            TokenKind::Ident(s, false) if s == "async" => {
                let next = self.peek_next(true)?;
                if !next.preceded_by_newline && next.kind == TokenKind::KwFunction {
                    let func = self.parse_function(false, false)?;
                    Ok(Expr::Function(func))
                } else {
                    self.advance(false)?;
                    Ok(Expr::Ident(s))
                }
            }
            TokenKind::Ident(s, escaped) => {
                if escaped && ((self.in_async && s == "await") || (self.in_generator && s == "yield")) {
                    return Err(JsError::syntax(format!("Escaped keyword '{}' not allowed here", s)));
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
            if self.eat(&TokenKind::Ellipsis, true)? {
                elems.push(ArrayElem::Spread(self.parse_assign_expr(true)?));
            } else {
                elems.push(ArrayElem::Expr(self.parse_assign_expr(true)?));
            }
            if !self.eat(&TokenKind::Comma, true)? {
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
                props.push(ObjectProp::Spread(self.parse_assign_expr(true)?));
                if !self.eat(&TokenKind::Comma, true)? {
                    break;
                }
                continue;
            }

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
                let func = self.parse_method_func(is_async, is_generator, name)?;
                props.push(ObjectProp::Method { key, kind, func });
            } else if self.eat(&TokenKind::Colon, true)? {
                let value = self.parse_assign_expr(true)?;
                props.push(ObjectProp::Property {
                    key,
                    value,
                    shorthand: false,
                });
            } else if let PropKey::Ident(ref name) = key {
                let value = if self.eat(&TokenKind::Eq, true)? {
                    let right = self.parse_assign_expr(true)?;
                    Expr::Assign {
                        op: AssignOp::Assign,
                        left: AssignTarget::Ident(name.clone()),
                        right: Box::new(right),
                    }
                } else {
                    Expr::Ident(name.clone())
                };
                props.push(ObjectProp::Property {
                    key,
                    value,
                    shorthand: true,
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
                if escaped && ((self.in_async && s == "await") || (self.in_generator && s == "yield")) {
                    return Err(JsError::syntax(format!("Escaped keyword '{}' not allowed as binding", s)));
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
                    ArrayElem::Spread(e) => {
                        if i + 1 != len {
                            return Err(JsError::syntax(
                                "Rest element must be last in destructuring pattern",
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
                    ObjectProp::Spread(e) => {
                        if i + 1 != len {
                            return Err(JsError::syntax(
                                "Rest element must be last in object pattern",
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
