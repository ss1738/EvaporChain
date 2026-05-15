use crate::{ScriptError, ScriptType, Value};

// ─── Token ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Contract,
    State,
    Fn,
    Let,
    If,
    Else,
    While,
    Return,
    SelfKw,
    True,
    False,
    OnEvaporate,
    OnGrace,
    OnRefresh,

    // Type keywords
    U64Type,
    BoolType,
    StringType,
    AddressType,
    MapType,
    ArrayType,

    // Identifiers and literals
    Ident(String),
    IntLit(u64),
    StrLit(String),

    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    EqEq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
    And,
    Or,
    Not,
    Assign,
    PlusAssign,
    MinusAssign,
    Arrow, // ->

    // Delimiters
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,

    // End
    Eof,
}

// ─── AST ────────────────────────────────────────────────────────────────────

/// Top-level contract definition.
#[derive(Debug, Clone)]
pub struct Contract {
    pub name: String,
    pub state_fields: Vec<StateField>,
    pub functions: Vec<Function>,
    pub lifecycle_hooks: Vec<LifecycleHook>,
}

/// A state field declaration.
#[derive(Debug, Clone)]
pub struct StateField {
    pub name: String,
    pub ty: ScriptType,
    pub default: Option<Expr>,
}

/// A function definition.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<(String, ScriptType)>,
    pub return_type: Option<ScriptType>,
    pub body: Vec<Stmt>,
}

/// Lifecycle hooks called by the chain.
#[derive(Debug, Clone)]
pub enum LifecycleHook {
    OnEvaporate(Vec<Stmt>),
    OnGrace(Vec<Stmt>),
    OnRefresh(Vec<Stmt>),
}

/// Statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    Let {
        name: String,
        value: Expr,
    },
    Assign {
        target: AssignTarget,
        value: Expr,
    },
    CompoundAssign {
        target: AssignTarget,
        op: BinOp,
        value: Expr,
    },
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    Require {
        condition: Expr,
        message: Expr,
    },
    Emit(Expr),
    ExprStmt(Expr),
}

/// Assignment target.
#[derive(Debug, Clone)]
pub enum AssignTarget {
    Variable(String),
    StateField(String),
    MapEntry(String, Box<Expr>),
    ArrayElement(String, Box<Expr>),
}

/// Binary operator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Gt,
    Lt,
    Gte,
    Lte,
    And,
    Or,
}

/// Unary operator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Not,
    Neg,
}

/// Expression.
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Value),
    Variable(String),
    StateAccess(String),
    MapAccess(String, Box<Expr>),
    BinaryOp {
        left: Box<Expr>,
        op: BinOp,
        right: Box<Expr>,
    },
    UnaryOp {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
    ArrayLiteral(Vec<Expr>),
    ArrayAccess(Box<Expr>, Box<Expr>),
}

// ─── Lexer ──────────────────────────────────────────────────────────────────

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
}

impl Lexer {
    fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        if let Some(c) = ch {
            self.pos += 1;
            if c == '\n' {
                self.line += 1;
            }
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Skip whitespace
            while self.peek().is_some_and(|c| c.is_whitespace()) {
                self.advance();
            }
            // Skip line comments
            if self.pos + 1 < self.chars.len()
                && self.chars[self.pos] == '/'
                && self.chars[self.pos + 1] == '/'
            {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.advance();
                }
                continue;
            }
            break;
        }
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.advance();
        }
        self.chars[start..self.pos].iter().collect()
    }

    fn read_number(&mut self) -> Result<u64, ScriptError> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.advance();
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        s.parse().map_err(|_| ScriptError::Parse {
            line: self.line,
            message: format!("integer literal overflows u64: {s}"),
        })
    }

    fn read_string(&mut self) -> Result<String, ScriptError> {
        // Opening quote already consumed
        // Audit O2 (2026-05-15): cap string literal length.  The 1M-token limit
        // does not bound a single token's size; an adversarial contract with a
        // multi-MiB string literal allocates unbounded memory during tokenisation.
        const MAX_STRING_LITERAL: usize = 65_536; // 64 KiB per literal
        let mut s = String::new();
        loop {
            match self.advance() {
                Some('"') => return Ok(s),
                Some('\\') => match self.advance() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('\\') => s.push('\\'),
                    Some('"') => s.push('"'),
                    Some(c) => s.push(c),
                    None => {
                        return Err(ScriptError::Parse {
                            line: self.line,
                            message: "unterminated escape in string".into(),
                        })
                    }
                },
                Some(c) => {
                    if s.len() >= MAX_STRING_LITERAL {
                        return Err(ScriptError::Parse {
                            line: self.line,
                            message: format!(
                                "string literal too large: {} bytes (max {MAX_STRING_LITERAL})",
                                s.len()
                            ),
                        });
                    }
                    s.push(c);
                }
                None => {
                    return Err(ScriptError::Parse {
                        line: self.line,
                        message: "unterminated string literal".into(),
                    })
                }
            }
        }
    }

    fn next_token(&mut self) -> Result<(Token, usize), ScriptError> {
        self.skip_whitespace_and_comments();
        let line = self.line;

        let ch = match self.peek() {
            Some(c) => c,
            None => return Ok((Token::Eof, line)),
        };

        // Identifiers and keywords
        if ch.is_alphabetic() || ch == '_' {
            let ident = self.read_ident();
            let tok = match ident.as_str() {
                "contract" => Token::Contract,
                "state" => Token::State,
                "fn" => Token::Fn,
                "let" => Token::Let,
                "if" => Token::If,
                "else" => Token::Else,
                "while" => Token::While,
                "return" => Token::Return,
                "self" => Token::SelfKw,
                "true" => Token::True,
                "false" => Token::False,
                "on_evaporate" => Token::OnEvaporate,
                "on_grace" => Token::OnGrace,
                "on_refresh" => Token::OnRefresh,
                "u64" => Token::U64Type,
                "bool" => Token::BoolType,
                "string" => Token::StringType,
                "address" => Token::AddressType,
                "map" => Token::MapType,
                "array" => Token::ArrayType,
                _ => Token::Ident(ident),
            };
            return Ok((tok, line));
        }

        // Number literals
        if ch.is_ascii_digit() {
            let n = self.read_number()?;
            return Ok((Token::IntLit(n), line));
        }

        // String literals
        if ch == '"' {
            self.advance(); // consume opening quote
            let s = self.read_string()?;
            return Ok((Token::StrLit(s), line));
        }

        // Operators and punctuation
        self.advance(); // consume current char
        let tok = match ch {
            '+' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::PlusAssign
                } else {
                    Token::Plus
                }
            }
            '-' => {
                if self.peek() == Some('>') {
                    self.advance();
                    Token::Arrow
                } else if self.peek() == Some('=') {
                    self.advance();
                    Token::MinusAssign
                } else {
                    Token::Minus
                }
            }
            '*' => Token::Star,
            '/' => Token::Slash,
            '%' => Token::Percent,
            '=' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::EqEq
                } else {
                    Token::Assign
                }
            }
            '!' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::Neq
                } else {
                    Token::Not
                }
            }
            '>' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::Gte
                } else {
                    Token::Gt
                }
            }
            '<' => {
                if self.peek() == Some('=') {
                    self.advance();
                    Token::Lte
                } else {
                    Token::Lt
                }
            }
            '&' => {
                if self.peek() == Some('&') {
                    self.advance();
                    Token::And
                } else {
                    return Err(ScriptError::Parse {
                        line,
                        message: "expected '&&'".into(),
                    });
                }
            }
            '|' => {
                if self.peek() == Some('|') {
                    self.advance();
                    Token::Or
                } else {
                    return Err(ScriptError::Parse {
                        line,
                        message: "expected '||'".into(),
                    });
                }
            }
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '(' => Token::LParen,
            ')' => Token::RParen,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            ',' => Token::Comma,
            ':' => Token::Colon,
            '.' => Token::Dot,
            _ => {
                return Err(ScriptError::Parse {
                    line,
                    message: format!("unexpected character: '{ch}'"),
                })
            }
        };

        Ok((tok, line))
    }

    fn tokenize(&mut self) -> Result<Vec<(Token, usize)>, ScriptError> {
        // Re-audit (2026-05-02): cap token count to bound parser
        // memory. Without this, a pathological source with millions
        // of single-char tokens (e.g., `((((...((((` repeated) can
        // allocate unbounded `Vec<Token>` before any depth-limit or
        // size check fires. Conservative cap — 1M tokens is far
        // beyond any reasonable contract source.
        const MAX_TOKENS: usize = 1_000_000;
        let mut tokens = Vec::new();
        loop {
            let (tok, line) = self.next_token()?;
            let is_eof = tok == Token::Eof;
            tokens.push((tok, line));
            if tokens.len() > MAX_TOKENS {
                return Err(ScriptError::Runtime(format!(
                    "source too large: token count exceeds {MAX_TOKENS}"
                )));
            }
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }
}

// ─── Parser ─────────────────────────────────────────────────────────────────

const MAX_EXPR_DEPTH: usize = 64;

struct Parser {
    tokens: Vec<(Token, usize)>,
    pos: usize,
    expr_depth: usize,
}

impl Parser {
    fn new(tokens: Vec<(Token, usize)>) -> Self {
        Self {
            tokens,
            pos: 0,
            expr_depth: 0,
        }
    }

    fn peek(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .map(|(t, _)| t)
            .unwrap_or(&Token::Eof)
    }

    fn line(&self) -> usize {
        self.tokens.get(self.pos).map(|(_, l)| *l).unwrap_or(0)
    }

    fn advance(&mut self) -> Token {
        let tok = self
            .tokens
            .get(self.pos)
            .map(|(t, _)| t.clone())
            .unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ScriptError> {
        let line = self.line();
        let tok = self.advance();
        if std::mem::discriminant(&tok) != std::mem::discriminant(expected) {
            return Err(ScriptError::Parse {
                line,
                message: format!("expected {expected:?}, got {tok:?}"),
            });
        }
        Ok(())
    }

    fn expect_ident(&mut self) -> Result<String, ScriptError> {
        let line = self.line();
        match self.advance() {
            Token::Ident(name) => Ok(name),
            tok => Err(ScriptError::Parse {
                line,
                message: format!("expected identifier, got {tok:?}"),
            }),
        }
    }

    // ─── Contract ───

    fn parse_contract(&mut self) -> Result<Contract, ScriptError> {
        self.expect(&Token::Contract)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut state_fields = Vec::new();
        let mut functions = Vec::new();
        let mut lifecycle_hooks = Vec::new();

        while *self.peek() != Token::RBrace && *self.peek() != Token::Eof {
            match self.peek().clone() {
                Token::State => {
                    if !state_fields.is_empty() {
                        return Err(ScriptError::Compile(
                            "duplicate state block — merge all fields into a single state {} block"
                                .into(),
                        ));
                    }
                    state_fields = self.parse_state_block()?;
                }
                Token::Fn => {
                    functions.push(self.parse_function()?);
                }
                Token::OnEvaporate => {
                    self.advance();
                    self.expect(&Token::LParen)?;
                    self.expect(&Token::RParen)?;
                    let body = self.parse_block()?;
                    lifecycle_hooks.push(LifecycleHook::OnEvaporate(body));
                }
                Token::OnGrace => {
                    self.advance();
                    self.expect(&Token::LParen)?;
                    self.expect(&Token::RParen)?;
                    let body = self.parse_block()?;
                    lifecycle_hooks.push(LifecycleHook::OnGrace(body));
                }
                Token::OnRefresh => {
                    self.advance();
                    self.expect(&Token::LParen)?;
                    self.expect(&Token::RParen)?;
                    let body = self.parse_block()?;
                    lifecycle_hooks.push(LifecycleHook::OnRefresh(body));
                }
                tok => {
                    return Err(ScriptError::Parse {
                        line: self.line(),
                        message: format!("expected state, fn, or lifecycle hook, got {tok:?}"),
                    });
                }
            }
        }

        self.expect(&Token::RBrace)?;

        Ok(Contract {
            name,
            state_fields,
            functions,
            lifecycle_hooks,
        })
    }

    // ─── State Block ───

    fn parse_state_block(&mut self) -> Result<Vec<StateField>, ScriptError> {
        self.expect(&Token::State)?;
        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();
        while *self.peek() != Token::RBrace {
            let name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;

            let default = if *self.peek() == Token::Assign {
                self.advance();
                Some(self.parse_expr()?)
            } else {
                None
            };

            fields.push(StateField { name, ty, default });
        }

        self.expect(&Token::RBrace)?;
        Ok(fields)
    }

    // ─── Types ───

    fn parse_type(&mut self) -> Result<ScriptType, ScriptError> {
        let line = self.line();
        match self.advance() {
            Token::U64Type => Ok(ScriptType::U64),
            Token::BoolType => Ok(ScriptType::Bool),
            Token::StringType => Ok(ScriptType::String),
            Token::AddressType => Ok(ScriptType::Address),
            Token::MapType => {
                self.expect(&Token::LBracket)?;
                let key_ty = self.parse_type()?;
                self.expect(&Token::Arrow)?;
                let val_ty = self.parse_type()?;
                self.expect(&Token::RBracket)?;
                Ok(ScriptType::Map(Box::new(key_ty), Box::new(val_ty)))
            }
            Token::ArrayType => {
                self.expect(&Token::LBracket)?;
                let elem_ty = self.parse_type()?;
                self.expect(&Token::RBracket)?;
                Ok(ScriptType::Array(Box::new(elem_ty)))
            }
            tok => Err(ScriptError::Parse {
                line,
                message: format!("expected type, got {tok:?}"),
            }),
        }
    }

    // ─── Function ───

    fn parse_function(&mut self) -> Result<Function, ScriptError> {
        self.expect(&Token::Fn)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;

        let mut params = Vec::new();
        while *self.peek() != Token::RParen {
            if !params.is_empty() {
                self.expect(&Token::Comma)?;
            }
            let pname = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let pty = self.parse_type()?;
            params.push((pname, pty));
        }
        self.expect(&Token::RParen)?;

        let return_type = if *self.peek() == Token::Arrow {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        let body = self.parse_block()?;

        Ok(Function {
            name,
            params,
            return_type,
            body,
        })
    }

    // ─── Block ───

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ScriptError> {
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while *self.peek() != Token::RBrace && *self.peek() != Token::Eof {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    // ─── Statement ───

    fn parse_stmt(&mut self) -> Result<Stmt, ScriptError> {
        match self.peek().clone() {
            Token::Let => self.parse_let(),
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::Return => self.parse_return(),
            Token::SelfKw => self.parse_self_assign_or_expr(),
            Token::Ident(ref name) if name == "require" => self.parse_require(),
            Token::Ident(ref name) if name == "emit" => self.parse_emit(),
            Token::Ident(_) => self.parse_ident_stmt(),
            _ => {
                let expr = self.parse_expr()?;
                Ok(Stmt::ExprStmt(expr))
            }
        }
    }

    /// Parse a statement starting with an identifier.
    /// Handles: `x = expr`, `x += expr`, `x -= expr`, or falls back to expr statement.
    fn parse_ident_stmt(&mut self) -> Result<Stmt, ScriptError> {
        let name = self.expect_ident()?;

        if *self.peek() == Token::LBracket {
            self.advance();
            let index = self.parse_expr()?;
            self.expect(&Token::RBracket)?;
            match self.peek().clone() {
                Token::Assign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    return Ok(Stmt::Assign {
                        target: AssignTarget::ArrayElement(name, Box::new(index)),
                        value,
                    });
                }
                Token::PlusAssign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    return Ok(Stmt::CompoundAssign {
                        target: AssignTarget::ArrayElement(name, Box::new(index)),
                        op: BinOp::Add,
                        value,
                    });
                }
                Token::MinusAssign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    return Ok(Stmt::CompoundAssign {
                        target: AssignTarget::ArrayElement(name, Box::new(index)),
                        op: BinOp::Sub,
                        value,
                    });
                }
                _ => {
                    return Ok(Stmt::ExprStmt(Expr::ArrayAccess(
                        Box::new(Expr::Variable(name)),
                        Box::new(index),
                    )));
                }
            }
        }

        match self.peek().clone() {
            Token::Assign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::Assign {
                    target: AssignTarget::Variable(name),
                    value,
                })
            }
            Token::PlusAssign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::CompoundAssign {
                    target: AssignTarget::Variable(name),
                    op: BinOp::Add,
                    value,
                })
            }
            Token::MinusAssign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::CompoundAssign {
                    target: AssignTarget::Variable(name),
                    op: BinOp::Sub,
                    value,
                })
            }
            _ => {
                // Not an assignment — reparse as expression statement.
                // We already consumed the ident, so construct the expr manually.
                // Check if it's a function call: name(...)
                if *self.peek() == Token::LParen {
                    self.advance(); // consume '('
                    let mut args = Vec::new();
                    if *self.peek() != Token::RParen {
                        args.push(self.parse_expr()?);
                        while *self.peek() == Token::Comma {
                            self.advance();
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Stmt::ExprStmt(Expr::FunctionCall { name, args }))
                } else {
                    Ok(Stmt::ExprStmt(Expr::Variable(name)))
                }
            }
        }
    }

    fn parse_let(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'let'
        let name = self.expect_ident()?;

        // Optional type annotation
        if *self.peek() == Token::Colon {
            self.advance();
            let _ty = self.parse_type()?;
        }

        self.expect(&Token::Assign)?;
        let value = self.parse_expr()?;
        Ok(Stmt::Let { name, value })
    }

    fn parse_if(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'if'

        // Allow optional parens around condition
        let has_paren = *self.peek() == Token::LParen;
        if has_paren {
            self.advance();
        }
        let condition = self.parse_expr()?;
        if has_paren {
            self.expect(&Token::RParen)?;
        }

        let then_body = self.parse_block()?;

        let else_body = if *self.peek() == Token::Else {
            self.advance();
            if *self.peek() == Token::If {
                // else if
                let nested_if = self.parse_if()?;
                Some(vec![nested_if])
            } else {
                Some(self.parse_block()?)
            }
        } else {
            None
        };

        Ok(Stmt::If {
            condition,
            then_body,
            else_body,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'while'

        // Allow optional parens around condition
        let has_paren = *self.peek() == Token::LParen;
        if has_paren {
            self.advance();
        }
        let condition = self.parse_expr()?;
        if has_paren {
            self.expect(&Token::RParen)?;
        }

        let body = self.parse_block()?;

        Ok(Stmt::While { condition, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'return'
        if *self.peek() == Token::RBrace {
            return Ok(Stmt::Return(None));
        }
        let expr = self.parse_expr()?;
        Ok(Stmt::Return(Some(expr)))
    }

    fn parse_require(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'require'
        self.expect(&Token::LParen)?;
        let condition = self.parse_expr()?;
        self.expect(&Token::Comma)?;
        let message = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        Ok(Stmt::Require { condition, message })
    }

    fn parse_emit(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'emit'
        self.expect(&Token::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        Ok(Stmt::Emit(expr))
    }

    fn parse_self_assign_or_expr(&mut self) -> Result<Stmt, ScriptError> {
        self.advance(); // consume 'self'
        self.expect(&Token::Dot)?;
        let field = self.expect_ident()?;

        // Check for map access: self.field[key]
        if *self.peek() == Token::LBracket {
            self.advance(); // consume '['
            let key_expr = self.parse_expr()?;
            self.expect(&Token::RBracket)?;

            let target = AssignTarget::MapEntry(field, Box::new(key_expr));

            return match self.peek().clone() {
                Token::Assign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    Ok(Stmt::Assign { target, value })
                }
                Token::PlusAssign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    Ok(Stmt::CompoundAssign {
                        target,
                        op: BinOp::Add,
                        value,
                    })
                }
                Token::MinusAssign => {
                    self.advance();
                    let value = self.parse_expr()?;
                    Ok(Stmt::CompoundAssign {
                        target,
                        op: BinOp::Sub,
                        value,
                    })
                }
                _ => {
                    // Expression statement: self.field[key] used as expression
                    // Reconstruct the expression
                    Err(ScriptError::Parse {
                        line: self.line(),
                        message: "expected assignment operator after map access".into(),
                    })
                }
            };
        }

        // Simple field: self.field = ... or self.field += ...
        let target = AssignTarget::StateField(field);

        match self.peek().clone() {
            Token::Assign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::Assign { target, value })
            }
            Token::PlusAssign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::CompoundAssign {
                    target,
                    op: BinOp::Add,
                    value,
                })
            }
            Token::MinusAssign => {
                self.advance();
                let value = self.parse_expr()?;
                Ok(Stmt::CompoundAssign {
                    target,
                    op: BinOp::Sub,
                    value,
                })
            }
            _ => Err(ScriptError::Parse {
                line: self.line(),
                message: "expected assignment operator after self.field".into(),
            }),
        }
    }

    // ─── Expression Parsing (Pratt parser) ───

    fn parse_expr(&mut self) -> Result<Expr, ScriptError> {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            return Err(ScriptError::Parse {
                line: self.line(),
                message: format!("expression nesting depth exceeds maximum ({MAX_EXPR_DEPTH})"),
            });
        }
        let result = self.parse_or();
        self.expr_depth -= 1;
        result
    }

    fn parse_or(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_and()?;
        while *self.peek() == Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::Or,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_equality()?;
        while *self.peek() == Token::And {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op: BinOp::And,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_comparison()?;
        loop {
            let op = match self.peek() {
                Token::EqEq => BinOp::Eq,
                Token::Neq => BinOp::Neq,
                _ => break,
            };
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Token::Gt => BinOp::Gt,
                Token::Lt => BinOp::Lt,
                Token::Gte => BinOp::Gte,
                Token::Lte => BinOp::Lte,
                _ => break,
            };
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_multiplicative()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, ScriptError> {
        let mut left = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.parse_unary()?;
            left = Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, ScriptError> {
        match self.peek() {
            Token::Not => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            Token::Minus => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::UnaryOp {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, ScriptError> {
        let line = self.line();
        match self.peek().clone() {
            Token::IntLit(n) => {
                self.advance();
                Ok(Expr::Literal(Value::U64(n)))
            }
            Token::StrLit(s) => {
                self.advance();
                Ok(Expr::Literal(Value::Str(s)))
            }
            Token::True => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(true)))
            }
            Token::False => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(false)))
            }
            Token::SelfKw => {
                self.advance();
                self.expect(&Token::Dot)?;
                let field = self.expect_ident()?;
                if *self.peek() == Token::LBracket {
                    self.advance();
                    let key = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    Ok(Expr::MapAccess(field, Box::new(key)))
                } else {
                    Ok(Expr::StateAccess(field))
                }
            }
            Token::Ident(name) => {
                self.advance();
                if *self.peek() == Token::LParen {
                    self.advance();
                    let mut args = Vec::new();
                    while *self.peek() != Token::RParen {
                        if !args.is_empty() {
                            self.expect(&Token::Comma)?;
                        }
                        args.push(self.parse_expr()?);
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::FunctionCall { name, args })
                } else if *self.peek() == Token::LBracket {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    Ok(Expr::ArrayAccess(
                        Box::new(Expr::Variable(name)),
                        Box::new(index),
                    ))
                } else {
                    Ok(Expr::Variable(name))
                }
            }
            Token::LBracket => {
                self.advance();
                let mut elements = Vec::new();
                while *self.peek() != Token::RBracket {
                    if !elements.is_empty() {
                        self.expect(&Token::Comma)?;
                    }
                    elements.push(self.parse_expr()?);
                }
                self.expect(&Token::RBracket)?;
                Ok(Expr::ArrayLiteral(elements))
            }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            tok => Err(ScriptError::Parse {
                line,
                message: format!("unexpected token in expression: {tok:?}"),
            }),
        }
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Parse EvaporScript source code into a Contract AST.
pub fn parse(source: &str) -> Result<Contract, ScriptError> {
    let mut lexer = Lexer::new(source);
    let tokens = lexer.tokenize()?;
    let mut parser = Parser::new(tokens);
    parser.parse_contract()
}

/// Tokenize source code (exposed for testing).
pub fn tokenize(source: &str) -> Result<Vec<(Token, usize)>, ScriptError> {
    let mut lexer = Lexer::new(source);
    lexer.tokenize()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const LOYALTY_POINTS: &str = r#"
contract LoyaltyPoints {
    state {
        name: string = "ShopPoints"
        points: map[address -> u64]
        total_issued: u64 = 0
    }

    fn issue(to: address, amount: u64) {
        require(caller == owner, "only owner")
        self.points[to] += amount
        self.total_issued += amount
    }

    fn spend(amount: u64) {
        require(self.points[caller] >= amount, "insufficient points")
        self.points[caller] -= amount
    }

    fn balance(addr: address) -> u64 {
        return self.points[addr]
    }

    on_evaporate() {
        emit("loyalty program expired")
    }
}
"#;

    #[test]
    fn test_parse_simple_contract() {
        let contract = parse(LOYALTY_POINTS).unwrap();
        assert_eq!(contract.name, "LoyaltyPoints");
        assert_eq!(contract.state_fields.len(), 3);
        assert_eq!(contract.functions.len(), 3);
        assert_eq!(contract.lifecycle_hooks.len(), 1);
    }

    #[test]
    fn test_parse_state_fields() {
        let contract = parse(LOYALTY_POINTS).unwrap();

        assert_eq!(contract.state_fields[0].name, "name");
        assert_eq!(contract.state_fields[0].ty, ScriptType::String);
        assert!(contract.state_fields[0].default.is_some());

        assert_eq!(contract.state_fields[1].name, "points");
        assert_eq!(
            contract.state_fields[1].ty,
            ScriptType::Map(Box::new(ScriptType::Address), Box::new(ScriptType::U64))
        );

        assert_eq!(contract.state_fields[2].name, "total_issued");
        assert_eq!(contract.state_fields[2].ty, ScriptType::U64);
    }

    #[test]
    fn test_parse_functions() {
        let contract = parse(LOYALTY_POINTS).unwrap();

        let issue = &contract.functions[0];
        assert_eq!(issue.name, "issue");
        assert_eq!(issue.params.len(), 2);
        assert_eq!(issue.params[0].0, "to");
        assert_eq!(issue.params[0].1, ScriptType::Address);
        assert_eq!(issue.params[1].0, "amount");
        assert_eq!(issue.params[1].1, ScriptType::U64);
        assert!(issue.return_type.is_none());
        assert_eq!(issue.body.len(), 3);

        let balance_fn = &contract.functions[2];
        assert_eq!(balance_fn.name, "balance");
        assert_eq!(balance_fn.return_type, Some(ScriptType::U64));
    }

    #[test]
    fn test_parse_if_else() {
        let src = r#"
contract Test {
    state {
        x: u64 = 0
    }
    fn check(val: u64) {
        if val > 10 {
            self.x = 1
        } else {
            self.x = 0
        }
    }
}
"#;
        let contract = parse(src).unwrap();
        let check = &contract.functions[0];
        assert_eq!(check.body.len(), 1);
        match &check.body[0] {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                assert_eq!(then_body.len(), 1);
                assert!(else_body.is_some());
                assert_eq!(else_body.as_ref().unwrap().len(), 1);
            }
            other => panic!("expected if statement, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_map_access() {
        let src = r#"
contract Balances {
    state {
        balances: map[address -> u64]
    }
    fn get(addr: address) -> u64 {
        return self.balances[addr]
    }
    fn set(addr: address, val: u64) {
        self.balances[addr] = val
    }
}
"#;
        let contract = parse(src).unwrap();
        assert_eq!(contract.functions.len(), 2);

        // Check getter has a return with map access
        match &contract.functions[0].body[0] {
            Stmt::Return(Some(Expr::MapAccess(field, _))) => {
                assert_eq!(field, "balances");
            }
            other => panic!("expected return with map access, got {other:?}"),
        }

        // Check setter has an assign to map entry
        match &contract.functions[1].body[0] {
            Stmt::Assign {
                target: AssignTarget::MapEntry(field, _),
                ..
            } => {
                assert_eq!(field, "balances");
            }
            other => panic!("expected map assign, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_lifecycle_hooks() {
        let contract = parse(LOYALTY_POINTS).unwrap();
        assert_eq!(contract.lifecycle_hooks.len(), 1);
        match &contract.lifecycle_hooks[0] {
            LifecycleHook::OnEvaporate(body) => {
                assert_eq!(body.len(), 1);
                match &body[0] {
                    Stmt::Emit(_) => {}
                    other => panic!("expected emit, got {other:?}"),
                }
            }
            other => panic!("expected on_evaporate, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_all_lifecycle_hooks() {
        let src = r#"
contract Full {
    state { x: u64 = 0 }
    on_evaporate() { emit("evaporated") }
    on_grace() { emit("grace") }
    on_refresh() { emit("refreshed") }
}
"#;
        let contract = parse(src).unwrap();
        assert_eq!(contract.lifecycle_hooks.len(), 3);
    }

    #[test]
    fn test_parse_require_statement() {
        let src = r#"
contract Auth {
    state { owner_set: bool = false }
    fn check() {
        require(caller == owner, "not authorized")
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Require { condition, message } => {
                match condition {
                    Expr::BinaryOp { op: BinOp::Eq, .. } => {}
                    other => panic!("expected == comparison, got {other:?}"),
                }
                match message {
                    Expr::Literal(Value::Str(s)) => assert_eq!(s, "not authorized"),
                    other => panic!("expected string literal, got {other:?}"),
                }
            }
            other => panic!("expected require, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_let_statement() {
        let src = r#"
contract Vars {
    state { x: u64 = 0 }
    fn compute(a: u64, b: u64) -> u64 {
        let sum = a + b
        let doubled = sum * 2
        return doubled
    }
}
"#;
        let contract = parse(src).unwrap();
        assert_eq!(contract.functions[0].body.len(), 3);
        match &contract.functions[0].body[0] {
            Stmt::Let { name, .. } => assert_eq!(name, "sum"),
            other => panic!("expected let, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_compound_assignment() {
        let src = r#"
contract Counter {
    state { count: u64 = 0 }
    fn increment(n: u64) {
        self.count += n
    }
    fn decrement(n: u64) {
        self.count -= n
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::CompoundAssign {
                target: AssignTarget::StateField(f),
                op: BinOp::Add,
                ..
            } => assert_eq!(f, "count"),
            other => panic!("expected compound add, got {other:?}"),
        }
        match &contract.functions[1].body[0] {
            Stmt::CompoundAssign {
                target: AssignTarget::StateField(f),
                op: BinOp::Sub,
                ..
            } => assert_eq!(f, "count"),
            other => panic!("expected compound sub, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_nested_expressions() {
        let src = r#"
contract Math {
    state { x: u64 = 0 }
    fn test(a: u64, b: u64, c: u64) -> u64 {
        return (a + b) * c
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Return(Some(Expr::BinaryOp { op: BinOp::Mul, .. })) => {}
            other => panic!("expected multiply, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_boolean_operators() {
        let src = r#"
contract Logic {
    state { x: bool = false }
    fn test(a: bool, b: bool) -> bool {
        return a && b || !a
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Return(Some(Expr::BinaryOp { op: BinOp::Or, .. })) => {}
            other => panic!("expected or expression, got {other:?}"),
        }
    }

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("contract Foo { }").unwrap();
        assert_eq!(tokens[0].0, Token::Contract);
        assert_eq!(tokens[1].0, Token::Ident("Foo".into()));
        assert_eq!(tokens[2].0, Token::LBrace);
        assert_eq!(tokens[3].0, Token::RBrace);
        assert_eq!(tokens[4].0, Token::Eof);
    }

    #[test]
    fn test_parse_empty_contract() {
        let src = "contract Empty { }";
        let contract = parse(src).unwrap();
        assert_eq!(contract.name, "Empty");
        assert!(contract.state_fields.is_empty());
        assert!(contract.functions.is_empty());
        assert!(contract.lifecycle_hooks.is_empty());
    }

    #[test]
    fn test_parse_function_call_in_expr() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn test() -> u64 {
        return balance(caller)
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Return(Some(Expr::FunctionCall { name, args })) => {
                assert_eq!(name, "balance");
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected function call, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_array_literal() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn make() {
        let arr = [1, 2, 3]
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Let {
                name,
                value: Expr::ArrayLiteral(elems),
            } => {
                assert_eq!(name, "arr");
                assert_eq!(elems.len(), 3);
            }
            other => panic!("expected let with array literal, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_array_access() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn get() -> u64 {
        return arr[0]
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Return(Some(Expr::ArrayAccess(_, _))) => {}
            other => panic!("expected return with array access, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_array_element_assign() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn set() {
        arr[0] = 42
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Assign {
                target: AssignTarget::ArrayElement(name, _),
                ..
            } => {
                assert_eq!(name, "arr");
            }
            other => panic!("expected array element assign, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_array_type() {
        let src = r#"
contract Test {
    state { items: array[u64] }
}
"#;
        let contract = parse(src).unwrap();
        assert_eq!(
            contract.state_fields[0].ty,
            ScriptType::Array(Box::new(ScriptType::U64))
        );
    }

    #[test]
    fn test_parse_empty_array() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn empty() {
        let arr = []
    }
}
"#;
        let contract = parse(src).unwrap();
        match &contract.functions[0].body[0] {
            Stmt::Let {
                value: Expr::ArrayLiteral(elems),
                ..
            } => {
                assert_eq!(elems.len(), 0);
            }
            other => panic!("expected empty array literal, got {other:?}"),
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // Security regression tests (C-13, C-06, C-05)
    // ═══════════════════════════════════════════════════════════════

    /// C-13: Integer literal overflow must return a parse error, never silently become 0.
    #[test]
    fn test_c13_integer_overflow_returns_parse_error() {
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn set() {
        self.x = 99999999999999999999
    }
}
"#;
        let result = parse(src);
        assert!(
            result.is_err(),
            "parsing overflowing integer must fail, not silently become 0"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("overflow"),
            "error should mention overflow, got: {err}"
        );
    }

    /// C-13: Boundary value — u64::MAX should parse successfully.
    #[test]
    fn test_c13_u64_max_parses_ok() {
        let src = format!(
            r#"contract Test {{ state {{ x: u64 = 0 }} fn set() {{ self.x = {} }} }}"#,
            u64::MAX
        );
        let result = parse(&src);
        assert!(result.is_ok(), "u64::MAX should parse successfully");
    }

    /// C-13: u64::MAX + 1 must fail.
    #[test]
    fn test_c13_u64_max_plus_one_fails() {
        // 18446744073709551616 = u64::MAX + 1
        let src = r#"
contract Test {
    state { x: u64 = 0 }
    fn set() {
        self.x = 18446744073709551616
    }
}
"#;
        let result = parse(src);
        assert!(result.is_err(), "u64::MAX + 1 must fail to parse");
    }

    /// C-06: Deeply nested parenthesized expressions must be rejected, not stack-overflow.
    #[test]
    fn test_c06_deep_nesting_rejected() {
        // Build expression with 100 nested parens: (((((...0...)))))
        let open = "(".repeat(100);
        let close = ")".repeat(100);
        let src = format!(
            r#"contract Test {{ state {{ x: u64 = 0 }} fn f() -> u64 {{ return {open}0{close} }} }}"#,
        );
        let result = parse(&src);
        assert!(
            result.is_err(),
            "deeply nested expression (100 levels) must be rejected"
        );
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("depth") || err.contains("nesting"),
            "error should mention depth/nesting, got: {err}"
        );
    }

    /// C-06: Nesting at exactly MAX_EXPR_DEPTH (64) should be rejected.
    #[test]
    fn test_c06_nesting_at_max_depth_rejected() {
        let open = "(".repeat(65);
        let close = ")".repeat(65);
        let src = format!(
            r#"contract Test {{ state {{ x: u64 = 0 }} fn f() -> u64 {{ return {open}0{close} }} }}"#,
        );
        let result = parse(&src);
        assert!(result.is_err(), "nesting at depth 65 must be rejected");
    }

    /// C-06: Nesting below the limit should succeed.
    #[test]
    fn test_c06_moderate_nesting_ok() {
        // 10 levels of nesting should be fine
        let open = "(".repeat(10);
        let close = ")".repeat(10);
        let src = format!(
            r#"contract Test {{ state {{ x: u64 = 0 }} fn f() -> u64 {{ return {open}42{close} }} }}"#,
        );
        let result = parse(&src);
        assert!(
            result.is_ok(),
            "moderate nesting (10 levels) should succeed"
        );
    }
}
