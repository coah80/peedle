use crate::ast::*;
use crate::bytecode::*;
use crate::error::JsError;
use std::rc::Rc;

struct LoopContext {
    label: Option<String>,
    is_loop: bool,
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    break_env_depth: u32,
    break_stack_depth: u32,
    break_handler_depth: u32,
    cont_env_depth: u32,
    cont_stack_depth: u32,
    cont_handler_depth: u32,
}

pub struct Compiler {
    chunk: Chunk,
    loops: Vec<LoopContext>,
    env_depth: u32,
    stack_depth: u32,
    handler_depth: u32,
    track_completion: bool,
    pending_label: Option<String>,
    strict: bool,
}

impl Compiler {
    pub fn compile_program(program: &Program) -> Result<Rc<Chunk>, JsError> {
        let mut comp = Compiler {
            chunk: Chunk::new(program.strict, true),
            loops: Vec::new(),
            env_depth: 0,
            stack_depth: 0,
            handler_depth: 0,
            track_completion: !program.is_module,
            pending_label: None,
            strict: program.strict,
        };
        collect_hoisted(&program.body, &mut comp.chunk, program.strict)?;
        collect_block_lex(&program.body, &mut comp.chunk.lex_names);
        for stmt in &program.body {
            comp.compile_stmt(stmt)?;
        }
        if comp.track_completion {
            comp.emit(Op::PushLastValue);
        } else {
            comp.emit(Op::PushUndefined);
        }
        comp.emit(Op::Return);
        Ok(Rc::new(comp.chunk))
    }

    pub fn compile_function(func: &FuncNode) -> Result<Rc<FuncTemplate>, JsError> {
        let mut comp = Compiler {
            chunk: Chunk::new(func.strict, false),
            loops: Vec::new(),
            env_depth: 0,
            stack_depth: 0,
            handler_depth: 0,
            track_completion: false,
            pending_label: None,
            strict: func.strict,
        };
        match &func.body {
            FuncBody::Block(stmts) => {
                collect_hoisted(stmts, &mut comp.chunk, func.strict)?;
                collect_block_lex(stmts, &mut comp.chunk.lex_names);
                for stmt in stmts {
                    comp.compile_stmt(stmt)?;
                }
                comp.emit(Op::PushUndefined);
                comp.emit(Op::Return);
            }
            FuncBody::Expr(expr) => {
                comp.compile_expr(expr)?;
                comp.emit(Op::Return);
            }
        }
        Ok(Rc::new(FuncTemplate {
            name: func.name.clone(),
            chunk: Rc::new(comp.chunk),
            params: Rc::new(func.params.clone()),
            is_async: func.is_async,
            is_generator: func.is_generator,
            is_arrow: func.is_arrow,
            strict: func.strict,
            source: func.source.clone(),
        }))
    }

    pub fn compile_expr_chunk(expr: &Expr, strict: bool) -> Result<Rc<Chunk>, JsError> {
        let mut comp = Compiler {
            chunk: Chunk::new(strict, false),
            loops: Vec::new(),
            env_depth: 0,
            stack_depth: 0,
            handler_depth: 0,
            track_completion: false,
            pending_label: None,
            strict,
        };
        comp.compile_expr(expr)?;
        comp.emit(Op::Return);
        Ok(Rc::new(comp.chunk))
    }

    pub fn compile_block_chunk(stmts: &[Stmt], strict: bool) -> Result<Rc<Chunk>, JsError> {
        let mut comp = Compiler {
            chunk: Chunk::new(strict, false),
            loops: Vec::new(),
            env_depth: 0,
            stack_depth: 0,
            handler_depth: 0,
            track_completion: false,
            pending_label: None,
            strict,
        };
        collect_hoisted(stmts, &mut comp.chunk, strict)?;
        collect_block_lex(stmts, &mut comp.chunk.lex_names);
        for s in stmts {
            comp.compile_stmt(s)?;
        }
        comp.emit(Op::PushUndefined);
        comp.emit(Op::Return);
        Ok(Rc::new(comp.chunk))
    }

    fn emit(&mut self, op: Op) -> usize {
        self.chunk.emit(op)
    }

    fn current_ip(&self) -> u32 {
        self.chunk.ops.len() as u32
    }

    fn str_const(&mut self, s: &str) -> u32 {
        self.chunk.add_string(s.to_string())
    }

    fn push_scope(&mut self) {
        self.emit(Op::EnterScope);
        self.env_depth += 1;
    }

    fn pop_scope(&mut self) {
        self.emit(Op::ExitScope);
        self.env_depth -= 1;
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), JsError> {
        let label = self.pending_label.take();
        match stmt {
            Stmt::Empty | Stmt::FuncDecl(_) => Ok(()),
            Stmt::Debugger => {
                self.emit(Op::Debugger);
                Ok(())
            }
            Stmt::Block(stmts) => {
                if let Some(lbl) = label {
                    let break_env = self.env_depth;
                    let break_stack = self.stack_depth;
                    let break_handler = self.handler_depth;
                    self.loops.push(LoopContext {
                        label: Some(lbl),
                        is_loop: false,
                        break_jumps: Vec::new(),
                        continue_jumps: Vec::new(),
                        break_env_depth: break_env,
                        break_stack_depth: break_stack,
                        break_handler_depth: break_handler,
                        cont_env_depth: break_env,
                        cont_stack_depth: break_stack,
                        cont_handler_depth: break_handler,
                    });
                    self.compile_block_stmts(stmts)?;
                    let ctx = self.loops.pop().unwrap();
                    let end = self.current_ip();
                    for j in ctx.break_jumps {
                        self.chunk.patch_jump(j, end);
                    }
                    Ok(())
                } else {
                    self.compile_block_stmts(stmts)
                }
            }
            Stmt::VarDecl { kind, decls } => self.compile_var_decl(*kind, decls),
            Stmt::ClassDecl(cls) => {
                let name = cls.name.clone().unwrap_or_default();
                let idx = self.str_const(&name);
                self.compile_class(cls)?;
                self.emit(Op::InitBinding(idx));
                self.stack_depth -= 1;
                Ok(())
            }
            Stmt::Expr(expr) => {
                self.compile_expr(expr)?;
                if self.track_completion {
                    self.emit(Op::SetLastValue);
                }
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                Ok(())
            }
            Stmt::If {
                test,
                consequent,
                alternate,
            } => {
                self.compile_expr(test)?;
                let j_else = self.emit(Op::JumpIfFalse(0));
                self.stack_depth -= 1;
                self.compile_stmt(consequent)?;
                if let Some(alt) = alternate {
                    let j_end = self.emit(Op::Jump(0));
                    let else_ip = self.current_ip();
                    self.chunk.patch_jump(j_else, else_ip);
                    self.compile_stmt(alt)?;
                    let end_ip = self.current_ip();
                    self.chunk.patch_jump(j_end, end_ip);
                } else {
                    let end_ip = self.current_ip();
                    self.chunk.patch_jump(j_else, end_ip);
                }
                Ok(())
            }
            Stmt::While { test, body } => {
                let break_env = self.env_depth;
                let break_stack = self.stack_depth;
                let break_handler = self.handler_depth;
                self.loops.push(LoopContext {
                    label,
                    is_loop: true,
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    break_env_depth: break_env,
                    break_stack_depth: break_stack,
                    break_handler_depth: break_handler,
                    cont_env_depth: break_env,
                    cont_stack_depth: break_stack,
                    cont_handler_depth: break_handler,
                });
                let loop_start = self.current_ip();
                self.compile_expr(test)?;
                let j_exit = self.emit(Op::JumpIfFalse(0));
                self.stack_depth -= 1;
                self.compile_stmt(body)?;
                self.emit(Op::Jump(loop_start));
                let loop_end = self.current_ip();
                self.chunk.patch_jump(j_exit, loop_end);
                let ctx = self.loops.pop().unwrap();
                for j in ctx.continue_jumps {
                    self.chunk.patch_jump(j, loop_start);
                }
                for j in ctx.break_jumps {
                    self.chunk.patch_jump(j, loop_end);
                }
                Ok(())
            }
            Stmt::DoWhile { body, test } => {
                let break_env = self.env_depth;
                let break_stack = self.stack_depth;
                let break_handler = self.handler_depth;
                self.loops.push(LoopContext {
                    label,
                    is_loop: true,
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    break_env_depth: break_env,
                    break_stack_depth: break_stack,
                    break_handler_depth: break_handler,
                    cont_env_depth: break_env,
                    cont_stack_depth: break_stack,
                    cont_handler_depth: break_handler,
                });
                let body_start = self.current_ip();
                self.compile_stmt(body)?;
                let test_ip = self.current_ip();
                self.compile_expr(test)?;
                self.emit(Op::JumpIfTrue(body_start));
                self.stack_depth -= 1;
                let loop_end = self.current_ip();
                let ctx = self.loops.pop().unwrap();
                for j in ctx.continue_jumps {
                    self.chunk.patch_jump(j, test_ip);
                }
                for j in ctx.break_jumps {
                    self.chunk.patch_jump(j, loop_end);
                }
                Ok(())
            }
            Stmt::For {
                init,
                test,
                update,
                body,
            } => self.compile_for(label, init, test, update, body),
            Stmt::ForIn { left, right, body } => self.compile_for_in(label, left, right, body),
            Stmt::ForOf {
                left,
                right,
                body,
                is_await,
            } => self.compile_for_of(label, left, right, body, *is_await),
            Stmt::Switch {
                discriminant,
                cases,
            } => self.compile_switch(label, discriminant, cases),
            Stmt::Try {
                block,
                handler,
                finalizer,
            } => self.compile_try(block, handler, finalizer),
            Stmt::Throw(expr) => {
                self.compile_expr(expr)?;
                self.emit(Op::Throw);
                self.stack_depth -= 1;
                Ok(())
            }
            Stmt::Return(expr) => {
                if let Some(e) = expr {
                    self.compile_expr(e)?;
                } else {
                    self.emit(Op::PushUndefined);
                    self.stack_depth += 1;
                }
                self.emit(Op::Return);
                self.stack_depth -= 1;
                Ok(())
            }
            Stmt::Break(lbl) => self.compile_break(lbl.as_deref()),
            Stmt::Continue(lbl) => self.compile_continue(lbl.as_deref()),
            Stmt::Labeled { label: lbl, body } => {
                if matches!(
                    body.as_ref(),
                    Stmt::While { .. }
                        | Stmt::DoWhile { .. }
                        | Stmt::For { .. }
                        | Stmt::ForIn { .. }
                        | Stmt::ForOf { .. }
                        | Stmt::Switch { .. }
                        | Stmt::Block(_)
                ) {
                    self.pending_label = Some(lbl.clone());
                    self.compile_stmt(body)
                } else {
                    let break_env = self.env_depth;
                    let break_stack = self.stack_depth;
                    let break_handler = self.handler_depth;
                    self.loops.push(LoopContext {
                        label: Some(lbl.clone()),
                        is_loop: false,
                        break_jumps: Vec::new(),
                        continue_jumps: Vec::new(),
                        break_env_depth: break_env,
                        break_stack_depth: break_stack,
                        break_handler_depth: break_handler,
                        cont_env_depth: break_env,
                        cont_stack_depth: break_stack,
                        cont_handler_depth: break_handler,
                    });
                    self.compile_stmt(body)?;
                    let ctx = self.loops.pop().unwrap();
                    let end = self.current_ip();
                    for j in ctx.break_jumps {
                        self.chunk.patch_jump(j, end);
                    }
                    Ok(())
                }
            }
            Stmt::With { object, body } => {
                self.compile_expr(object)?;
                self.emit(Op::EnterWith);
                self.stack_depth -= 1;
                self.env_depth += 1;
                self.compile_stmt(body)?;
                self.pop_scope();
                Ok(())
            }
            Stmt::Import(imp) => self.compile_import(imp),
            Stmt::Export(exp) => self.compile_export(exp),
        }
    }

    fn compile_block_stmts(&mut self, stmts: &[Stmt]) -> Result<(), JsError> {
        let mut lex_names = Vec::new();
        let mut block_funcs = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::VarDecl {
                    kind: VarKind::Let,
                    decls,
                } => {
                    for d in decls {
                        collect_pattern_names(&d.pattern, &mut lex_names, false);
                    }
                }
                Stmt::VarDecl {
                    kind: VarKind::Const,
                    decls,
                } => {
                    for d in decls {
                        collect_pattern_names(&d.pattern, &mut lex_names, true);
                    }
                }
                Stmt::ClassDecl(cls) => {
                    if let Some(ref name) = cls.name {
                        lex_names.push((name.clone(), false));
                    }
                }
                Stmt::FuncDecl(func) => {
                    if let Some(ref name) = func.name {
                        block_funcs.push((name.clone(), func.clone()));
                    }
                }
                _ => {}
            }
        }
        let needs_scope = !lex_names.is_empty() || !block_funcs.is_empty();
        if needs_scope {
            self.push_scope();
            for (name, is_const) in &lex_names {
                let idx = self.str_const(name);
                if *is_const {
                    self.emit(Op::DeclareConst(idx));
                } else {
                    self.emit(Op::DeclareLet(idx));
                }
            }
            for (name, func) in &block_funcs {
                let tmpl = Compiler::compile_function(func)?;
                let fidx = self.chunk.add_const(ConstValue::Func(tmpl));
                let nidx = self.str_const(name);
                self.emit(Op::DeclareLet(nidx));
                self.emit(Op::CreateFunction(fidx));
                self.stack_depth += 1;
                self.emit(Op::InitBinding(nidx));
                self.stack_depth -= 1;
            }
        }
        for stmt in stmts {
            self.compile_stmt(stmt)?;
        }
        if needs_scope {
            self.pop_scope();
        }
        Ok(())
    }

    fn compile_var_decl(&mut self, kind: VarKind, decls: &[VarDeclarator]) -> Result<(), JsError> {
        for decl in decls {
            if let Some(ref init) = decl.init {
                self.compile_expr(init)?;
                self.compile_pattern_bind(&decl.pattern, kind)?;
            } else if kind != VarKind::Var {
                self.emit(Op::PushUndefined);
                self.stack_depth += 1;
                self.compile_pattern_bind(&decl.pattern, kind)?;
            }
        }
        Ok(())
    }

    fn compile_for(
        &mut self,
        label: Option<String>,
        init: &Option<ForInit>,
        test: &Option<Expr>,
        update: &Option<Expr>,
        body: &Stmt,
    ) -> Result<(), JsError> {
        let mut lex_names = Vec::new();
        let mut per_iter_names = Vec::new();
        let has_lex_scope = match init {
            Some(ForInit::VarDecl { kind, decls }) if *kind != VarKind::Var => {
                for d in decls {
                    collect_pattern_names(&d.pattern, &mut lex_names, *kind == VarKind::Const);
                }
                for (n, _) in &lex_names {
                    per_iter_names.push(n.clone());
                }
                true
            }
            _ => false,
        };

        let break_env = self.env_depth;
        let break_stack = self.stack_depth;
        let break_handler = self.handler_depth;

        if has_lex_scope {
            self.push_scope();
            for (name, is_const) in &lex_names {
                let idx = self.str_const(name);
                if *is_const {
                    self.emit(Op::DeclareConst(idx));
                } else {
                    self.emit(Op::DeclareLet(idx));
                }
            }
        }

        if let Some(for_init) = init {
            match for_init {
                ForInit::VarDecl { kind, decls } => self.compile_var_decl(*kind, decls)?,
                ForInit::Expr(e) => {
                    self.compile_expr(e)?;
                    self.emit(Op::Pop);
                    self.stack_depth -= 1;
                }
            }
        }

        let cont_env = self.env_depth;
        let cont_stack = self.stack_depth;
        let cont_handler = self.handler_depth;
        self.loops.push(LoopContext {
            label,
            is_loop: true,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            break_env_depth: break_env,
            break_stack_depth: break_stack,
            break_handler_depth: break_handler,
            cont_env_depth: cont_env,
            cont_stack_depth: cont_stack,
            cont_handler_depth: cont_handler,
        });

        let names_const = if !per_iter_names.is_empty() {
            Some(self.chunk.add_const(ConstValue::Names(per_iter_names)))
        } else {
            None
        };

        let loop_start = self.current_ip();
        let j_exit = if let Some(t) = test {
            self.compile_expr(t)?;
            let j = self.emit(Op::JumpIfFalse(0));
            self.stack_depth -= 1;
            Some(j)
        } else {
            None
        };

        if let Some(nc) = names_const {
            self.emit(Op::CloneLoopEnv(nc));
        }
        self.compile_stmt(body)?;
        let update_ip = self.current_ip();
        if let Some(nc) = names_const {
            self.emit(Op::CloneLoopEnv(nc));
        }
        if let Some(u) = update {
            self.compile_expr(u)?;
            self.emit(Op::Pop);
            self.stack_depth -= 1;
        }
        self.emit(Op::Jump(loop_start));
        let after_loop = self.current_ip();
        if let Some(j) = j_exit {
            self.chunk.patch_jump(j, after_loop);
        }
        if has_lex_scope {
            self.pop_scope();
        }
        let final_end = self.current_ip();
        let ctx = self.loops.pop().unwrap();
        for j in ctx.continue_jumps {
            self.chunk.patch_jump(j, update_ip);
        }
        for j in ctx.break_jumps {
            self.chunk.patch_jump(j, final_end);
        }
        Ok(())
    }

    fn compile_for_in(
        &mut self,
        label: Option<String>,
        left: &ForInOfLeft,
        right: &Expr,
        body: &Stmt,
    ) -> Result<(), JsError> {
        self.compile_expr(right)?;
        self.emit(Op::ForInInit);

        let break_env = self.env_depth;
        let break_stack = self.stack_depth;
        let break_handler = self.handler_depth;
        self.loops.push(LoopContext {
            label,
            is_loop: true,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            break_env_depth: break_env,
            break_stack_depth: break_stack,
            break_handler_depth: break_handler,
            cont_env_depth: break_env,
            cont_stack_depth: break_stack,
            cont_handler_depth: break_handler,
        });

        let loop_start = self.current_ip();
        let j_done = self.emit(Op::ForInNext(0));
        self.stack_depth += 1;

        let has_lex = matches!(
            left,
            ForInOfLeft::VarDecl {
                kind: VarKind::Let | VarKind::Const,
                ..
            }
        );
        if has_lex {
            self.push_scope();
            if let ForInOfLeft::VarDecl { kind, pattern } = left {
                let mut names = Vec::new();
                collect_pattern_names(pattern, &mut names, *kind == VarKind::Const);
                for (n, is_c) in names {
                    let idx = self.str_const(&n);
                    if is_c {
                        self.emit(Op::DeclareConst(idx));
                    } else {
                        self.emit(Op::DeclareLet(idx));
                    }
                }
            }
        }
        match left {
            ForInOfLeft::VarDecl { kind, pattern } => {
                self.compile_pattern_bind(pattern, *kind)?;
            }
            ForInOfLeft::Pattern(pat) => {
                self.compile_pattern_assign(pat)?;
            }
        }
        self.compile_stmt(body)?;
        if has_lex {
            self.pop_scope();
        }
        self.emit(Op::Jump(loop_start));
        let done_ip = self.current_ip();
        self.chunk.patch_jump(j_done, done_ip);
        let ctx = self.loops.pop().unwrap();
        for j in ctx.continue_jumps {
            self.chunk.patch_jump(j, loop_start);
        }
        for j in ctx.break_jumps {
            self.chunk.patch_jump(j, done_ip);
        }
        self.emit(Op::Pop);
        self.stack_depth -= 1;
        Ok(())
    }

    fn compile_for_of(
        &mut self,
        label: Option<String>,
        left: &ForInOfLeft,
        right: &Expr,
        body: &Stmt,
        is_await: bool,
    ) -> Result<(), JsError> {
        self.compile_expr(right)?;
        self.emit(Op::GetIterator(is_await));

        let break_env = self.env_depth;
        let break_stack = self.stack_depth;
        let break_handler = self.handler_depth;
        self.loops.push(LoopContext {
            label,
            is_loop: true,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            break_env_depth: break_env,
            break_stack_depth: break_stack,
            break_handler_depth: break_handler,
            cont_env_depth: break_env,
            cont_stack_depth: break_stack,
            cont_handler_depth: break_handler,
        });

        let loop_start = self.current_ip();
        let j_done = self.emit(Op::IteratorNext(0));
        self.stack_depth += 1;
        if is_await {
            self.emit(Op::Await);
        }

        let has_lex = matches!(
            left,
            ForInOfLeft::VarDecl {
                kind: VarKind::Let | VarKind::Const,
                ..
            }
        );
        if has_lex {
            self.push_scope();
            if let ForInOfLeft::VarDecl { kind, pattern } = left {
                let mut names = Vec::new();
                collect_pattern_names(pattern, &mut names, *kind == VarKind::Const);
                for (n, is_c) in names {
                    let idx = self.str_const(&n);
                    if is_c {
                        self.emit(Op::DeclareConst(idx));
                    } else {
                        self.emit(Op::DeclareLet(idx));
                    }
                }
            }
        }
        match left {
            ForInOfLeft::VarDecl { kind, pattern } => {
                self.compile_pattern_bind(pattern, *kind)?;
            }
            ForInOfLeft::Pattern(pat) => {
                self.compile_pattern_assign(pat)?;
            }
        }
        self.compile_stmt(body)?;
        if has_lex {
            self.pop_scope();
        }
        self.emit(Op::Jump(loop_start));
        let break_ip = self.current_ip();
        self.emit(Op::IteratorClose);
        let done_ip = self.current_ip();
        self.chunk.patch_jump(j_done, done_ip);
        let ctx = self.loops.pop().unwrap();
        for j in ctx.continue_jumps {
            self.chunk.patch_jump(j, loop_start);
        }
        for j in ctx.break_jumps {
            self.chunk.patch_jump(j, break_ip);
        }
        self.emit(Op::Pop);
        self.stack_depth -= 1;
        Ok(())
    }

    fn compile_switch(
        &mut self,
        label: Option<String>,
        discriminant: &Expr,
        cases: &[SwitchCase],
    ) -> Result<(), JsError> {
        self.compile_expr(discriminant)?;
        let break_env = self.env_depth;
        let break_stack = self.stack_depth;
        let break_handler = self.handler_depth;
        self.loops.push(LoopContext {
            label,
            is_loop: false,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            break_env_depth: break_env,
            break_stack_depth: break_stack,
            break_handler_depth: break_handler,
            cont_env_depth: break_env,
            cont_stack_depth: break_stack,
            cont_handler_depth: break_handler,
        });

        let mut test_jumps: Vec<(usize, usize)> = Vec::new();
        let mut default_idx: Option<usize> = None;

        for (i, case) in cases.iter().enumerate() {
            if let Some(ref test_expr) = case.test {
                self.emit(Op::Dup);
                self.stack_depth += 1;
                self.compile_expr(test_expr)?;
                self.emit(Op::Binary(BinaryOp::StrictEq));
                self.stack_depth -= 1;
                let j = self.emit(Op::JumpIfTrue(0));
                self.stack_depth -= 1;
                test_jumps.push((i, j));
            } else {
                default_idx = Some(i);
            }
        }
        let j_default = self.emit(Op::Jump(0));

        let mut case_ips = Vec::with_capacity(cases.len());
        self.push_scope();
        let mut lex_names = Vec::new();
        for case in cases {
            collect_block_lex(&case.consequent, &mut lex_names);
        }
        for (name, is_c) in lex_names {
            let idx = self.str_const(&name);
            if is_c {
                self.emit(Op::DeclareConst(idx));
            } else {
                self.emit(Op::DeclareLet(idx));
            }
        }

        for case in cases {
            case_ips.push(self.current_ip());
            for stmt in &case.consequent {
                self.compile_stmt(stmt)?;
            }
        }
        self.pop_scope();

        let end_ip = self.current_ip();
        for (i, j) in test_jumps {
            self.chunk.patch_jump(j, case_ips[i]);
        }
        if let Some(di) = default_idx {
            self.chunk.patch_jump(j_default, case_ips[di]);
        } else {
            self.chunk.patch_jump(j_default, end_ip);
        }

        let ctx = self.loops.pop().unwrap();
        for j in ctx.break_jumps {
            self.chunk.patch_jump(j, end_ip);
        }
        self.emit(Op::Pop);
        self.stack_depth -= 1;
        Ok(())
    }

    fn compile_try(
        &mut self,
        block: &[Stmt],
        handler: &Option<CatchClause>,
        finalizer: &Option<Vec<Stmt>>,
    ) -> Result<(), JsError> {
        let try_push_idx = self.emit(Op::TryPush {
            catch_ip: None,
            finally_ip: None,
        });
        self.handler_depth += 1;

        self.compile_block_stmts(block)?;

        let j_after_try = self.emit(Op::Jump(0));
        let mut catch_ip = None;
        let mut j_after_catch = None;

        if let Some(catch) = handler {
            catch_ip = Some(self.current_ip());
            self.stack_depth += 1;
            self.push_scope();
            if let Some(ref pat) = catch.param {
                let mut names = Vec::new();
                collect_pattern_names(pat, &mut names, false);
                for (n, _) in names {
                    let idx = self.str_const(&n);
                    self.emit(Op::DeclareLet(idx));
                }
                self.compile_pattern_bind(pat, VarKind::Let)?;
            } else {
                self.emit(Op::Pop);
                self.stack_depth -= 1;
            }
            self.compile_block_stmts(&catch.body)?;
            self.pop_scope();
            j_after_catch = Some(self.emit(Op::Jump(0)));
        }

        let mut finally_ip = None;
        if let Some(fin) = finalizer {
            let fin_entry = self.current_ip();
            finally_ip = Some(fin_entry);
            self.chunk.patch_jump(j_after_try, fin_entry);
            if let Some(jc) = j_after_catch {
                self.chunk.patch_jump(jc, fin_entry);
            }
            self.compile_block_stmts(fin)?;
            self.emit(Op::FinallyEnd);
        } else {
            let pop_ip = self.current_ip();
            self.chunk.patch_jump(j_after_try, pop_ip);
            if let Some(jc) = j_after_catch {
                self.chunk.patch_jump(jc, pop_ip);
            }
            self.emit(Op::TryPop);
        }
        self.handler_depth -= 1;

        self.chunk.ops[try_push_idx] = Op::TryPush {
            catch_ip,
            finally_ip,
        };
        Ok(())
    }

    fn compile_break(&mut self, label: Option<&str>) -> Result<(), JsError> {
        let idx = if let Some(lbl) = label {
            self.loops
                .iter()
                .rposition(|c| c.label.as_deref() == Some(lbl))
                .ok_or_else(|| JsError::syntax(format!("Undefined label '{}'", lbl)))?
        } else {
            self.loops
                .iter()
                .rposition(|c| c.is_loop || c.label.is_none())
                .ok_or_else(|| JsError::syntax("Illegal break statement"))?
        };
        let env_depth = self.loops[idx].break_env_depth;
        let stack_depth = self.loops[idx].break_stack_depth;
        let handler_depth = self.loops[idx].break_handler_depth;
        let j = self.emit(Op::Break {
            target: 0,
            env_depth,
            stack_depth,
            handler_depth,
        });
        self.loops[idx].break_jumps.push(j);
        Ok(())
    }

    fn compile_continue(&mut self, label: Option<&str>) -> Result<(), JsError> {
        let idx = if let Some(lbl) = label {
            self.loops
                .iter()
                .rposition(|c| c.is_loop && c.label.as_deref() == Some(lbl))
                .ok_or_else(|| JsError::syntax(format!("Undefined loop label '{}'", lbl)))?
        } else {
            self.loops
                .iter()
                .rposition(|c| c.is_loop)
                .ok_or_else(|| JsError::syntax("Illegal continue statement"))?
        };
        let env_depth = self.loops[idx].cont_env_depth;
        let stack_depth = self.loops[idx].cont_stack_depth;
        let handler_depth = self.loops[idx].cont_handler_depth;
        let j = self.emit(Op::Break {
            target: 0,
            env_depth,
            stack_depth,
            handler_depth,
        });
        self.loops[idx].continue_jumps.push(j);
        Ok(())
    }

    fn compile_import(&mut self, imp: &ImportDecl) -> Result<(), JsError> {
        let src_idx = self.str_const(&imp.source);
        self.emit(Op::ImportModule(src_idx));
        self.stack_depth += 1;
        for spec in &imp.specifiers {
            match spec {
                ImportSpecifier::Default(local) => {
                    let def_idx = self.str_const("default");
                    let loc_idx = self.str_const(local);
                    self.emit(Op::ImportBinding(def_idx, loc_idx));
                }
                ImportSpecifier::Named { imported, local } => {
                    let imp_idx = self.str_const(imported);
                    let loc_idx = self.str_const(local);
                    self.emit(Op::ImportBinding(imp_idx, loc_idx));
                }
                ImportSpecifier::Namespace(local) => {
                    let loc_idx = self.str_const(local);
                    self.emit(Op::ImportNamespace(loc_idx));
                }
            }
        }
        self.emit(Op::Pop);
        self.stack_depth -= 1;
        Ok(())
    }

    fn compile_export(&mut self, exp: &ExportDecl) -> Result<(), JsError> {
        match exp {
            ExportDecl::DefaultExpr(expr) => {
                self.compile_expr(expr)?;
                let idx = self.str_const("default");
                self.emit(Op::ExportBinding(idx));
                self.stack_depth -= 1;
            }
            ExportDecl::DefaultFunc(func) => {
                let tmpl = Compiler::compile_function(func)?;
                let fidx = self.chunk.add_const(ConstValue::Func(tmpl));
                self.emit(Op::CreateFunction(fidx));
                self.stack_depth += 1;
                if let Some(ref name) = func.name {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    let nidx = self.str_const(name);
                    self.emit(Op::InitBinding(nidx));
                    self.stack_depth -= 1;
                }
                let idx = self.str_const("default");
                self.emit(Op::ExportBinding(idx));
                self.stack_depth -= 1;
            }
            ExportDecl::DefaultClass(cls) => {
                self.compile_class(cls)?;
                if let Some(ref name) = cls.name {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    let nidx = self.str_const(name);
                    self.emit(Op::InitBinding(nidx));
                    self.stack_depth -= 1;
                }
                let idx = self.str_const("default");
                self.emit(Op::ExportBinding(idx));
                self.stack_depth -= 1;
            }
            ExportDecl::Decl(stmt) => {
                self.compile_stmt(stmt)?;
                let mut names = Vec::new();
                match stmt.as_ref() {
                    Stmt::VarDecl { decls, .. } => {
                        for d in decls {
                            collect_pattern_names(&d.pattern, &mut names, false);
                        }
                    }
                    Stmt::FuncDecl(f) => {
                        if let Some(ref n) = f.name {
                            names.push((n.clone(), false));
                        }
                    }
                    Stmt::ClassDecl(c) => {
                        if let Some(ref n) = c.name {
                            names.push((n.clone(), false));
                        }
                    }
                    _ => {}
                }
                for (name, _) in names {
                    let nidx = self.str_const(&name);
                    self.emit(Op::GetVar(nidx));
                    self.stack_depth += 1;
                    self.emit(Op::ExportBinding(nidx));
                    self.stack_depth -= 1;
                }
            }
            ExportDecl::Named { specifiers, source } => {
                if let Some(ref src) = source {
                    let sidx = self.str_const(src);
                    self.emit(Op::ImportModule(sidx));
                    self.stack_depth += 1;
                    for spec in specifiers {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        let lidx = self.str_const(&spec.local);
                        self.emit(Op::GetProp(lidx));
                        let eidx = self.str_const(&spec.exported);
                        self.emit(Op::ExportBinding(eidx));
                        self.stack_depth -= 1;
                    }
                    self.emit(Op::Pop);
                    self.stack_depth -= 1;
                } else {
                    for spec in specifiers {
                        let lidx = self.str_const(&spec.local);
                        self.emit(Op::GetVar(lidx));
                        self.stack_depth += 1;
                        let eidx = self.str_const(&spec.exported);
                        self.emit(Op::ExportBinding(eidx));
                        self.stack_depth -= 1;
                    }
                }
            }
            ExportDecl::All { exported, source } => {
                let sidx = self.str_const(source);
                self.emit(Op::ImportModule(sidx));
                self.stack_depth += 1;
                if let Some(ref name) = exported {
                    let eidx = self.str_const(name);
                    self.emit(Op::ExportBinding(eidx));
                    self.stack_depth -= 1;
                } else {
                    let star = self.str_const("*");
                    self.emit(Op::ExportBinding(star));
                    self.stack_depth -= 1;
                }
            }
        }
        Ok(())
    }

    pub fn compile_pattern_bind(&mut self, pat: &Pattern, kind: VarKind) -> Result<(), JsError> {
        match pat {
            Pattern::Ident(name) => {
                let idx = self.str_const(name);
                if kind == VarKind::Var {
                    self.emit(Op::SetVar(idx));
                } else {
                    self.emit(Op::InitBinding(idx));
                }
                self.stack_depth -= 1;
                Ok(())
            }
            Pattern::Assign { left, right } => {
                let j_skip = self.emit(Op::JumpIfNotUndefined(0));
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                self.compile_expr(right)?;
                let target = self.current_ip();
                self.chunk.patch_jump(j_skip, target);
                self.compile_pattern_bind(left, kind)
            }
            Pattern::Array { elements, rest } => {
                self.emit(Op::GetIterator(false));
                for el in elements {
                    self.emit(Op::IteratorNextValue);
                    self.stack_depth += 1;
                    if let Some(sub) = el {
                        self.compile_pattern_bind(sub, kind)?;
                    } else {
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                    }
                }
                if let Some(rest_pat) = rest {
                    self.emit(Op::IteratorRest);
                    self.stack_depth += 1;
                    self.compile_pattern_bind(rest_pat, kind)?;
                } else {
                    self.emit(Op::IteratorClose);
                }
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                Ok(())
            }
            Pattern::Object { props, rest } => {
                let mut excluded_names = Vec::new();
                for prop in props {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    match &prop.key {
                        PropKey::Ident(s) | PropKey::String(s) => {
                            excluded_names.push(s.clone());
                            let idx = self.str_const(s);
                            self.emit(Op::GetProp(idx));
                        }
                        PropKey::Number(n) => {
                            let s = crate::value::number_to_string(*n);
                            excluded_names.push(s.clone());
                            let idx = self.str_const(&s);
                            self.emit(Op::GetProp(idx));
                        }
                        PropKey::Computed(expr) => {
                            self.compile_expr(expr)?;
                            self.emit(Op::GetElem);
                            self.stack_depth -= 1;
                        }
                        PropKey::Private(s) => {
                            let idx = self.str_const(s);
                            self.emit(Op::GetPrivate(idx));
                        }
                    }
                    self.compile_pattern_bind(&prop.value, kind)?;
                }
                if let Some(rest_pat) = rest {
                    let cidx = self.chunk.add_const(ConstValue::Names(excluded_names));
                    self.emit(Op::ObjectRest(cidx));
                    self.compile_pattern_bind(rest_pat, kind)?;
                } else {
                    self.emit(Op::Pop);
                    self.stack_depth -= 1;
                }
                Ok(())
            }
            Pattern::Member(expr) => self.compile_member_assign_from_stack(expr),
        }
    }

    fn compile_pattern_assign(&mut self, pat: &Pattern) -> Result<(), JsError> {
        match pat {
            Pattern::Ident(name) => {
                let idx = self.str_const(name);
                self.emit(Op::SetVar(idx));
                self.stack_depth -= 1;
                Ok(())
            }
            Pattern::Member(expr) => self.compile_member_assign_from_stack(expr),
            Pattern::Assign { left, right } => {
                let j_skip = self.emit(Op::JumpIfNotUndefined(0));
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                self.compile_expr(right)?;
                let target = self.current_ip();
                self.chunk.patch_jump(j_skip, target);
                self.compile_pattern_assign(left)
            }
            Pattern::Array { elements, rest } => {
                self.emit(Op::GetIterator(false));
                for el in elements {
                    self.emit(Op::IteratorNextValue);
                    self.stack_depth += 1;
                    if let Some(sub) = el {
                        self.compile_pattern_assign(sub)?;
                    } else {
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                    }
                }
                if let Some(rest_pat) = rest {
                    self.emit(Op::IteratorRest);
                    self.stack_depth += 1;
                    self.compile_pattern_assign(rest_pat)?;
                } else {
                    self.emit(Op::IteratorClose);
                }
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                Ok(())
            }
            Pattern::Object { props, rest } => {
                let mut excluded = Vec::new();
                for prop in props {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    match &prop.key {
                        PropKey::Ident(s) | PropKey::String(s) => {
                            excluded.push(s.clone());
                            let idx = self.str_const(s);
                            self.emit(Op::GetProp(idx));
                        }
                        PropKey::Number(n) => {
                            let s = crate::value::number_to_string(*n);
                            excluded.push(s.clone());
                            let idx = self.str_const(&s);
                            self.emit(Op::GetProp(idx));
                        }
                        PropKey::Computed(expr) => {
                            self.compile_expr(expr)?;
                            self.emit(Op::GetElem);
                            self.stack_depth -= 1;
                        }
                        PropKey::Private(s) => {
                            let idx = self.str_const(s);
                            self.emit(Op::GetPrivate(idx));
                        }
                    }
                    self.compile_pattern_assign(&prop.value)?;
                }
                if let Some(rest_pat) = rest {
                    let cidx = self.chunk.add_const(ConstValue::Names(excluded));
                    self.emit(Op::ObjectRest(cidx));
                    self.compile_pattern_assign(rest_pat)?;
                } else {
                    self.emit(Op::Pop);
                    self.stack_depth -= 1;
                }
                Ok(())
            }
        }
    }

    fn compile_member_assign_from_stack(&mut self, expr: &Expr) -> Result<(), JsError> {
        let Expr::Member {
            object, property, ..
        } = expr
        else {
            return Err(JsError::syntax("Expected member expression"));
        };
        match property {
            MemberProp::Ident(name) => {
                self.compile_expr(object)?;
                self.emit(Op::Swap);
                let idx = self.str_const(name);
                self.emit(Op::SetProp(idx));
                self.stack_depth -= 1;
                self.emit(Op::Pop);
                self.stack_depth -= 1;
            }
            MemberProp::Private(name) => {
                self.compile_expr(object)?;
                self.emit(Op::Swap);
                let idx = self.str_const(name);
                self.emit(Op::SetPrivate(idx));
                self.stack_depth -= 1;
                self.emit(Op::Pop);
                self.stack_depth -= 1;
            }
            MemberProp::Computed(key) => {
                self.compile_expr(object)?;
                self.compile_expr(key)?;
                self.emit(Op::Rot3);
                self.emit(Op::SetElem);
                self.stack_depth -= 2;
                self.emit(Op::Pop);
                self.stack_depth -= 1;
            }
        }
        Ok(())
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<(), JsError> {
        match expr {
            Expr::Undefined => {
                self.emit(Op::PushUndefined);
                self.stack_depth += 1;
            }
            Expr::Null => {
                self.emit(Op::PushNull);
                self.stack_depth += 1;
            }
            Expr::Bool(true) => {
                self.emit(Op::PushTrue);
                self.stack_depth += 1;
            }
            Expr::Bool(false) => {
                self.emit(Op::PushFalse);
                self.stack_depth += 1;
            }
            Expr::Number(n) => {
                let idx = self.chunk.add_const(ConstValue::Number(*n));
                self.emit(Op::PushConst(idx));
                self.stack_depth += 1;
            }
            Expr::BigInt(s) => {
                let val = parse_bigint_literal(s)?;
                let idx = self.chunk.add_const(ConstValue::BigInt(val));
                self.emit(Op::PushConst(idx));
                self.stack_depth += 1;
            }
            Expr::String(s) => {
                let idx = self.str_const(s);
                self.emit(Op::PushConst(idx));
                self.stack_depth += 1;
            }
            Expr::RegExp { pattern, flags } => {
                let idx = self
                    .chunk
                    .add_const(ConstValue::RegExp(pattern.clone(), flags.clone()));
                self.emit(Op::PushConst(idx));
                self.stack_depth += 1;
            }
            Expr::This => {
                self.emit(Op::PushThis);
                self.stack_depth += 1;
            }
            Expr::Super => {
                self.emit(Op::PushThis);
                self.stack_depth += 1;
            }
            Expr::NewTarget => {
                self.emit(Op::PushNewTarget);
                self.stack_depth += 1;
            }
            Expr::ImportMeta => {
                self.emit(Op::PushImportMeta);
                self.stack_depth += 1;
            }
            Expr::DynamicImport(spec) => {
                self.compile_expr(spec)?;
                self.emit(Op::DynamicImport);
            }
            Expr::Ident(name) => {
                let idx = self.str_const(name);
                self.emit(Op::GetVar(idx));
                self.stack_depth += 1;
            }
            Expr::PrivateIn { name, right } => {
                self.compile_expr(right)?;
                let idx = self.str_const(name);
                self.emit(Op::HasPrivate(idx));
            }
            Expr::Array(elems) => {
                let has_spread_or_hole = elems
                    .iter()
                    .any(|e| matches!(e, ArrayElem::Spread(_) | ArrayElem::Hole));
                if !has_spread_or_hole {
                    for el in elems {
                        if let ArrayElem::Expr(e) = el {
                            self.compile_expr(e)?;
                        }
                    }
                    let n = elems.len() as u32;
                    self.emit(Op::CreateArray(n));
                    self.stack_depth = self.stack_depth + 1 - n;
                } else {
                    self.emit(Op::CreateArray(0));
                    self.stack_depth += 1;
                    for el in elems {
                        match el {
                            ArrayElem::Hole => {
                                self.emit(Op::ArrayPushHole);
                            }
                            ArrayElem::Expr(e) => {
                                self.compile_expr(e)?;
                                self.emit(Op::ArrayPush);
                                self.stack_depth -= 1;
                            }
                            ArrayElem::Spread(e) => {
                                self.compile_expr(e)?;
                                self.emit(Op::ArraySpread);
                                self.stack_depth -= 1;
                            }
                        }
                    }
                }
            }
            Expr::Object(props) => {
                self.emit(Op::CreateObject);
                self.stack_depth += 1;
                for prop in props {
                    match prop {
                        ObjectProp::Spread(e) => {
                            self.compile_expr(e)?;
                            self.emit(Op::ObjectSpread);
                            self.stack_depth -= 1;
                        }
                        ObjectProp::Property { key, value, .. } => match key {
                            PropKey::Ident(s) | PropKey::String(s) => {
                                self.compile_expr(value)?;
                                let idx = self.str_const(s);
                                self.emit(Op::ObjectSetProp(idx));
                                self.stack_depth -= 1;
                            }
                            PropKey::Number(n) => {
                                self.compile_expr(value)?;
                                let s = crate::value::number_to_string(*n);
                                let idx = self.str_const(&s);
                                self.emit(Op::ObjectSetProp(idx));
                                self.stack_depth -= 1;
                            }
                            PropKey::Computed(k) => {
                                self.compile_expr(k)?;
                                self.compile_expr(value)?;
                                self.emit(Op::ObjectSetComputed);
                                self.stack_depth -= 2;
                            }
                            PropKey::Private(s) => {
                                self.compile_expr(value)?;
                                let idx = self.str_const(s);
                                self.emit(Op::ObjectSetProp(idx));
                                self.stack_depth -= 1;
                            }
                        },
                        ObjectProp::Method { key, kind, func } => {
                            let tmpl = Compiler::compile_function(func)?;
                            let fidx = self.chunk.add_const(ConstValue::Func(tmpl));
                            match key {
                                PropKey::Ident(s) | PropKey::String(s) => {
                                    self.emit(Op::CreateFunction(fidx));
                                    self.stack_depth += 1;
                                    let idx = self.str_const(s);
                                    match kind {
                                        MethodKind::Get => self.emit(Op::ObjectSetGetter(idx)),
                                        MethodKind::Set => self.emit(Op::ObjectSetSetter(idx)),
                                        _ => self.emit(Op::ObjectSetProp(idx)),
                                    };
                                    self.stack_depth -= 1;
                                }
                                PropKey::Number(n) => {
                                    self.emit(Op::CreateFunction(fidx));
                                    self.stack_depth += 1;
                                    let s = crate::value::number_to_string(*n);
                                    let idx = self.str_const(&s);
                                    match kind {
                                        MethodKind::Get => self.emit(Op::ObjectSetGetter(idx)),
                                        MethodKind::Set => self.emit(Op::ObjectSetSetter(idx)),
                                        _ => self.emit(Op::ObjectSetProp(idx)),
                                    };
                                    self.stack_depth -= 1;
                                }
                                PropKey::Computed(k) => {
                                    self.compile_expr(k)?;
                                    self.emit(Op::CreateFunction(fidx));
                                    self.stack_depth += 1;
                                    match kind {
                                        MethodKind::Get => self.emit(Op::ObjectSetGetterComputed),
                                        MethodKind::Set => self.emit(Op::ObjectSetSetterComputed),
                                        _ => self.emit(Op::ObjectSetComputed),
                                    };
                                    self.stack_depth -= 2;
                                }
                                PropKey::Private(s) => {
                                    self.emit(Op::CreateFunction(fidx));
                                    self.stack_depth += 1;
                                    let idx = self.str_const(s);
                                    self.emit(Op::ObjectSetProp(idx));
                                    self.stack_depth -= 1;
                                }
                            }
                        }
                    }
                }
            }
            Expr::Function(func) | Expr::Arrow(func) => {
                let tmpl = Compiler::compile_function(func)?;
                let idx = self.chunk.add_const(ConstValue::Func(tmpl));
                self.emit(Op::CreateFunction(idx));
                self.stack_depth += 1;
            }
            Expr::Class(cls) => {
                self.compile_class(cls)?;
            }
            Expr::Template { tag, quasis, exprs } => {
                if let Some(tag_expr) = tag {
                    let raw: Vec<String> = quasis.iter().map(|q| q.raw.clone()).collect();
                    let cooked: Vec<Option<String>> = quasis.iter().map(|q| q.cooked.clone()).collect();
                    let tidx = self
                        .chunk
                        .add_const(ConstValue::TemplateStrings { raw, cooked });
                    let argc = (1 + exprs.len()) as u32;
                    if let Expr::Member {
                        object, property, ..
                    } = tag_expr.as_ref()
                    {
                        self.compile_expr(object)?;
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.compile_member_get(property)?;
                        self.emit(Op::Swap);
                        self.emit(Op::CreateTemplateObject(tidx));
                        self.stack_depth += 1;
                        for e in exprs {
                            self.compile_expr(e)?;
                        }
                        self.emit(Op::CallMethod(argc));
                        self.stack_depth -= argc + 1;
                    } else {
                        self.compile_expr(tag_expr)?;
                        self.emit(Op::CreateTemplateObject(tidx));
                        self.stack_depth += 1;
                        for e in exprs {
                            self.compile_expr(e)?;
                        }
                        self.emit(Op::Call(argc));
                        self.stack_depth -= argc;
                    }
                } else {
                    let first = quasis[0].cooked.as_deref().unwrap_or("");
                    let idx = self.str_const(first);
                    self.emit(Op::PushConst(idx));
                    self.stack_depth += 1;
                    for (i, e) in exprs.iter().enumerate() {
                        self.compile_expr(e)?;
                        self.emit(Op::Binary(BinaryOp::Add));
                        self.stack_depth -= 1;
                        let part = quasis[i + 1].cooked.as_deref().unwrap_or("");
                        if !part.is_empty() {
                            let pidx = self.str_const(part);
                            self.emit(Op::PushConst(pidx));
                            self.stack_depth += 1;
                            self.emit(Op::Binary(BinaryOp::Add));
                            self.stack_depth -= 1;
                        }
                    }
                }
            }
            Expr::OptionalChain(inner) => {
                let mut short_circuits = Vec::new();
                self.compile_optional_inner(inner, &mut short_circuits)?;
                let j_done = self.emit(Op::Jump(0));
                let undef_ip = self.current_ip();
                for j in short_circuits {
                    self.chunk.patch_jump(j, undef_ip);
                }
                self.emit(Op::Pop);
                self.emit(Op::PushUndefined);
                let end_ip = self.current_ip();
                self.chunk.patch_jump(j_done, end_ip);
            }
            Expr::Member {
                object, property, ..
            } => {
                if matches!(object.as_ref(), Expr::Super) {
                    match property {
                        MemberProp::Ident(name) => {
                            let idx = self.str_const(name);
                            self.emit(Op::GetSuperProp(idx));
                            self.stack_depth += 1;
                        }
                        MemberProp::Computed(key) => {
                            self.compile_expr(key)?;
                            self.emit(Op::GetSuperElem);
                        }
                        MemberProp::Private(_) => {
                            return Err(JsError::syntax("Private member not allowed on super"));
                        }
                    }
                } else {
                    self.compile_expr(object)?;
                    self.compile_member_get(property)?;
                }
            }
            Expr::Call { callee, args, .. } => self.compile_call(callee, args)?,
            Expr::New { callee, args } => {
                self.compile_expr(callee)?;
                let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
                if !has_spread {
                    for arg in args {
                        if let CallArg::Expr(e) = arg {
                            self.compile_expr(e)?;
                        }
                    }
                    let argc = args.len() as u32;
                    self.emit(Op::New(argc));
                    self.stack_depth -= argc;
                } else {
                    self.compile_args_array(args)?;
                    self.emit(Op::NewSpread);
                    self.stack_depth -= 1;
                }
            }
            Expr::Unary { op, arg } => self.compile_unary(*op, arg)?,
            Expr::Update { op, arg, prefix } => self.compile_update(*op, arg, *prefix)?,
            Expr::Binary { op, left, right } => {
                self.compile_expr(left)?;
                self.compile_expr(right)?;
                self.emit(Op::Binary(*op));
                self.stack_depth -= 1;
            }
            Expr::Logical { op, left, right } => {
                self.compile_expr(left)?;
                self.emit(Op::Dup);
                self.stack_depth += 1;
                let j_short = match op {
                    LogicalOp::And => self.emit(Op::JumpIfFalse(0)),
                    LogicalOp::Or => self.emit(Op::JumpIfTrue(0)),
                    LogicalOp::Nullish => self.emit(Op::JumpIfNotNullish(0)),
                };
                self.stack_depth -= 1;
                self.emit(Op::Pop);
                self.stack_depth -= 1;
                self.compile_expr(right)?;
                let end_ip = self.current_ip();
                self.chunk.patch_jump(j_short, end_ip);
            }
            Expr::Conditional {
                test,
                consequent,
                alternate,
            } => {
                self.compile_expr(test)?;
                let j_alt = self.emit(Op::JumpIfFalse(0));
                self.stack_depth -= 1;
                self.compile_expr(consequent)?;
                let j_end = self.emit(Op::Jump(0));
                let alt_ip = self.current_ip();
                self.chunk.patch_jump(j_alt, alt_ip);
                self.stack_depth -= 1;
                self.compile_expr(alternate)?;
                let end_ip = self.current_ip();
                self.chunk.patch_jump(j_end, end_ip);
            }
            Expr::Assign { op, left, right } => self.compile_assign(*op, left, right)?,
            Expr::Sequence(exprs) => {
                let len = exprs.len();
                for (i, e) in exprs.iter().enumerate() {
                    self.compile_expr(e)?;
                    if i + 1 < len {
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                    }
                }
            }
            Expr::Yield { arg, delegate } => {
                if let Some(a) = arg {
                    self.compile_expr(a)?;
                } else {
                    self.emit(Op::PushUndefined);
                    self.stack_depth += 1;
                }
                if *delegate {
                    self.emit(Op::YieldDelegate);
                } else {
                    self.emit(Op::Yield);
                }
            }
            Expr::Await(inner) => {
                self.compile_expr(inner)?;
                self.emit(Op::Await);
            }
        }
        Ok(())
    }

    fn compile_optional_inner(
        &mut self,
        expr: &Expr,
        short_circuits: &mut Vec<usize>,
    ) -> Result<(), JsError> {
        match expr {
            Expr::Member {
                object,
                property,
                optional,
            } => {
                self.compile_optional_inner(object, short_circuits)?;
                if *optional {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    let j = self.emit(Op::JumpIfNullish(0));
                    self.stack_depth -= 1;
                    short_circuits.push(j);
                }
                self.compile_member_get(property)?;
            }
            Expr::Call {
                callee,
                args,
                optional,
            } => {
                if let Expr::Member {
                    object,
                    property,
                    optional: mem_opt,
                } = callee.as_ref()
                {
                    self.compile_optional_inner(object, short_circuits)?;
                    if *mem_opt {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        let j = self.emit(Op::JumpIfNullish(0));
                        self.stack_depth -= 1;
                        short_circuits.push(j);
                    }
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.compile_member_get(property)?;
                    if *optional {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        let j = self.emit(Op::JumpIfNullish(0));
                        self.stack_depth -= 1;
                        let j_ok = self.emit(Op::Jump(0));
                        let null_ip = self.current_ip();
                        self.chunk.patch_jump(j, null_ip);
                        self.emit(Op::Pop);
                        let j2 = self.emit(Op::Jump(0));
                        short_circuits.push(j2);
                        let ok_ip = self.current_ip();
                        self.chunk.patch_jump(j_ok, ok_ip);
                    }
                    self.emit(Op::Swap);
                    self.compile_call_args_and_emit(args, true)?;
                } else {
                    self.compile_optional_inner(callee, short_circuits)?;
                    if *optional {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        let j = self.emit(Op::JumpIfNullish(0));
                        self.stack_depth -= 1;
                        short_circuits.push(j);
                    }
                    self.compile_call_args_and_emit(args, false)?;
                }
            }
            other => self.compile_expr(other)?,
        }
        Ok(())
    }

    fn compile_member_get(&mut self, property: &MemberProp) -> Result<(), JsError> {
        match property {
            MemberProp::Ident(name) => {
                let idx = self.str_const(name);
                self.emit(Op::GetProp(idx));
            }
            MemberProp::Computed(key) => {
                self.compile_expr(key)?;
                self.emit(Op::GetElem);
                self.stack_depth -= 1;
            }
            MemberProp::Private(name) => {
                let idx = self.str_const(name);
                self.emit(Op::GetPrivate(idx));
            }
        }
        Ok(())
    }

    fn compile_call(&mut self, callee: &Expr, args: &[CallArg]) -> Result<(), JsError> {
        if matches!(callee, Expr::Super) {
            let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
            if !has_spread {
                for arg in args {
                    if let CallArg::Expr(e) = arg {
                        self.compile_expr(e)?;
                    }
                }
                let argc = args.len() as u32;
                self.emit(Op::CallSuper(argc));
                self.stack_depth = self.stack_depth + 1 - argc;
            } else {
                self.compile_args_array(args)?;
                self.emit(Op::CallSuperSpread);
            }
            return Ok(());
        }

        if let Expr::Ident(name) = callee {
            if name == "eval" && !args.iter().any(|a| matches!(a, CallArg::Spread(_))) {
                let idx = self.str_const("eval");
                self.emit(Op::GetVar(idx));
                self.stack_depth += 1;
                for arg in args {
                    if let CallArg::Expr(e) = arg {
                        self.compile_expr(e)?;
                    }
                }
                let argc = args.len() as u32;
                self.emit(Op::CallDirectEval(argc));
                self.stack_depth -= argc;
                return Ok(());
            }
        }

        if let Expr::Member {
            object, property, ..
        } = callee
        {
            if matches!(object.as_ref(), Expr::Super) {
                match property {
                    MemberProp::Ident(name) => {
                        let idx = self.str_const(name);
                        self.emit(Op::GetSuperProp(idx));
                        self.stack_depth += 1;
                    }
                    MemberProp::Computed(k) => {
                        self.compile_expr(k)?;
                        self.emit(Op::GetSuperElem);
                    }
                    MemberProp::Private(_) => {
                        return Err(JsError::syntax("Private member on super"));
                    }
                }
                self.emit(Op::PushThis);
                self.stack_depth += 1;
                return self.compile_call_args_and_emit(args, true);
            }
            self.compile_expr(object)?;
            self.emit(Op::Dup);
            self.stack_depth += 1;
            self.compile_member_get(property)?;
            self.emit(Op::Swap);
            return self.compile_call_args_and_emit(args, true);
        }

        self.compile_expr(callee)?;
        self.compile_call_args_and_emit(args, false)
    }

    fn compile_call_args_and_emit(
        &mut self,
        args: &[CallArg],
        is_method: bool,
    ) -> Result<(), JsError> {
        let has_spread = args.iter().any(|a| matches!(a, CallArg::Spread(_)));
        if !has_spread {
            for arg in args {
                if let CallArg::Expr(e) = arg {
                    self.compile_expr(e)?;
                }
            }
            let argc = args.len() as u32;
            if is_method {
                self.emit(Op::CallMethod(argc));
                self.stack_depth -= argc + 1;
            } else {
                self.emit(Op::Call(argc));
                self.stack_depth -= argc;
            }
        } else {
            self.compile_args_array(args)?;
            if is_method {
                self.emit(Op::CallMethodSpread);
                self.stack_depth -= 2;
            } else {
                self.emit(Op::CallSpread);
                self.stack_depth -= 1;
            }
        }
        Ok(())
    }

    fn compile_args_array(&mut self, args: &[CallArg]) -> Result<(), JsError> {
        self.emit(Op::CreateArray(0));
        self.stack_depth += 1;
        for arg in args {
            match arg {
                CallArg::Expr(e) => {
                    self.compile_expr(e)?;
                    self.emit(Op::ArrayPush);
                    self.stack_depth -= 1;
                }
                CallArg::Spread(e) => {
                    self.compile_expr(e)?;
                    self.emit(Op::ArraySpread);
                    self.stack_depth -= 1;
                }
            }
        }
        Ok(())
    }

    fn compile_unary(&mut self, op: UnaryOp, arg: &Expr) -> Result<(), JsError> {
        match op {
            UnaryOp::TypeOf => {
                if let Expr::Ident(name) = arg {
                    let idx = self.str_const(name);
                    self.emit(Op::GetVarTypeof(idx));
                    self.stack_depth += 1;
                } else {
                    self.compile_expr(arg)?;
                    self.emit(Op::Unary(UnaryOp::TypeOf));
                }
            }
            UnaryOp::Delete => match arg {
                Expr::Ident(name) => {
                    if self.strict {
                        return Err(JsError::syntax(
                            "Delete of an unqualified identifier in strict mode",
                        ));
                    }
                    let idx = self.str_const(name);
                    self.emit(Op::DeleteVar(idx));
                    self.stack_depth += 1;
                }
                Expr::Member {
                    object, property, ..
                } => {
                    self.compile_expr(object)?;
                    match property {
                        MemberProp::Ident(name) => {
                            let idx = self.str_const(name);
                            self.emit(Op::DeleteProp(idx));
                        }
                        MemberProp::Computed(k) => {
                            self.compile_expr(k)?;
                            self.emit(Op::DeleteElem);
                            self.stack_depth -= 1;
                        }
                        MemberProp::Private(_) => {
                            return Err(JsError::syntax("Private fields cannot be deleted"));
                        }
                    }
                }
                Expr::OptionalChain(inner) => {
                    if let Expr::Member {
                        object,
                        property,
                        optional,
                    } = inner.as_ref()
                    {
                        self.compile_expr(object)?;
                        if *optional {
                            self.emit(Op::Dup);
                            self.stack_depth += 1;
                            let j = self.emit(Op::JumpIfNullish(0));
                            self.stack_depth -= 1;
                            match property {
                                MemberProp::Ident(name) => {
                                    let idx = self.str_const(name);
                                    self.emit(Op::DeleteProp(idx));
                                }
                                MemberProp::Computed(k) => {
                                    self.compile_expr(k)?;
                                    self.emit(Op::DeleteElem);
                                    self.stack_depth -= 1;
                                }
                                MemberProp::Private(_) => {
                                    return Err(JsError::syntax("Private fields cannot be deleted"));
                                }
                            }
                            let j_end = self.emit(Op::Jump(0));
                            let null_ip = self.current_ip();
                            self.chunk.patch_jump(j, null_ip);
                            self.emit(Op::Pop);
                            self.emit(Op::PushTrue);
                            let end_ip = self.current_ip();
                            self.chunk.patch_jump(j_end, end_ip);
                        } else {
                            self.compile_unary(UnaryOp::Delete, inner)?;
                        }
                    } else {
                        self.compile_expr(arg)?;
                        self.emit(Op::Pop);
                        self.emit(Op::PushTrue);
                    }
                }
                _ => {
                    self.compile_expr(arg)?;
                    self.emit(Op::Pop);
                    self.emit(Op::PushTrue);
                }
            },
            other => {
                self.compile_expr(arg)?;
                self.emit(Op::Unary(other));
            }
        }
        Ok(())
    }

    fn compile_update(&mut self, op: UpdateOp, arg: &Expr, prefix: bool) -> Result<(), JsError> {
        let step_op = match op {
            UpdateOp::Inc => Op::Inc,
            UpdateOp::Dec => Op::Dec,
        };
        match arg {
            Expr::Ident(name) => {
                let idx = self.str_const(name);
                self.emit(Op::GetVar(idx));
                self.stack_depth += 1;
                self.emit(Op::ToNumber);
                if !prefix {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                }
                self.emit(step_op);
                if prefix {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                }
                self.emit(Op::SetVar(idx));
                self.stack_depth -= 1;
            }
            Expr::Member {
                object, property, ..
            } => match property {
                MemberProp::Ident(name) => {
                    let idx = self.str_const(name);
                    self.compile_expr(object)?;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.emit(Op::GetProp(idx));
                    self.emit(Op::ToNumber);
                    if !prefix {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.emit(Op::Rot3);
                        self.emit(Op::Rot3);
                        self.emit(step_op);
                        self.emit(Op::SetProp(idx));
                        self.stack_depth -= 1;
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                    } else {
                        self.emit(step_op);
                        self.emit(Op::SetProp(idx));
                        self.stack_depth -= 1;
                    }
                }
                MemberProp::Private(name) => {
                    let idx = self.str_const(name);
                    self.compile_expr(object)?;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.emit(Op::GetPrivate(idx));
                    self.emit(Op::ToNumber);
                    if !prefix {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.emit(Op::Rot3);
                        self.emit(Op::Rot3);
                        self.emit(step_op);
                        self.emit(Op::SetPrivate(idx));
                        self.stack_depth -= 1;
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                    } else {
                        self.emit(step_op);
                        self.emit(Op::SetPrivate(idx));
                        self.stack_depth -= 1;
                    }
                }
                MemberProp::Computed(key) => {
                    self.compile_expr(object)?;
                    self.compile_expr(key)?;
                    self.emit(Op::Dup2);
                    self.stack_depth += 2;
                    self.emit(Op::GetElem);
                    self.stack_depth -= 1;
                    self.emit(Op::ToNumber);
                    if !prefix {
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.emit(step_op);
                        let tmp = self.str_const("__upd_tmp__");
                        self.push_scope();
                        self.emit(Op::DeclareLet(tmp));
                        self.emit(Op::InitBinding(tmp));
                        self.stack_depth -= 1;
                        self.emit(Op::Rot3);
                        self.emit(Op::Rot3);
                        self.emit(Op::GetVar(tmp));
                        self.stack_depth += 1;
                        self.emit(Op::SetElem);
                        self.stack_depth -= 2;
                        self.emit(Op::Pop);
                        self.stack_depth -= 1;
                        self.pop_scope();
                    } else {
                        self.emit(step_op);
                        self.emit(Op::SetElem);
                        self.stack_depth -= 2;
                    }
                }
            },
            _ => return Err(JsError::syntax("Invalid left-hand side in update expression")),
        }
        Ok(())
    }

    fn compile_assign(
        &mut self,
        op: AssignOp,
        left: &AssignTarget,
        right: &Expr,
    ) -> Result<(), JsError> {
        if op == AssignOp::Assign {
            match left {
                AssignTarget::Ident(name) => {
                    self.compile_expr(right)?;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    let idx = self.str_const(name);
                    self.emit(Op::SetVar(idx));
                    self.stack_depth -= 1;
                }
                AssignTarget::Member { object, property } => {
                    if matches!(object.as_ref(), Expr::Super) {
                        match property {
                            MemberProp::Ident(name) => {
                                self.compile_expr(right)?;
                                let idx = self.str_const(name);
                                self.emit(Op::SetSuperProp(idx));
                            }
                            MemberProp::Computed(k) => {
                                self.compile_expr(k)?;
                                self.compile_expr(right)?;
                                self.emit(Op::SetSuperElem);
                                self.stack_depth -= 1;
                            }
                            MemberProp::Private(_) => {
                                return Err(JsError::syntax("Private field on super"));
                            }
                        }
                    } else {
                        self.compile_expr(object)?;
                        match property {
                            MemberProp::Ident(name) => {
                                self.compile_expr(right)?;
                                let idx = self.str_const(name);
                                self.emit(Op::SetProp(idx));
                                self.stack_depth -= 1;
                            }
                            MemberProp::Private(name) => {
                                self.compile_expr(right)?;
                                let idx = self.str_const(name);
                                self.emit(Op::SetPrivate(idx));
                                self.stack_depth -= 1;
                            }
                            MemberProp::Computed(k) => {
                                self.compile_expr(k)?;
                                self.compile_expr(right)?;
                                self.emit(Op::SetElem);
                                self.stack_depth -= 2;
                            }
                        }
                    }
                }
                AssignTarget::Pattern(pat) => {
                    self.compile_expr(right)?;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.compile_pattern_assign(pat)?;
                }
            }
            return Ok(());
        }

        let is_logical = matches!(op, AssignOp::And | AssignOp::Or | AssignOp::Nullish);
        match left {
            AssignTarget::Ident(name) => {
                let idx = self.str_const(name);
                self.emit(Op::GetVar(idx));
                self.stack_depth += 1;
                if is_logical {
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    let j_skip = match op {
                        AssignOp::And => self.emit(Op::JumpIfFalse(0)),
                        AssignOp::Or => self.emit(Op::JumpIfTrue(0)),
                        AssignOp::Nullish => self.emit(Op::JumpIfNotNullish(0)),
                        _ => unreachable!(),
                    };
                    self.stack_depth -= 1;
                    self.emit(Op::Pop);
                    self.stack_depth -= 1;
                    self.compile_expr(right)?;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.emit(Op::SetVar(idx));
                    self.stack_depth -= 1;
                    let end = self.current_ip();
                    self.chunk.patch_jump(j_skip, end);
                } else {
                    self.compile_expr(right)?;
                    self.emit(Op::Binary(assign_to_binary(op)));
                    self.stack_depth -= 1;
                    self.emit(Op::Dup);
                    self.stack_depth += 1;
                    self.emit(Op::SetVar(idx));
                    self.stack_depth -= 1;
                }
            }
            AssignTarget::Member { object, property } => {
                self.compile_expr(object)?;
                match property {
                    MemberProp::Ident(name) => {
                        let idx = self.str_const(name);
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.emit(Op::GetProp(idx));
                        if is_logical {
                            self.emit(Op::Dup);
                            self.stack_depth += 1;
                            let j_skip = match op {
                                AssignOp::And => self.emit(Op::JumpIfFalse(0)),
                                AssignOp::Or => self.emit(Op::JumpIfTrue(0)),
                                AssignOp::Nullish => self.emit(Op::JumpIfNotNullish(0)),
                                _ => unreachable!(),
                            };
                            self.stack_depth -= 1;
                            self.emit(Op::Pop);
                            self.stack_depth -= 1;
                            self.compile_expr(right)?;
                            self.emit(Op::SetProp(idx));
                            self.stack_depth -= 1;
                            let j_end = self.emit(Op::Jump(0));
                            let skip_ip = self.current_ip();
                            self.chunk.patch_jump(j_skip, skip_ip);
                            self.emit(Op::Swap);
                            self.emit(Op::Pop);
                            let end_ip = self.current_ip();
                            self.chunk.patch_jump(j_end, end_ip);
                        } else {
                            self.compile_expr(right)?;
                            self.emit(Op::Binary(assign_to_binary(op)));
                            self.stack_depth -= 1;
                            self.emit(Op::SetProp(idx));
                            self.stack_depth -= 1;
                        }
                    }
                    MemberProp::Private(name) => {
                        let idx = self.str_const(name);
                        self.emit(Op::Dup);
                        self.stack_depth += 1;
                        self.emit(Op::GetPrivate(idx));
                        if is_logical {
                            self.emit(Op::Dup);
                            self.stack_depth += 1;
                            let j_skip = match op {
                                AssignOp::And => self.emit(Op::JumpIfFalse(0)),
                                AssignOp::Or => self.emit(Op::JumpIfTrue(0)),
                                AssignOp::Nullish => self.emit(Op::JumpIfNotNullish(0)),
                                _ => unreachable!(),
                            };
                            self.stack_depth -= 1;
                            self.emit(Op::Pop);
                            self.stack_depth -= 1;
                            self.compile_expr(right)?;
                            self.emit(Op::SetPrivate(idx));
                            self.stack_depth -= 1;
                            let j_end = self.emit(Op::Jump(0));
                            let skip_ip = self.current_ip();
                            self.chunk.patch_jump(j_skip, skip_ip);
                            self.emit(Op::Swap);
                            self.emit(Op::Pop);
                            let end_ip = self.current_ip();
                            self.chunk.patch_jump(j_end, end_ip);
                        } else {
                            self.compile_expr(right)?;
                            self.emit(Op::Binary(assign_to_binary(op)));
                            self.stack_depth -= 1;
                            self.emit(Op::SetPrivate(idx));
                            self.stack_depth -= 1;
                        }
                    }
                    MemberProp::Computed(k) => {
                        self.compile_expr(k)?;
                        self.emit(Op::Dup2);
                        self.stack_depth += 2;
                        self.emit(Op::GetElem);
                        self.stack_depth -= 1;
                        if is_logical {
                            self.emit(Op::Dup);
                            self.stack_depth += 1;
                            let j_skip = match op {
                                AssignOp::And => self.emit(Op::JumpIfFalse(0)),
                                AssignOp::Or => self.emit(Op::JumpIfTrue(0)),
                                AssignOp::Nullish => self.emit(Op::JumpIfNotNullish(0)),
                                _ => unreachable!(),
                            };
                            self.stack_depth -= 1;
                            self.emit(Op::Pop);
                            self.stack_depth -= 1;
                            self.compile_expr(right)?;
                            self.emit(Op::SetElem);
                            self.stack_depth -= 2;
                            let j_end = self.emit(Op::Jump(0));
                            let skip_ip = self.current_ip();
                            self.chunk.patch_jump(j_skip, skip_ip);
                            self.emit(Op::Rot3);
                            self.emit(Op::Pop);
                            self.emit(Op::Pop);
                            let end_ip = self.current_ip();
                            self.chunk.patch_jump(j_end, end_ip);
                        } else {
                            self.compile_expr(right)?;
                            self.emit(Op::Binary(assign_to_binary(op)));
                            self.stack_depth -= 1;
                            self.emit(Op::SetElem);
                            self.stack_depth -= 2;
                        }
                    }
                }
            }
            AssignTarget::Pattern(_) => {
                return Err(JsError::syntax(
                    "Compound assignment not allowed on destructuring pattern",
                ));
            }
        }
        Ok(())
    }

    fn compile_class(&mut self, cls: &ClassNode) -> Result<(), JsError> {
        let has_super = cls.super_class.is_some();
        if let Some(ref sup) = cls.super_class {
            self.compile_expr(sup)?;
        }

        let mut constructor = None;
        let mut methods = Vec::new();
        let mut fields = Vec::new();
        let mut static_blocks = Vec::new();
        let mut computed_count = 0u32;

        for elem in &cls.elements {
            match elem {
                ClassElement::Method {
                    key,
                    kind,
                    func,
                    is_static,
                    is_private,
                } => {
                    let tmpl = Compiler::compile_function(func)?;
                    if *kind == MethodKind::Constructor && !*is_static {
                        constructor = Some(tmpl);
                        continue;
                    }
                    let (name, is_computed) = match key {
                        PropKey::Ident(s) | PropKey::String(s) | PropKey::Private(s) => {
                            (Some(s.clone()), false)
                        }
                        PropKey::Number(n) => {
                            (Some(crate::value::number_to_string(*n)), false)
                        }
                        PropKey::Computed(e) => {
                            self.compile_expr(e)?;
                            computed_count += 1;
                            (None, true)
                        }
                    };
                    methods.push(ClassMethodTemplate {
                        name,
                        is_computed,
                        is_static: *is_static,
                        is_private: *is_private,
                        kind: *kind,
                        func: tmpl,
                    });
                }
                ClassElement::Field {
                    key,
                    init,
                    is_static,
                    is_private,
                } => {
                    let (name, is_computed) = match key {
                        PropKey::Ident(s) | PropKey::String(s) | PropKey::Private(s) => {
                            (Some(s.clone()), false)
                        }
                        PropKey::Number(n) => {
                            (Some(crate::value::number_to_string(*n)), false)
                        }
                        PropKey::Computed(e) => {
                            self.compile_expr(e)?;
                            computed_count += 1;
                            (None, true)
                        }
                    };
                    let init_chunk = match init {
                        Some(e) => Some(Compiler::compile_expr_chunk(e, true)?),
                        None => None,
                    };
                    fields.push(ClassFieldTemplate {
                        name,
                        is_computed,
                        is_static: *is_static,
                        is_private: *is_private,
                        init: init_chunk,
                    });
                }
                ClassElement::StaticBlock(stmts) => {
                    static_blocks.push(Compiler::compile_block_chunk(stmts, true)?);
                }
            }
        }

        let tmpl = Rc::new(ClassTemplate {
            name: cls.name.clone(),
            has_super,
            constructor,
            methods,
            fields,
            static_blocks,
        });
        let idx = self.chunk.add_const(ConstValue::Class(tmpl));
        self.emit(Op::CreateClass(idx));
        let popped = computed_count + if has_super { 1 } else { 0 };
        self.stack_depth = self.stack_depth + 1 - popped;
        Ok(())
    }
}

fn assign_to_binary(op: AssignOp) -> BinaryOp {
    match op {
        AssignOp::Add => BinaryOp::Add,
        AssignOp::Sub => BinaryOp::Sub,
        AssignOp::Mul => BinaryOp::Mul,
        AssignOp::Div => BinaryOp::Div,
        AssignOp::Rem => BinaryOp::Rem,
        AssignOp::Exp => BinaryOp::Exp,
        AssignOp::Shl => BinaryOp::Shl,
        AssignOp::Shr => BinaryOp::Shr,
        AssignOp::UShr => BinaryOp::UShr,
        AssignOp::BitAnd => BinaryOp::BitAnd,
        AssignOp::BitOr => BinaryOp::BitOr,
        AssignOp::BitXor => BinaryOp::BitXor,
        _ => unreachable!(),
    }
}

fn parse_bigint_literal(s: &str) -> Result<i128, JsError> {
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i128::from_str_radix(hex, 16)
    } else if let Some(oct) = s.strip_prefix("0o").or_else(|| s.strip_prefix("0O")) {
        i128::from_str_radix(oct, 8)
    } else if let Some(bin) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        i128::from_str_radix(bin, 2)
    } else {
        s.parse::<i128>()
    }
    .map_err(|_| JsError::syntax(format!("BigInt literal out of range: {}", s)))
}

fn collect_hoisted(stmts: &[Stmt], chunk: &mut Chunk, strict: bool) -> Result<(), JsError> {
    for stmt in stmts {
        match stmt {
            Stmt::FuncDecl(func) => {
                if let Some(ref name) = func.name {
                    let tmpl = Compiler::compile_function(func)?;
                    chunk.hoisted_funcs.push((name.clone(), tmpl));
                }
            }
            Stmt::Export(ExportDecl::Decl(inner)) => {
                collect_hoisted_stmt(inner, chunk, strict, true)?;
            }
            _ => collect_hoisted_stmt(stmt, chunk, strict, false)?,
        }
    }
    Ok(())
}

fn collect_hoisted_stmt(
    stmt: &Stmt,
    chunk: &mut Chunk,
    strict: bool,
    top_level: bool,
) -> Result<(), JsError> {
    match stmt {
        Stmt::VarDecl {
            kind: VarKind::Var,
            decls,
        } => {
            let mut names = Vec::new();
            for d in decls {
                collect_pattern_names(&d.pattern, &mut names, false);
            }
            for (n, _) in names {
                if !chunk.hoisted_vars.contains(&n) {
                    chunk.hoisted_vars.push(n);
                }
            }
        }
        Stmt::FuncDecl(func) if top_level || !strict => {
            if let Some(ref name) = func.name {
                if top_level {
                    let tmpl = Compiler::compile_function(func)?;
                    chunk.hoisted_funcs.push((name.clone(), tmpl));
                } else if !chunk.hoisted_vars.contains(name) {
                    chunk.hoisted_vars.push(name.clone());
                }
            }
        }
        Stmt::Block(stmts) => {
            for s in stmts {
                collect_hoisted_stmt(s, chunk, strict, false)?;
            }
        }
        Stmt::If {
            consequent,
            alternate,
            ..
        } => {
            collect_hoisted_stmt(consequent, chunk, strict, false)?;
            if let Some(alt) = alternate {
                collect_hoisted_stmt(alt, chunk, strict, false)?;
            }
        }
        Stmt::While { body, .. }
        | Stmt::DoWhile { body, .. }
        | Stmt::Labeled { body, .. }
        | Stmt::With { body, .. } => {
            collect_hoisted_stmt(body, chunk, strict, false)?;
        }
        Stmt::For { init, body, .. } => {
            if let Some(ForInit::VarDecl {
                kind: VarKind::Var,
                decls,
            }) = init
            {
                let mut names = Vec::new();
                for d in decls {
                    collect_pattern_names(&d.pattern, &mut names, false);
                }
                for (n, _) in names {
                    if !chunk.hoisted_vars.contains(&n) {
                        chunk.hoisted_vars.push(n);
                    }
                }
            }
            collect_hoisted_stmt(body, chunk, strict, false)?;
        }
        Stmt::ForIn { left, body, .. } | Stmt::ForOf { left, body, .. } => {
            if let ForInOfLeft::VarDecl {
                kind: VarKind::Var,
                pattern,
            } = left
            {
                let mut names = Vec::new();
                collect_pattern_names(pattern, &mut names, false);
                for (n, _) in names {
                    if !chunk.hoisted_vars.contains(&n) {
                        chunk.hoisted_vars.push(n);
                    }
                }
            }
            collect_hoisted_stmt(body, chunk, strict, false)?;
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &c.consequent {
                    collect_hoisted_stmt(s, chunk, strict, false)?;
                }
            }
        }
        Stmt::Try {
            block,
            handler,
            finalizer,
        } => {
            for s in block {
                collect_hoisted_stmt(s, chunk, strict, false)?;
            }
            if let Some(h) = handler {
                for s in &h.body {
                    collect_hoisted_stmt(s, chunk, strict, false)?;
                }
            }
            if let Some(f) = finalizer {
                for s in f {
                    collect_hoisted_stmt(s, chunk, strict, false)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_block_lex(stmts: &[Stmt], out: &mut Vec<(String, bool)>) {
    for stmt in stmts {
        match stmt {
            Stmt::VarDecl {
                kind: VarKind::Let,
                decls,
            } => {
                for d in decls {
                    collect_pattern_names(&d.pattern, out, false);
                }
            }
            Stmt::VarDecl {
                kind: VarKind::Const,
                decls,
            } => {
                for d in decls {
                    collect_pattern_names(&d.pattern, out, true);
                }
            }
            Stmt::ClassDecl(cls) => {
                if let Some(ref name) = cls.name {
                    out.push((name.clone(), false));
                }
            }
            Stmt::Export(ExportDecl::Decl(inner)) => {
                collect_block_lex(std::slice::from_ref(inner.as_ref()), out);
            }
            Stmt::Export(ExportDecl::DefaultClass(cls)) => {
                if let Some(ref name) = cls.name {
                    out.push((name.clone(), false));
                }
            }
            Stmt::Export(ExportDecl::DefaultFunc(func)) => {
                if let Some(ref name) = func.name {
                    out.push((name.clone(), false));
                }
            }
            _ => {}
        }
    }
}

pub fn collect_pattern_names(pat: &Pattern, out: &mut Vec<(String, bool)>, is_const: bool) {
    match pat {
        Pattern::Ident(name) => out.push((name.clone(), is_const)),
        Pattern::Assign { left, .. } => collect_pattern_names(left, out, is_const),
        Pattern::Array { elements, rest } => {
            for el in elements.iter().flatten() {
                collect_pattern_names(el, out, is_const);
            }
            if let Some(r) = rest {
                collect_pattern_names(r, out, is_const);
            }
        }
        Pattern::Object { props, rest } => {
            for p in props {
                collect_pattern_names(&p.value, out, is_const);
            }
            if let Some(r) = rest {
                collect_pattern_names(r, out, is_const);
            }
        }
        Pattern::Member(_) => {}
    }
}
