use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

use crate::core::dom::{Document, NodeId};

#[derive(Error, Debug)]
pub enum SelectorError {
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Invalid selector: {0}")]
    InvalidSelector(String),
    #[error("Unsupported pseudo-class: {0}")]
    UnsupportedPseudoClass(String),
    #[error("Unsupported pseudo-element: {0}")]
    UnsupportedPseudoElement(String),
}

pub type Result<T> = std::result::Result<T, SelectorError>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Specificity {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

impl Default for Specificity {
    fn default() -> Self {
        Self::new()
    }
}

impl Specificity {
    pub fn new() -> Self {
        Self {
            a: 0,
            b: 0,
            c: 0,
            d: 0,
        }
    }
    pub fn with_inline() -> Self {
        Self {
            a: 1,
            b: 0,
            c: 0,
            d: 0,
        }
    }
    pub fn value(&self) -> u32 {
        self.a * 1000 + self.b * 100 + self.c * 10 + self.d
    }
    pub fn add(&mut self, other: &Specificity) {
        self.a += other.a;
        self.b += other.b;
        self.c += other.c;
        self.d += other.d;
    }
}

impl PartialOrd for Specificity {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Specificity {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.value().cmp(&other.value())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Combinator {
    None,
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AttributeOperator {
    Exists,
    Equal,
    Contains,
    DashMatch,
    StartsWith,
    EndsWith,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AttributeSelector {
    pub name: String,
    pub namespace: Option<String>,
    pub operator: AttributeOperator,
    pub value: Option<String>,
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PseudoClass {
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(NthPattern),
    NthLastChild(NthPattern),
    NthOfType(NthPattern),
    NthLastOfType(NthPattern),
    Not(Box<SimpleSelector>),
    Hover,
    Active,
    Focus,
    Visited,
    Link,
    Target,
    Enabled,
    Disabled,
    Checked,
    Indeterminate,
    Valid,
    Invalid,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    Lang(String),
    Dir(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PseudoElement {
    Before,
    After,
    FirstLine,
    FirstLetter,
    Backdrop,
    Placeholder,
    Selection,
    Marker,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NthPattern {
    pub a: i32,
    pub b: i32,
}

impl NthPattern {
    pub fn new(a: i32, b: i32) -> Self {
        Self { a, b }
    }

    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        match input {
            "odd" => Ok(Self::new(2, 1)),
            "even" => Ok(Self::new(2, 0)),
            _ => {
                if input == "n" {
                    Ok(Self::new(1, 0))
                } else if input.ends_with('n') {
                    let a_str = input.strip_suffix('n').unwrap().trim();
                    let a = match a_str {
                        "" | "+" => 1,
                        "-" => -1,
                        _ => a_str
                            .parse()
                            .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?,
                    };
                    Ok(Self::new(a, 0))
                } else if let Some(plus_pos) = input.find('+') {
                    let (a_part, b_part) = (&input[..plus_pos], &input[plus_pos + 1..]);
                    let a_str = a_part.trim().trim_end_matches('n').trim();
                    let b_str = b_part.trim();
                    let a = if a_str.is_empty() {
                        1
                    } else {
                        a_str
                            .parse()
                            .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?
                    };
                    let b = b_str
                        .parse::<i32>()
                        .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?;
                    Ok(Self::new(a, b))
                } else if let Some(minus_pos) = input.rfind('-') {
                    let (a_part, b_part) = (&input[..minus_pos], &input[minus_pos + 1..]);
                    let a_str = a_part.trim().trim_end_matches('n').trim();
                    let b_str = b_part.trim();
                    let a = if a_str.is_empty() {
                        1
                    } else {
                        a_str
                            .parse()
                            .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?
                    };
                    let b = b_str
                        .parse::<i32>()
                        .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?;
                    Ok(Self::new(a, -b))
                } else {
                    let b = input
                        .parse()
                        .map_err(|_| SelectorError::Parse("Invalid nth pattern".into()))?;
                    Ok(Self::new(0, b))
                }
            }
        }
    }

    pub fn matches(&self, position: i32) -> bool {
        if self.a == 0 {
            position == self.b
        } else {
            let n = (position - self.b) as f64 / self.a as f64;
            n >= 0.0 && n.fract() == 0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SimpleSelector {
    pub element_name: Option<String>,
    pub namespace: Option<String>,
    pub id: Option<String>,
    pub classes: SmallVec<[String; 4]>,
    pub attributes: SmallVec<[AttributeSelector; 2]>,
    pub pseudo_classes: SmallVec<[PseudoClass; 2]>,
    pub pseudo_element: Option<PseudoElement>,
}

impl Default for SimpleSelector {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleSelector {
    pub fn new() -> Self {
        Self {
            element_name: None,
            namespace: None,
            id: None,
            classes: SmallVec::new(),
            attributes: SmallVec::new(),
            pseudo_classes: SmallVec::new(),
            pseudo_element: None,
        }
    }

    pub fn with_element(element_name: String) -> Self {
        Self {
            element_name: Some(element_name),
            ..Self::new()
        }
    }
    pub fn with_id(id: String) -> Self {
        Self {
            id: Some(id),
            ..Self::new()
        }
    }
    pub fn with_class(class: String) -> Self {
        let mut classes = SmallVec::new();
        classes.push(class);
        Self {
            classes,
            ..Self::new()
        }
    }

    pub fn specificity(&self) -> Specificity {
        let mut spec = Specificity::new();
        if self.id.is_some() {
            spec.b += 1
        }
        spec.c += self.classes.len() as u32;
        spec.c += self.attributes.len() as u32;
        spec.c += self.pseudo_classes.len() as u32;
        if self.element_name.is_some() {
            spec.d += 1
        }
        if self.pseudo_element.is_some() {
            spec.d += 1
        }
        spec
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComplexSelector {
    pub simple_selector: SimpleSelector,
    pub combinator: Combinator,
    pub next: Option<Box<ComplexSelector>>,
}

impl ComplexSelector {
    pub fn new(simple_selector: SimpleSelector) -> Self {
        Self {
            simple_selector,
            combinator: Combinator::None,
            next: None,
        }
    }
    pub fn with_combinator(
        simple_selector: SimpleSelector,
        combinator: Combinator,
        next: ComplexSelector,
    ) -> Self {
        Self {
            simple_selector,
            combinator,
            next: Some(Box::new(next)),
        }
    }
    pub fn specificity(&self) -> Specificity {
        let mut spec = self.simple_selector.specificity();
        if let Some(ref next) = self.next {
            spec.add(&next.specificity())
        }
        spec
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selector {
    pub complex_selectors: SmallVec<[ComplexSelector; 2]>,
}

impl Default for Selector {
    fn default() -> Self {
        Self::new()
    }
}

impl Selector {
    pub fn new() -> Self {
        Self {
            complex_selectors: SmallVec::new(),
        }
    }
    pub fn parse(input: &str) -> Result<Self> {
        let mut parser = SelectorParser::new(input);
        parser.parse()
    }
    pub fn specificity(&self) -> u32 {
        self.complex_selectors
            .iter()
            .map(|cs| cs.specificity().value())
            .max()
            .unwrap_or(0)
    }
    pub fn matches(&self, node_id: NodeId, document: &Document, matcher: &SelectorMatcher) -> bool {
        self.complex_selectors
            .iter()
            .any(|cs| matcher.matches_complex_selector(cs, node_id, document))
    }
}

pub struct SelectorParser<'a> {
    input: &'a str,
    position: usize,
    current: Option<char>,
}

impl<'a> SelectorParser<'a> {
    fn new(input: &'a str) -> Self {
        let mut p = Self {
            input,
            position: 0,
            current: None,
        };
        p.advance();
        p
    }

    fn parse(&mut self) -> Result<Selector> {
        let mut complex_selectors = SmallVec::new();
        loop {
            self.skip_whitespace();
            if self.is_at_end() {
                break;
            }
            complex_selectors.push(self.parse_complex_selector()?);
            self.skip_whitespace();
            if !self.consume_char(',') {
                break;
            }
        }
        Ok(Selector { complex_selectors })
    }

    fn parse_complex_selector(&mut self) -> Result<ComplexSelector> {
        let simple = self.parse_simple_selector()?;
        self.skip_whitespace();
        if self.is_at_end() || self.current == Some(',') {
            return Ok(ComplexSelector::new(simple));
        }
        let comb = self.parse_combinator();
        if comb != Combinator::None {
            let next = self.parse_complex_selector()?;
            Ok(ComplexSelector::with_combinator(simple, comb, next))
        } else {
            Ok(ComplexSelector::new(simple))
        }
    }

    fn parse_simple_selector(&mut self) -> Result<SimpleSelector> {
        let mut sel = SimpleSelector::new();
        while !self.is_at_end() {
            match self.current {
                Some('*') => {
                    self.advance();
                    sel.element_name = Some("*".into())
                }
                Some('#') => {
                    self.advance();
                    sel.id = Some(self.parse_name()?)
                }
                Some('.') => {
                    self.advance();
                    sel.classes.push(self.parse_name()?)
                }
                Some('[') => sel.attributes.push(self.parse_attribute()?),
                Some(':') => {
                    self.advance();
                    if self.current == Some(':') {
                        self.advance();
                        sel.pseudo_element = Some(self.parse_pseudo_element()?)
                    } else {
                        sel.pseudo_classes.push(self.parse_pseudo_class()?)
                    }
                }
                Some(c) if c.is_alphabetic() || c == '_' => {
                    if sel.element_name.is_none() {
                        sel.element_name = Some(self.parse_name()?)
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        Ok(sel)
    }

    fn parse_combinator(&mut self) -> Combinator {
        self.skip_whitespace();
        let comb = match self.current {
            Some('>') => {
                self.advance();
                Combinator::Child
            }
            Some('+') => {
                self.advance();
                Combinator::NextSibling
            }
            Some('~') => {
                self.advance();
                Combinator::SubsequentSibling
            }
            _ => {
                if !self.is_at_end() && self.current != Some(',') {
                    Combinator::Descendant
                } else {
                    Combinator::None
                }
            }
        };
        self.skip_whitespace();
        comb
    }

    fn parse_attribute(&mut self) -> Result<AttributeSelector> {
        self.expect_char('[')?;
        let name = self.parse_name()?;
        self.skip_whitespace();
        let (op, val) = if self.current == Some(']') {
            (AttributeOperator::Exists, None)
        } else {
            let operator = match self.current {
                Some('=') => {
                    self.advance();
                    AttributeOperator::Equal
                }
                Some('~') => {
                    self.advance();
                    self.expect_char('=')?;
                    AttributeOperator::Contains
                }
                Some('|') => {
                    self.advance();
                    self.expect_char('=')?;
                    AttributeOperator::DashMatch
                }
                Some('^') => {
                    self.advance();
                    self.expect_char('=')?;
                    AttributeOperator::StartsWith
                }
                Some('$') => {
                    self.advance();
                    self.expect_char('=')?;
                    AttributeOperator::EndsWith
                }
                Some('*') => {
                    self.advance();
                    self.expect_char('=')?;
                    AttributeOperator::Substring
                }
                _ => return Err(SelectorError::Parse("Expected attribute operator".into())),
            };
            self.skip_whitespace();
            let value = if self.current == Some('"') || self.current == Some('\'') {
                Some(self.parse_string()?)
            } else {
                Some(self.parse_name()?)
            };
            (operator, value)
        };
        self.skip_whitespace();
        let case_insensitive = if matches!(self.current, Some('i') | Some('I')) {
            self.advance();
            self.skip_whitespace();
            true
        } else {
            false
        };
        self.expect_char(']')?;
        Ok(AttributeSelector {
            name,
            namespace: None,
            operator: op,
            value: val,
            case_insensitive,
        })
    }

    fn parse_pseudo_class(&mut self) -> Result<PseudoClass> {
        let name = self.parse_name()?;
        match name.as_str() {
            "root" => Ok(PseudoClass::Root),
            "empty" => Ok(PseudoClass::Empty),
            "first-child" => Ok(PseudoClass::FirstChild),
            "last-child" => Ok(PseudoClass::LastChild),
            "only-child" => Ok(PseudoClass::OnlyChild),
            "first-of-type" => Ok(PseudoClass::FirstOfType),
            "last-of-type" => Ok(PseudoClass::LastOfType),
            "only-of-type" => Ok(PseudoClass::OnlyOfType),
            "hover" => Ok(PseudoClass::Hover),
            "active" => Ok(PseudoClass::Active),
            "focus" => Ok(PseudoClass::Focus),
            "visited" => Ok(PseudoClass::Visited),
            "link" => Ok(PseudoClass::Link),
            "target" => Ok(PseudoClass::Target),
            "enabled" => Ok(PseudoClass::Enabled),
            "disabled" => Ok(PseudoClass::Disabled),
            "checked" => Ok(PseudoClass::Checked),
            "indeterminate" => Ok(PseudoClass::Indeterminate),
            "valid" => Ok(PseudoClass::Valid),
            "invalid" => Ok(PseudoClass::Invalid),
            "required" => Ok(PseudoClass::Required),
            "optional" => Ok(PseudoClass::Optional),
            "read-only" => Ok(PseudoClass::ReadOnly),
            "read-write" => Ok(PseudoClass::ReadWrite),
            "nth-child" => {
                self.expect_char('(')?;
                let pat = self.parse_nth_pattern()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthChild(pat))
            }
            "nth-last-child" => {
                self.expect_char('(')?;
                let pat = self.parse_nth_pattern()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthLastChild(pat))
            }
            "nth-of-type" => {
                self.expect_char('(')?;
                let pat = self.parse_nth_pattern()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthOfType(pat))
            }
            "nth-last-of-type" => {
                self.expect_char('(')?;
                let pat = self.parse_nth_pattern()?;
                self.expect_char(')')?;
                Ok(PseudoClass::NthLastOfType(pat))
            }
            "not" => {
                self.expect_char('(')?;
                let sel = self.parse_simple_selector()?;
                self.expect_char(')')?;
                Ok(PseudoClass::Not(Box::new(sel)))
            }
            "lang" => {
                self.expect_char('(')?;
                let lang = self.parse_string().or_else(|_| self.parse_name())?;
                self.expect_char(')')?;
                Ok(PseudoClass::Lang(lang))
            }
            "dir" => {
                self.expect_char('(')?;
                let dir = self.parse_string().or_else(|_| self.parse_name())?;
                self.expect_char(')')?;
                Ok(PseudoClass::Dir(dir))
            }
            _ => Err(SelectorError::UnsupportedPseudoClass(name)),
        }
    }

    fn parse_pseudo_element(&mut self) -> Result<PseudoElement> {
        let name = self.parse_name()?;
        match name.as_str() {
            "before" => Ok(PseudoElement::Before),
            "after" => Ok(PseudoElement::After),
            "first-line" => Ok(PseudoElement::FirstLine),
            "first-letter" => Ok(PseudoElement::FirstLetter),
            "backdrop" => Ok(PseudoElement::Backdrop),
            "placeholder" => Ok(PseudoElement::Placeholder),
            "selection" => Ok(PseudoElement::Selection),
            "marker" => Ok(PseudoElement::Marker),
            _ => Err(SelectorError::UnsupportedPseudoElement(name)),
        }
    }

    fn parse_nth_pattern(&mut self) -> Result<NthPattern> {
        self.skip_whitespace();
        let start = self.position;
        while let Some(c) = self.current {
            if c == ')' || c.is_whitespace() {
                break;
            }
            self.advance();
        }
        let pat = &self.input[start..self.position];
        NthPattern::parse(pat)
    }

    fn parse_name(&mut self) -> Result<String> {
        let start = self.position;
        while let Some(c) = self.current {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                self.advance()
            } else {
                break;
            }
        }
        if start == self.position {
            Err(SelectorError::Parse("Expected name".into()))
        } else {
            Ok(self.input[start..self.position].to_string())
        }
    }

    fn parse_string(&mut self) -> Result<String> {
        let quote = self
            .current
            .ok_or_else(|| SelectorError::Parse("Expected string".into()))?;
        if quote != '"' && quote != '\'' {
            return Err(SelectorError::Parse("Expected quoted string".into()));
        }
        self.advance();
        let mut s = String::new();
        while let Some(c) = self.current {
            if c == quote {
                self.advance();
                return Ok(s);
            }
            if c == '\\' {
                self.advance();
                if let Some(e) = self.current {
                    s.push(e);
                    self.advance()
                }
            } else {
                s.push(c);
                self.advance()
            }
        }
        Err(SelectorError::Parse("Unterminated string".into()))
    }

    fn advance(&mut self) {
        if let Some(c) = self.current {
            self.position += c.len_utf8();
        }
        self.current = self.input[self.position..].chars().next();
    }

    fn is_at_end(&self) -> bool {
        self.current.is_none()
    }
    fn skip_whitespace(&mut self) {
        while let Some(c) = self.current {
            if c.is_whitespace() {
                self.advance()
            } else {
                break;
            }
        }
    }

    fn consume_char(&mut self, expected: char) -> bool {
        if self.current == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<()> {
        if self.consume_char(expected) {
            Ok(())
        } else {
            Err(SelectorError::Parse(format!("Expected '{}'", expected)))
        }
    }
}

#[derive(Debug)]
struct NodeCache {
    element_name: Option<String>,
    id: Option<String>,
    classes: HashSet<String>,
    attributes: HashMap<String, String>,
    children: Vec<NodeId>,
    parent: Option<NodeId>,
}

impl NodeCache {
    fn new() -> Self {
        Self {
            element_name: None,
            id: None,
            classes: HashSet::new(),
            attributes: HashMap::new(),
            children: Vec::new(),
            parent: None,
        }
    }
}

pub struct SelectorMatcher {
    node_cache: DashMap<NodeId, NodeCache>,
    match_cache: DashMap<(String, NodeId), bool>,
}

impl Default for SelectorMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectorMatcher {
    pub fn new() -> Self {
        Self {
            node_cache: DashMap::new(),
            match_cache: DashMap::new(),
        }
    }

    pub fn matches(&self, selector: &Selector, node_id: NodeId, document: &Document) -> bool {
        let key = (format!("{:?}", selector), node_id);
        if let Some(r) = self.match_cache.get(&key) {
            return *r;
        }
        let res = selector.matches(node_id, document, self);
        self.match_cache.insert(key, res);
        res
    }

    pub fn matches_complex_selector(
        &self,
        selector: &ComplexSelector,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        if !self.matches_simple_selector(&selector.simple_selector, node_id, document) {
            return false;
        }
        if let Some(next) = &selector.next {
            match selector.combinator {
                Combinator::None => true,
                Combinator::Descendant => self.matches_descendant(next, node_id, document),
                Combinator::Child => self.matches_child(next, node_id, document),
                Combinator::NextSibling => self.matches_next_sibling(next, node_id, document),
                Combinator::SubsequentSibling => {
                    self.matches_subsequent_sibling(next, node_id, document)
                }
            }
        } else {
            true
        }
    }

    fn matches_simple_selector(
        &self,
        selector: &SimpleSelector,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        let cache = self.get_or_create_node_cache(node_id, document);
        if let Some(name) = &selector.element_name {
            if name != "*" {
                if let Some(en) = &cache.element_name {
                    if en.to_lowercase() != name.to_lowercase() {
                        return false;
                    }
                } else {
                    return false;
                }
            }
        }
        if let Some(id) = &selector.id {
            if cache.id.as_ref() != Some(id) {
                return false;
            }
        }
        for class in &selector.classes {
            if !cache.classes.contains(class) {
                return false;
            }
        }
        for attr in &selector.attributes {
            if !self.matches_attribute(attr, &cache.attributes) {
                return false;
            }
        }
        for pc in &selector.pseudo_classes {
            if !self.matches_pseudo_class(pc, node_id, document) {
                return false;
            }
        }
        true
    }

    fn get_or_create_node_cache(
        &self,
        node_id: NodeId,
        document: &Document,
    ) -> dashmap::mapref::one::Ref<NodeId, NodeCache> {
        if !self.node_cache.contains_key(&node_id) {
            if let Some(node) = document.get_node(node_id) {
                let nd = node.read();
                let mut cache = NodeCache::new();
                cache.element_name = Some(nd.get_tag_name().to_string());
                cache.id = nd.get_attribute("id");
                if let Some(class_attr) = nd.get_attribute("class") {
                    cache.classes = class_attr
                        .split_whitespace()
                        .map(|s| s.to_string())
                        .collect();
                }
                cache.children = document.get_children(node_id);
                cache.parent = document.get_parent(node_id);
                self.node_cache.insert(node_id, cache);
            } else {
                self.node_cache.insert(node_id, NodeCache::new());
            }
        }
        self.node_cache.get(&node_id).unwrap()
    }

    fn matches_attribute(
        &self,
        attribute: &AttributeSelector,
        attributes: &HashMap<String, String>,
    ) -> bool {
        match &attribute.operator {
            AttributeOperator::Exists => attributes.contains_key(&attribute.name),
            AttributeOperator::Equal => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        if attribute.case_insensitive {
                            actual.to_lowercase() == expected.to_lowercase()
                        } else {
                            actual == expected
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            AttributeOperator::Contains => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        let vals: HashSet<&str> = actual.split_whitespace().collect();
                        if attribute.case_insensitive {
                            vals.iter()
                                .any(|v| v.to_lowercase() == expected.to_lowercase())
                        } else {
                            vals.contains(expected.as_str())
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            AttributeOperator::DashMatch => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        if attribute.case_insensitive {
                            let al = actual.to_lowercase();
                            let el = expected.to_lowercase();
                            al == el || al.starts_with(&format!("{}-", el))
                        } else {
                            actual == expected || actual.starts_with(&format!("{}-", expected))
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            AttributeOperator::StartsWith => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        if attribute.case_insensitive {
                            actual.to_lowercase().starts_with(&expected.to_lowercase())
                        } else {
                            actual.starts_with(expected)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            AttributeOperator::EndsWith => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        if attribute.case_insensitive {
                            actual.to_lowercase().ends_with(&expected.to_lowercase())
                        } else {
                            actual.ends_with(expected)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            AttributeOperator::Substring => {
                if let Some(expected) = &attribute.value {
                    if let Some(actual) = attributes.get(&attribute.name) {
                        if attribute.case_insensitive {
                            actual.to_lowercase().contains(&expected.to_lowercase())
                        } else {
                            actual.contains(expected)
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
        }
    }

    fn matches_pseudo_class(
        &self,
        pseudo_class: &PseudoClass,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        match pseudo_class {
            PseudoClass::Root => document.get_parent(node_id).is_none(),
            PseudoClass::Empty => {
                let children = document.get_children(node_id);
                children.is_empty()
                    || children.iter().all(|&cid| {
                        if let Some(child) = document.get_node(cid) {
                            child.read().get_text_content().trim().is_empty()
                        } else {
                            true
                        }
                    })
            }
            PseudoClass::FirstChild => document
                .get_parent(node_id)
                .is_some_and(|pid| document.get_children(pid).first() == Some(&node_id)),
            PseudoClass::LastChild => document
                .get_parent(node_id)
                .is_some_and(|pid| document.get_children(pid).last() == Some(&node_id)),
            PseudoClass::OnlyChild => document.get_parent(node_id).is_some_and(|pid| {
                let sib = document.get_children(pid);
                sib.len() == 1 && sib[0] == node_id
            }),
            PseudoClass::NthChild(pattern) => document.get_parent(node_id).is_some_and(|pid| {
                document
                    .get_children(pid)
                    .iter()
                    .position(|&id| id == node_id)
                    .map(|i| pattern.matches((i + 1) as i32))
                    .unwrap_or(false)
            }),
            PseudoClass::NthLastChild(pattern) => document.get_parent(node_id).is_some_and(|pid| {
                let sib = document.get_children(pid);
                sib.iter()
                    .position(|&id| id == node_id)
                    .map(|i| {
                        let pos = sib.len() - i;
                        pattern.matches(pos as i32)
                    })
                    .unwrap_or(false)
            }),
            PseudoClass::Not(inner) => !self.matches_simple_selector(inner, node_id, document),
            _ => false,
        }
    }

    fn matches_descendant(
        &self,
        selector: &ComplexSelector,
        mut node_id: NodeId,
        document: &Document,
    ) -> bool {
        while let Some(pid) = document.get_parent(node_id) {
            if self.matches_complex_selector(selector, pid, document) {
                return true;
            }
            node_id = pid;
        }
        false
    }

    fn matches_child(
        &self,
        selector: &ComplexSelector,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        document
            .get_parent(node_id)
            .is_some_and(|pid| self.matches_complex_selector(selector, pid, document))
    }

    fn matches_next_sibling(
        &self,
        selector: &ComplexSelector,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        document.get_parent(node_id).is_some_and(|pid| {
            let sib = document.get_children(pid);
            sib.iter().position(|&id| id == node_id).is_some_and(|i| {
                i > 0 && self.matches_complex_selector(selector, sib[i - 1], document)
            })
        })
    }

    fn matches_subsequent_sibling(
        &self,
        selector: &ComplexSelector,
        node_id: NodeId,
        document: &Document,
    ) -> bool {
        document.get_parent(node_id).is_some_and(|pid| {
            let sib = document.get_children(pid);
            sib.iter().position(|&id| id == node_id).is_some_and(|i| {
                sib.iter()
                    .take(i)
                    .any(|&sid| self.matches_complex_selector(selector, sid, document))
            })
        })
    }

    pub fn invalidate_cache(&self) {
        self.node_cache.clear();
        self.match_cache.clear();
    }

    pub fn invalidate_node_cache(&self, node_id: NodeId) {
        self.node_cache.remove(&node_id);
        let keys: Vec<_> = self
            .match_cache
            .iter()
            .filter(|e| e.key().1 == node_id)
            .map(|e| e.key().clone())
            .collect();
        for key in keys {
            self.match_cache.remove(&key);
        }
    }
}

pub struct SelectorEngine {
    matcher: Arc<SelectorMatcher>,
    cached_selectors: DashMap<String, Selector>,
}

impl Default for SelectorEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SelectorEngine {
    pub fn new() -> Self {
        Self {
            matcher: Arc::new(SelectorMatcher::new()),
            cached_selectors: DashMap::new(),
        }
    }

    fn collect_all_nodes(document: &Document) -> Vec<NodeId> {
        let mut nodes = Vec::new();
        if let Some(root) = document.get_root_node() {
            let mut stack = vec![root];
            while let Some(n) = stack.pop() {
                nodes.push(n);
                for &c in &document.get_children(n) {
                    stack.push(c);
                }
            }
        }
        nodes
    }

    pub fn parse_selector(&self, input: &str) -> Result<Selector> {
        if let Some(sel) = self.cached_selectors.get(input) {
            return Ok(sel.clone());
        }
        let sel = Selector::parse(input)?;
        self.cached_selectors.insert(input.to_string(), sel.clone());
        Ok(sel)
    }

    pub fn matches(
        &self,
        selector_text: &str,
        node_id: NodeId,
        document: &Document,
    ) -> Result<bool> {
        let sel = self.parse_selector(selector_text)?;
        Ok(self.matcher.matches(&sel, node_id, document))
    }

    pub fn query_selector(
        &self,
        selector_text: &str,
        document: &Document,
    ) -> Result<Option<NodeId>> {
        let sel = self.parse_selector(selector_text)?;
        for node_id in Self::collect_all_nodes(document) {
            if self.matcher.matches(&sel, node_id, document) {
                return Ok(Some(node_id));
            }
        }
        Ok(None)
    }

    pub fn query_selector_all(
        &self,
        selector_text: &str,
        document: &Document,
    ) -> Result<Vec<NodeId>> {
        let sel = self.parse_selector(selector_text)?;
        let mut res = Vec::new();
        for node_id in Self::collect_all_nodes(document) {
            if self.matcher.matches(&sel, node_id, document) {
                res.push(node_id);
            }
        }
        Ok(res)
    }

    pub fn invalidate_cache(&self) {
        self.matcher.invalidate_cache();
        self.cached_selectors.clear();
    }

    pub fn invalidate_node_cache(&self, node_id: NodeId) {
        self.matcher.invalidate_node_cache(node_id);
    }

    pub fn get_cache_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "selector_cache_size": self.cached_selectors.len(),
            "node_cache_size": self.matcher.node_cache.len(),
            "match_cache_size": self.matcher.match_cache.len(),
        })
    }
}
