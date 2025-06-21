use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use smallvec::SmallVec;
use thiserror::Error;
use serde::{Serialize, Deserialize};

use super::{CSSError, CSSValue, ComputedValue, CSSStyleDeclaration, Color};
use crate::core::css::selector::Selector;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Unexpected token: {0}")]
    UnexpectedToken(String),
    #[error("Unexpected end of input")]
    UnexpectedEOF,
    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),
    #[error("Invalid value: {0}")]
    InvalidValue(String),
    #[error("Invalid selector: {0}")]
    InvalidSelector(String),
    #[error("Invalid media query: {0}")]
    InvalidMediaQuery(String),
}

pub type Result<T> = std::result::Result<T, ParseError>;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    String(String),
    Number(f32),
    Dimension(f32, String),
    Percentage(f32),
    Hash(String),
    Url(String),
    Function(String),
    AtKeyword(String),
    Delim(char),
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Colon,
    Semicolon,
    Comma,
    Whitespace,
    Comment(String),
    EOF,
}

impl Token {
    pub fn is_whitespace(&self) -> bool {
        matches!(self, Token::Whitespace | Token::Comment(_))
    }
}

pub struct Tokenizer {
    input: Vec<char>,
    position: usize,
    current_char: Option<char>,
}

impl Tokenizer {
    pub fn new(input: &str) -> Self {
        let chars: Vec<char> = input.chars().collect();
        let current_char = chars.get(0).copied();
        
        Self {
            input: chars,
            position: 0,
            current_char,
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        
        while let Some(token) = self.next_token() {
            if matches!(token, Token::EOF) {
                break;
            }
            tokens.push(token);
        }
        
        tokens
    }

    fn next_token(&mut self) -> Option<Token> {
        while let Some(ch) = self.current_char {
            match ch {
                ' ' | '\t' | '\n' | '\r' => {
                    self.consume_whitespace();
                    return Some(Token::Whitespace);
                }
                '/' if self.peek_char() == Some('*') => {
                    return Some(self.consume_comment());
                }
                '(' => {
                    self.advance();
                    return Some(Token::LeftParen);
                }
                ')' => {
                    self.advance();
                    return Some(Token::RightParen);
                }
                '[' => {
                    self.advance();
                    return Some(Token::LeftBracket);
                }
                ']' => {
                    self.advance();
                    return Some(Token::RightBracket);
                }
                '{' => {
                    self.advance();
                    return Some(Token::LeftBrace);
                }
                '}' => {
                    self.advance();
                    return Some(Token::RightBrace);
                }
                ':' => {
                    self.advance();
                    return Some(Token::Colon);
                }
                ';' => {
                    self.advance();
                    return Some(Token::Semicolon);
                }
                ',' => {
                    self.advance();
                    return Some(Token::Comma);
                }
                '#' => {
                    return Some(self.consume_hash());
                }
                '"' | '\'' => {
                    return Some(self.consume_string(ch));
                }
                '@' => {
                    return Some(self.consume_at_keyword());
                }
                '-' | '0'..='9' => {
                    return Some(self.consume_numeric());
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    return Some(self.consume_ident_like());
                }
                c => {
                    self.advance();
                    return Some(Token::Delim(c));
                }
            }
        }
        
        Some(Token::EOF)
    }

    fn advance(&mut self) {
        self.position += 1;
        self.current_char = self.input.get(self.position).copied();
    }

    fn peek_char(&self) -> Option<char> {
        self.input.get(self.position + 1).copied()
    }

    fn consume_whitespace(&mut self) {
        while let Some(ch) = self.current_char {
            if matches!(ch, ' ' | '\t' | '\n' | '\r') {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn consume_comment(&mut self) -> Token {
        let mut comment = String::new();
        self.advance(); // consume '/'
        self.advance(); // consume '*'
        
        while let Some(ch) = self.current_char {
            if ch == '*' && self.peek_char() == Some('/') {
                self.advance(); // consume '*'
                self.advance(); // consume '/'
                break;
            }
            comment.push(ch);
            self.advance();
        }
        
        Token::Comment(comment)
    }

    fn consume_string(&mut self, quote: char) -> Token {
        let mut string = String::new();
        self.advance(); // consume quote
        
        while let Some(ch) = self.current_char {
            if ch == quote {
                self.advance();
                break;
            }
            
            if ch == '\\' {
                self.advance();
                if let Some(escaped) = self.current_char {
                    string.push(escaped);
                    self.advance();
                }
            } else {
                string.push(ch);
                self.advance();
            }
        }
        
        Token::String(string)
    }

    fn consume_hash(&mut self) -> Token {
        let mut hash = String::new();
        self.advance(); // consume '#'
        
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                hash.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        Token::Hash(hash)
    }

    fn consume_at_keyword(&mut self) -> Token {
        let mut keyword = String::new();
        self.advance(); // consume '@'
        
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                keyword.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        Token::AtKeyword(keyword)
    }

    fn consume_numeric(&mut self) -> Token {
        let mut number_str = String::new();
        
        if self.current_char == Some('-') {
            number_str.push('-');
            self.advance();
        }
        
        while let Some(ch) = self.current_char {
            if ch.is_ascii_digit() {
                number_str.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        if self.current_char == Some('.') {
            number_str.push('.');
            self.advance();
            
            while let Some(ch) = self.current_char {
                if ch.is_ascii_digit() {
                    number_str.push(ch);
                    self.advance();
                } else {
                    break;
                }
            }
        }
        
        let number: f32 = number_str.parse().unwrap_or(0.0);
        
        if self.current_char == Some('%') {
            self.advance();
            return Token::Percentage(number);
        }
        
        let mut unit = String::new();
        while let Some(ch) = self.current_char {
            if ch.is_alphabetic() {
                unit.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        if unit.is_empty() {
            Token::Number(number)
        } else {
            Token::Dimension(number, unit)
        }
    }

    fn consume_ident_like(&mut self) -> Token {
        let mut ident = String::new();
        
        while let Some(ch) = self.current_char {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                ident.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        
        if self.current_char == Some('(') {
            if ident == "url" {
                return self.consume_url();
            } else {
                return Token::Function(ident);
            }
        }
        
        Token::Ident(ident)
    }

    fn consume_url(&mut self) -> Token {
        self.advance(); // consume '('
        
        while let Some(ch) = self.current_char {
            if !ch.is_whitespace() {
                break;
            }
            self.advance();
        }
        
        let mut url = String::new();
        let mut in_quotes = false;
        let mut quote_char = '"';
        
        if let Some(ch) = self.current_char {
            if ch == '"' || ch == '\'' {
                in_quotes = true;
                quote_char = ch;
                self.advance();
            }
        }
        
        while let Some(ch) = self.current_char {
            if in_quotes {
                if ch == quote_char {
                    self.advance();
                    break;
                } else {
                    url.push(ch);
                    self.advance();
                }
            } else {
                if ch == ')' {
                    break;
                } else if ch.is_whitespace() {
                    break;
                } else {
                    url.push(ch);
                    self.advance();
                }
            }
        }
        
        while let Some(ch) = self.current_char {
            if ch == ')' {
                self.advance();
                break;
            } else if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
        
        Token::Url(url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CSSRule {
    Style(CSSStyleRule),
    Media(CSSMediaRule),
    Import(CSSImportRule),
    FontFace(CSSFontFaceRule),
    Keyframes(CSSKeyframesRule),
    Page(CSSPageRule),
    Namespace(CSSNamespaceRule),
    Supports(CSSSupportsRule),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSStyleRule {
    pub selectors: Vec<Selector>,
    pub declarations: CSSStyleDeclaration,
    pub specificity: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSMediaRule {
    pub media_query: MediaQuery,
    pub rules: Vec<CSSRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSImportRule {
    pub href: String,
    pub media_query: Option<MediaQuery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSFontFaceRule {
    pub declarations: CSSStyleDeclaration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSKeyframesRule {
    pub name: String,
    pub keyframes: Vec<CSSKeyframeRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSKeyframeRule {
    pub offset: KeyframeOffset,
    pub declarations: CSSStyleDeclaration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyframeOffset {
    Percentage(f32),
    Keyword(String), // "from", "to"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSPageRule {
    pub selector: Option<String>,
    pub declarations: CSSStyleDeclaration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSNamespaceRule {
    pub prefix: Option<String>,
    pub namespace_uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSSSupportsRule {
    pub condition: String,
    pub rules: Vec<CSSRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaQuery {
    pub media_type: Option<String>,
    pub conditions: Vec<MediaCondition>,
    pub is_not: bool,
    pub is_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaCondition {
    pub feature: String,
    pub value: Option<String>,
    pub operator: Option<MediaOperator>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaOperator {
    Min,
    Max,
    Equal,
}

pub struct CSSParser {
    tokens: Vec<Token>,
    position: usize,
}

impl CSSParser {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            position: 0,
        }
    }

    pub fn parse(&mut self, input: &str) -> Result<Vec<CSSRule>> {
        let mut tokenizer = Tokenizer::new(input);
        self.tokens = tokenizer.tokenize();
        self.position = 0;

        let mut rules = Vec::new();
        
        while !self.is_at_end() {
            self.skip_whitespace();
            
            if self.is_at_end() {
                break;
            }

            match self.parse_rule() {
                Ok(rule) => rules.push(rule),
                Err(e) => {
                    tracing::warn!("CSS parse error: {}", e);
                    self.recover_from_error();
                }
            }
        }

        Ok(rules)
    }

    pub fn parse_declarations(&self, input: &str) -> Result<Vec<(String, String, bool)>> {
        let mut tokenizer = Tokenizer::new(input);
        let tokens = tokenizer.tokenize();
        let mut parser = Self {
            tokens,
            position: 0,
        };

        let mut declarations = Vec::new();
        
        while !parser.is_at_end() {
            parser.skip_whitespace();
            
            if parser.is_at_end() {
                break;
            }

            if let Ok((property, value, important)) = parser.parse_declaration() {
                declarations.push((property, value, important));
            }

            parser.consume_if_match(&Token::Semicolon);
        }

        Ok(declarations)
    }

    fn parse_rule(&mut self) -> Result<CSSRule> {
        self.skip_whitespace();
        
        if let Some(Token::AtKeyword(keyword)) = self.current_token() {
            match keyword.as_str() {
                "media" => self.parse_media_rule(),
                "import" => self.parse_import_rule(),
                "font-face" => self.parse_font_face_rule(),
                "keyframes" | "-webkit-keyframes" | "-moz-keyframes" => self.parse_keyframes_rule(),
                "page" => self.parse_page_rule(),
                "namespace" => self.parse_namespace_rule(),
                "supports" => self.parse_supports_rule(),
                _ => Err(ParseError::InvalidSyntax(format!("Unknown at-rule: @{}", keyword)))
            }
        } else {
            self.parse_style_rule()
        }
    }

    fn parse_style_rule(&mut self) -> Result<CSSRule> {
        let selectors = self.parse_selectors()?;
        
        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let declarations = self.parse_declarations_block()?;
        
        self.expect_token(&Token::RightBrace)?;

        let specificity = selectors.iter()
            .map(|s| s.specificity())
            .max()
            .unwrap_or(0);

        Ok(CSSRule::Style(CSSStyleRule {
            selectors,
            declarations,
            specificity,
        }))
    }

    fn parse_media_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @media
        self.skip_whitespace();
        
        let media_query = self.parse_media_query()?;
        
        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let mut rules = Vec::new();
        while !self.check_token(&Token::RightBrace) && !self.is_at_end() {
            rules.push(self.parse_rule()?);
        }
        
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSRule::Media(CSSMediaRule {
            media_query,
            rules,
        }))
    }

    fn parse_import_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @import
        self.skip_whitespace();
        
        let href = match self.current_token() {
            Some(Token::String(s)) => {
                let href = s.clone();
                self.advance();
                href
            }
            Some(Token::Url(u)) => {
                let href = u.clone();
                self.advance();
                href
            }
            _ => return Err(ParseError::InvalidSyntax("Expected URL or string in @import".to_string()))
        };

        self.skip_whitespace();
        
        let media_query = if !self.check_token(&Token::Semicolon) && !self.is_at_end() {
            Some(self.parse_media_query()?)
        } else {
            None
        };

        self.consume_if_match(&Token::Semicolon);

        Ok(CSSRule::Import(CSSImportRule {
            href,
            media_query,
        }))
    }

    fn parse_font_face_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @font-face
        self.skip_whitespace();
        
        self.expect_token(&Token::LeftBrace)?;
        let declarations = self.parse_declarations_block()?;
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSRule::FontFace(CSSFontFaceRule {
            declarations,
        }))
    }

    fn parse_keyframes_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @keyframes
        self.skip_whitespace();
        
        let name = match self.current_token() {
            Some(Token::Ident(name)) => {
                let name = name.clone();
                self.advance();
                name
            }
            _ => return Err(ParseError::InvalidSyntax("Expected identifier in @keyframes".to_string()))
        };

        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let mut keyframes = Vec::new();
        while !self.check_token(&Token::RightBrace) && !self.is_at_end() {
            keyframes.push(self.parse_keyframe_rule()?);
        }
        
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSRule::Keyframes(CSSKeyframesRule {
            name,
            keyframes,
        }))
    }

    fn parse_keyframe_rule(&mut self) -> Result<CSSKeyframeRule> {
        self.skip_whitespace();
        
        let offset = match self.current_token() {
            Some(Token::Percentage(p)) => {
                let percentage = *p;
                self.advance();
                KeyframeOffset::Percentage(percentage)
            }
            Some(Token::Ident(keyword)) => {
                let keyword = keyword.clone();
                self.advance();
                KeyframeOffset::Keyword(keyword)
            }
            _ => return Err(ParseError::InvalidSyntax("Expected percentage or keyword in keyframe".to_string()))
        };

        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let declarations = self.parse_declarations_block()?;
        
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSKeyframeRule {
            offset,
            declarations,
        })
    }

    fn parse_page_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @page
        self.skip_whitespace();
        
        let selector = if let Some(Token::Ident(s)) = self.current_token() {
            let selector = Some(s.clone());
            self.advance();
            selector
        } else {
            None
        };

        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let declarations = self.parse_declarations_block()?;
        
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSRule::Page(CSSPageRule {
            selector,
            declarations,
        }))
    }

    fn parse_namespace_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @namespace
        self.skip_whitespace();
        
        let (prefix, namespace_uri) = if let Some(Token::Ident(prefix)) = self.current_token() {
            let prefix = prefix.clone();
            self.advance();
            self.skip_whitespace();
            
            match self.current_token() {
                Some(Token::String(uri)) | Some(Token::Url(uri)) => {
                    let uri = uri.clone();
                    self.advance();
                    (Some(prefix), uri)
                }
                _ => return Err(ParseError::InvalidSyntax("Expected URI in @namespace".to_string()))
            }
        } else {
            match self.current_token() {
                Some(Token::String(uri)) | Some(Token::Url(uri)) => {
                    let uri = uri.clone();
                    self.advance();
                    (None, uri)
                }
                _ => return Err(ParseError::InvalidSyntax("Expected URI in @namespace".to_string()))
            }
        };

        self.consume_if_match(&Token::Semicolon);

        Ok(CSSRule::Namespace(CSSNamespaceRule {
            prefix,
            namespace_uri,
        }))
    }

    fn parse_supports_rule(&mut self) -> Result<CSSRule> {
        self.advance(); // consume @supports
        self.skip_whitespace();
        
        let mut condition = String::new();
        let mut paren_count = 0;
        
        while !self.check_token(&Token::LeftBrace) && !self.is_at_end() {
            match self.current_token() {
                Some(Token::LeftParen) => {
                    paren_count += 1;
                    condition.push('(');
                }
                Some(Token::RightParen) => {
                    paren_count -= 1;
                    condition.push(')');
                    if paren_count == 0 {
                        self.advance();
                        break;
                    }
                }
                Some(Token::Ident(s)) => condition.push_str(s),
                Some(Token::String(s)) => {
                    condition.push('"');
                    condition.push_str(s);
                    condition.push('"');
                }
                Some(Token::Colon) => condition.push(':'),
                Some(Token::Whitespace) => condition.push(' '),
                _ => {}
            }
            self.advance();
        }

        self.skip_whitespace();
        self.expect_token(&Token::LeftBrace)?;
        
        let mut rules = Vec::new();
        while !self.check_token(&Token::RightBrace) && !self.is_at_end() {
            rules.push(self.parse_rule()?);
        }
        
        self.expect_token(&Token::RightBrace)?;

        Ok(CSSRule::Supports(CSSSupportsRule {
            condition,
            rules,
        }))
    }

    fn parse_selectors(&mut self) -> Result<Vec<Selector>> {
        let mut selectors = Vec::new();
        
        loop {
            self.skip_whitespace();
            selectors.push(self.parse_selector()?);
            
            self.skip_whitespace();
            if self.consume_if_match(&Token::Comma) {
                continue;
            } else {
                break;
            }
        }
        
        Ok(selectors)
    }

    fn parse_selector(&mut self) -> Result<Selector> {
        let mut selector_text = String::new();
        
        while !self.check_token(&Token::LeftBrace) && 
              !self.check_token(&Token::Comma) && 
              !self.is_at_end() {
            match self.current_token() {
                Some(Token::Ident(s)) => selector_text.push_str(s),
                Some(Token::Hash(s)) => {
                    selector_text.push('#');
                    selector_text.push_str(s);
                }
                Some(Token::Delim('.')) => selector_text.push('.'),
                Some(Token::Delim('>')) => {
                    selector_text.push(' ');
                    selector_text.push('>');
                    selector_text.push(' ');
                }
                Some(Token::Delim('+')) => {
                    selector_text.push(' ');
                    selector_text.push('+');
                    selector_text.push(' ');
                }
                Some(Token::Delim('~')) => {
                    selector_text.push(' ');
                    selector_text.push('~');
                    selector_text.push(' ');
                }
                Some(Token::Delim('*')) => selector_text.push('*'),
                Some(Token::LeftBracket) => {
                    selector_text.push('[');
                    self.advance();
                    while !self.check_token(&Token::RightBracket) && !self.is_at_end() {
                        match self.current_token() {
                            Some(Token::Ident(s)) => selector_text.push_str(s),
                            Some(Token::String(s)) => {
                                selector_text.push('"');
                                selector_text.push_str(s);
                                selector_text.push('"');
                            }
                            Some(Token::Delim(c)) => selector_text.push(*c),
                            _ => {}
                        }
                        self.advance();
                    }
                    if self.check_token(&Token::RightBracket) {
                        selector_text.push(']');
                    }
                }
                Some(Token::Colon) => {
                    selector_text.push(':');
                    self.advance();
                    if let Some(Token::Ident(pseudo)) = self.current_token() {
                        selector_text.push_str(pseudo);
                    }
                }
                Some(Token::Whitespace) => {
                    if !selector_text.ends_with(' ') {
                        selector_text.push(' ');
                    }
                }
                _ => {}
            }
            
            self.advance();
        }
        
        Selector::parse(selector_text.trim())
            .map_err(|e| ParseError::InvalidSelector(e.to_string()))
    }

    fn parse_declarations_block(&mut self) -> Result<CSSStyleDeclaration> {
        let declarations = CSSStyleDeclaration::new();
        
        while !self.check_token(&Token::RightBrace) && !self.is_at_end() {
            self.skip_whitespace();
            
            if self.check_token(&Token::RightBrace) {
                break;
            }

            if let Ok((property, value, important)) = self.parse_declaration() {
                let _ = declarations.set_property(&property, &value, if important { "important" } else { "" });
            }

            self.consume_if_match(&Token::Semicolon);
        }

        Ok(declarations)
    }

    fn parse_declaration(&mut self) -> Result<(String, String, bool)> {
        self.skip_whitespace();
        
        let property = match self.current_token() {
            Some(Token::Ident(prop)) => {
                let property = prop.clone();
                self.advance();
                property
            }
            _ => return Err(ParseError::InvalidSyntax("Expected property name".to_string()))
        };

        self.skip_whitespace();
        self.expect_token(&Token::Colon)?;
        self.skip_whitespace();

        let mut value = String::new();
        let mut important = false;
        
        while !self.check_token(&Token::Semicolon) && 
              !self.check_token(&Token::RightBrace) && 
              !self.is_at_end() {
            match self.current_token() {
                Some(Token::Ident(s)) => {
                    if s == "important" && value.trim().ends_with('!') {
                        important = true;
                        value = value.trim_end_matches('!').trim().to_string();
                    } else {
                        if !value.is_empty() && !value.ends_with(' ') {
                            value.push(' ');
                        }
                        value.push_str(s);
                    }
                }
                Some(Token::String(s)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push('"');
                    value.push_str(s);
                    value.push('"');
                }
                Some(Token::Number(n)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push_str(&n.to_string());
                }
                Some(Token::Dimension(n, unit)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push_str(&format!("{}{}", n, unit));
                }
                Some(Token::Percentage(p)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push_str(&format!("{}%", p));
                }
                Some(Token::Hash(h)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push('#');
                    value.push_str(h);
                }
                Some(Token::Url(u)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push_str(&format!("url({})", u));
                }
                Some(Token::Function(f)) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                    value.push_str(f);
                    value.push('(');
                    self.advance();
                    
                    let mut paren_count = 1;
                    while paren_count > 0 && !self.is_at_end() {
                        match self.current_token() {
                            Some(Token::LeftParen) => {
                                paren_count += 1;
                                value.push('(');
                            }
                            Some(Token::RightParen) => {
                                paren_count -= 1;
                                value.push(')');
                            }
                            Some(Token::Ident(s)) => value.push_str(s),
                            Some(Token::Number(n)) => value.push_str(&n.to_string()),
                            Some(Token::Comma) => value.push(','),
                            Some(Token::Whitespace) => value.push(' '),
                            _ => {}
                        }
                        self.advance();
                    }
                    continue;
                }
                Some(Token::Delim(c)) => value.push(*c),
                Some(Token::Comma) => value.push(','),
                Some(Token::LeftParen) => value.push('('),
                Some(Token::RightParen) => value.push(')'),
                Some(Token::Whitespace) => {
                    if !value.is_empty() && !value.ends_with(' ') {
                        value.push(' ');
                    }
                }
                _ => {}
            }
            
            self.advance();
        }

        Ok((property, value.trim().to_string(), important))
    }

    fn parse_media_query(&mut self) -> Result<MediaQuery> {
        let mut is_not = false;
        let mut is_only = false;
        let mut media_type = None;
        let mut conditions = Vec::new();

        self.skip_whitespace();

        if let Some(Token::Ident(keyword)) = self.current_token() {
            match keyword.as_str() {
                "not" => {
                    is_not = true;
                    self.advance();
                    self.skip_whitespace();
                }
                "only" => {
                    is_only = true;
                    self.advance();
                    self.skip_whitespace();
                }
                _ => {}
            }
        }

        if let Some(Token::Ident(type_name)) = self.current_token() {
            if !type_name.starts_with('(') {
                media_type = Some(type_name.clone());
                self.advance();
                self.skip_whitespace();
            }
        }

        while self.check_token(&Token::LeftParen) && !self.is_at_end() {
            self.advance(); // consume '('
            
            let mut feature = String::new();
            let mut value = None;
            let mut operator = None;

            while !self.check_token(&Token::RightParen) && !self.is_at_end() {
                match self.current_token() {
                    Some(Token::Ident(s)) => {
                        if feature.is_empty() {
                            feature = s.clone();
                        } else if s == "min" && feature.is_empty() {
                            operator = Some(MediaOperator::Min);
                        } else if s == "max" && feature.is_empty() {
                            operator = Some(MediaOperator::Max);
                        }
                    }
                    Some(Token::Colon) => {
                        self.advance();
                        self.skip_whitespace();
                        
                        let mut val = String::new();
                        while !self.check_token(&Token::RightParen) && !self.is_at_end() {
                            match self.current_token() {
                                Some(Token::Number(n)) => val.push_str(&n.to_string()),
                                Some(Token::Dimension(n, unit)) => val.push_str(&format!("{}{}", n, unit)),
                                Some(Token::Ident(s)) => val.push_str(s),
                                _ => {}
                            }
                            self.advance();
                        }
                        value = Some(val.trim().to_string());
                        break;
                    }
                    _ => {}
                }
                self.advance();
            }

            if self.check_token(&Token::RightParen) {
                self.advance();
            }

            conditions.push(MediaCondition {
                feature,
                value,
                operator,
            });

            self.skip_whitespace();

            if let Some(Token::Ident(keyword)) = self.current_token() {
                if keyword == "and" {
                    self.advance();
                    self.skip_whitespace();
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(MediaQuery {
            media_type,
            conditions,
            is_not,
            is_only,
        })
    }

    fn current_token(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) {
        if self.position < self.tokens.len() {
            self.position += 1;
        }
    }

    fn is_at_end(&self) -> bool {
        self.position >= self.tokens.len()
    }

    fn check_token(&self, token: &Token) -> bool {
        if let Some(current) = self.current_token() {
            std::mem::discriminant(current) == std::mem::discriminant(token)
        } else {
            false
        }
    }

    fn consume_if_match(&mut self, token: &Token) -> bool {
        if self.check_token(token) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_token(&mut self, expected: &Token) -> Result<()> {
        if self.check_token(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::UnexpectedToken(format!("Expected {:?}, found {:?}", expected, self.current_token())))
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(token) = self.current_token() {
            if token.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn recover_from_error(&mut self) {
        let mut brace_count = 0;
        
        while !self.is_at_end() {
            match self.current_token() {
                Some(Token::LeftBrace) => brace_count += 1,
                Some(Token::RightBrace) => {
                    if brace_count > 0 {
                        brace_count -= 1;
                    } else {
                        self.advance();
                        return;
                    }
                }
                Some(Token::Semicolon) if brace_count == 0 => {
                    self.advance();
                    return;
                }
                _ => {}
            }
            self.advance();
        }
    }
}