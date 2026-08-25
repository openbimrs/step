//! Explicitly structural, partial EXPRESS declaration extraction.
//!
//! This module extracts schema names, entity headers, explicit positional
//! attributes, defined types, enumerations, and selects. It deliberately does
//! **not** implement full EXPRESS semantics: expressions, rules, functions,
//! procedures, constants, uniqueness constraints, inverse relationships,
//! derived attributes, and complete type checking remain opaque. Consumers
//! needing language validation must use a complete EXPRESS implementation.

/// A structurally parsed schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSchema {
    /// Declared schema name, or an empty string when absent.
    pub name: String,
    /// Entity declarations in source order.
    pub entities: Vec<EntityDef>,
    /// Type declarations in source order.
    pub types: Vec<TypeDef>,
}

/// One explicit positional attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Declared attribute name.
    pub name: String,
    /// Declared scalar or element type token.
    pub type_name: String,
    /// Whether `OPTIONAL` was present.
    pub optional: bool,
    /// Whether a `LIST`, `SET`, `ARRAY`, or `BAG` wrapper was present.
    pub aggregate: bool,
}

impl Attribute {
    /// Creates a required scalar attribute.
    #[must_use]
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            optional: false,
            aggregate: false,
        }
    }

    /// Marks the attribute as optional.
    #[must_use]
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Marks the attribute as an aggregate.
    #[must_use]
    pub const fn aggregate(mut self) -> Self {
        self.aggregate = true;
        self
    }
}

/// One structural entity declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDef {
    /// Declared entity name.
    pub name: String,
    /// First direct supertype, when a `SUBTYPE OF` clause is present.
    pub supertype: Option<String>,
    /// Whether the declaration includes `ABSTRACT`.
    pub abstract_: bool,
    /// Explicit attributes declared by this entity, excluding derived and
    /// inverse declarations.
    pub attributes: Vec<Attribute>,
}

impl EntityDef {
    /// Creates an empty concrete entity declaration.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supertype: None,
            abstract_: false,
            attributes: Vec::new(),
        }
    }

    /// Sets the direct supertype.
    #[must_use]
    pub fn with_supertype(mut self, supertype: impl Into<String>) -> Self {
        self.supertype = Some(supertype.into());
        self
    }

    /// Appends an explicit attribute in declaration order.
    #[must_use]
    pub fn with_attribute(mut self, attribute: Attribute) -> Self {
        self.attributes.push(attribute);
        self
    }
}

/// Structural shape of a `TYPE` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    /// Alias or other right-hand-side syntax retained as text.
    Defined(String),
    /// `ENUMERATION OF` member names.
    Enumeration(Vec<String>),
    /// `SELECT` member type names.
    Select(Vec<String>),
}

/// One `TYPE` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    /// Declared type name.
    pub name: String,
    /// Structurally recognized declaration kind.
    pub kind: TypeKind,
}

impl TypeDef {
    /// Returns whether this declaration aliases another type.
    #[must_use]
    pub const fn is_defined(&self) -> bool {
        matches!(self.kind, TypeKind::Defined(_))
    }
}

/// Extracts the supported structural subset from EXPRESS source.
///
/// Unsupported declarations and executable expressions are skipped. This
/// function is intentionally tolerant and returns the declarations it can
/// identify rather than claiming full language validation.
#[must_use]
pub fn parse(source: &str) -> ParsedSchema {
    let cleaned = strip_comments(source);
    let upper = ascii_uppercase(&cleaned);
    let name = schema_name(&cleaned, &upper).unwrap_or_default();
    let entities = blocks(&cleaned, &upper, "ENTITY", "END_ENTITY")
        .filter_map(parse_entity)
        .collect();
    let types = blocks(&cleaned, &upper, "TYPE", "END_TYPE")
        .filter_map(parse_type)
        .collect();
    ParsedSchema {
        name,
        entities,
        types,
    }
}

fn strip_comments(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut position = 0;
    let mut quoted = false;
    while position < bytes.len() {
        if bytes[position] == b'\'' {
            if quoted && bytes.get(position + 1) == Some(&b'\'') {
                position += 2;
                continue;
            }
            quoted = !quoted;
            position += 1;
            continue;
        }
        if !quoted && bytes[position..].starts_with(b"(*") {
            let start = position;
            position += 2;
            while position < bytes.len() && !bytes[position..].starts_with(b"*)") {
                position += 1;
            }
            position = (position + 2).min(bytes.len());
            blank_non_newlines(&mut output[start..position]);
            continue;
        }
        if !quoted && bytes[position..].starts_with(b"--") {
            let start = position;
            position += 2;
            while position < bytes.len() && bytes[position] != b'\n' {
                position += 1;
            }
            blank_non_newlines(&mut output[start..position]);
            continue;
        }
        position += 1;
    }
    String::from_utf8(output).expect("input was valid UTF-8")
}

fn blank_non_newlines(bytes: &mut [u8]) {
    for byte in bytes {
        if *byte != b'\n' && *byte != b'\r' {
            *byte = b' ';
        }
    }
}

fn ascii_uppercase(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    bytes.make_ascii_uppercase();
    String::from_utf8(bytes).expect("ASCII case conversion preserves UTF-8")
}

fn schema_name(source: &str, upper: &str) -> Option<String> {
    let start = find_keyword(upper, "SCHEMA", 0)? + "SCHEMA".len();
    let end = source[start..].find(';')? + start;
    source[start..end]
        .split_whitespace()
        .next()
        .map(ToOwned::to_owned)
}

fn blocks<'a>(
    source: &'a str,
    upper: &'a str,
    start_keyword: &'static str,
    end_keyword: &'static str,
) -> impl Iterator<Item = &'a str> {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        let start = find_keyword(upper, start_keyword, cursor)?;
        let end_start = find_keyword(upper, end_keyword, start + start_keyword.len())?;
        let semicolon = source[end_start..]
            .find(';')
            .map_or(source.len(), |offset| end_start + offset + 1);
        cursor = semicolon;
        Some(&source[start..semicolon])
    })
}

fn find_keyword(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let mut cursor = from;
    while let Some(relative) = haystack[cursor..].find(needle) {
        let position = cursor + relative;
        let before = position.checked_sub(1).and_then(|index| bytes.get(index));
        let after = bytes.get(position + needle.len());
        if before.is_none_or(|byte| !is_identifier_byte(*byte))
            && after.is_none_or(|byte| !is_identifier_byte(*byte))
        {
            return Some(position);
        }
        cursor = position + needle.len();
    }
    None
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn parse_entity(block: &str) -> Option<EntityDef> {
    let upper = ascii_uppercase(block);
    let header_end = block.find(';')?;
    let header = &block[..header_end];
    let header_upper = &upper[..header_end];
    let entity_position = find_keyword(header_upper, "ENTITY", 0)? + "ENTITY".len();
    let name = header[entity_position..]
        .split_whitespace()
        .next()?
        .trim_matches(|character: char| !character.is_alphanumeric() && character != '_')
        .to_owned();
    let supertype = clause_name(header, header_upper, "SUBTYPE OF");
    let abstract_ = find_keyword(header_upper, "ABSTRACT", 0).is_some();

    let body_end = ["DERIVE", "INVERSE", "UNIQUE", "WHERE", "END_ENTITY"]
        .into_iter()
        .filter_map(|keyword| find_keyword(&upper, keyword, header_end + 1))
        .min()
        .unwrap_or(block.len());
    let attributes = block[header_end + 1..body_end]
        .split(';')
        .filter_map(parse_attribute)
        .collect();

    Some(EntityDef {
        name,
        supertype,
        abstract_,
        attributes,
    })
}

fn clause_name(header: &str, upper: &str, clause: &str) -> Option<String> {
    let position = find_keyword(upper, clause, 0)? + clause.len();
    let open = header[position..].find('(')? + position + 1;
    let close = header[open..].find(')')? + open;
    header[open..close]
        .split(',')
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_attribute(statement: &str) -> Option<Attribute> {
    let (name, declaration) = statement.split_once(':')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let declaration = declaration.trim();
    let upper = ascii_uppercase(declaration);
    let optional = find_keyword(&upper, "OPTIONAL", 0).is_some();
    let aggregate = ["LIST", "SET", "ARRAY", "BAG"]
        .into_iter()
        .any(|keyword| find_keyword(&upper, keyword, 0).is_some());
    let scalar = if aggregate {
        find_keyword(&upper, "OF", 0)
            .map_or(declaration, |position| declaration[position + 2..].trim())
    } else if optional {
        find_keyword(&upper, "OPTIONAL", 0).map_or(declaration, |position| {
            declaration[position + "OPTIONAL".len()..].trim()
        })
    } else {
        declaration
    };
    let type_name = scalar
        .trim_start_matches(|character: char| character.is_ascii_whitespace())
        .strip_prefix("UNIQUE ")
        .unwrap_or(scalar)
        .split_whitespace()
        .next()?
        .trim_matches(|character: char| matches!(character, '(' | ')' | ';'))
        .to_owned();
    Some(Attribute {
        name: name.to_owned(),
        type_name,
        optional,
        aggregate,
    })
}

fn parse_type(block: &str) -> Option<TypeDef> {
    let upper = ascii_uppercase(block);
    let statement_end = block.find(';')?;
    let statement = &block[..statement_end];
    let statement_upper = &upper[..statement_end];
    let type_position = find_keyword(statement_upper, "TYPE", 0)? + "TYPE".len();
    let equals = statement[type_position..].find('=')? + type_position;
    let name = statement[type_position..equals].trim().to_owned();
    let right = statement[equals + 1..].trim();
    let right_upper = ascii_uppercase(right);
    let kind = if let Some(position) = find_keyword(&right_upper, "ENUMERATION", 0) {
        TypeKind::Enumeration(parenthesized_names(right, position + "ENUMERATION".len()))
    } else if let Some(position) = find_keyword(&right_upper, "SELECT", 0) {
        TypeKind::Select(parenthesized_names(right, position + "SELECT".len()))
    } else {
        TypeKind::Defined(right.to_owned())
    };
    Some(TypeDef { name, kind })
}

fn parenthesized_names(source: &str, from: usize) -> Vec<String> {
    let Some(open) = source[from..].find('(').map(|offset| from + offset + 1) else {
        return Vec::new();
    };
    let close = source[open..]
        .find(')')
        .map_or(source.len(), |offset| open + offset);
    source[open..close]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
