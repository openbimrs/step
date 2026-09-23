//! Explicitly structural, partial EXPRESS declaration extraction.
//!
//! This module extracts schema names, entity headers, explicit positional
//! attributes, the names of derived attributes, defined types, enumerations,
//! and selects. It deliberately does **not** implement full EXPRESS semantics:
//! expressions, rules, functions, procedures, constants, uniqueness
//! constraints, inverse relationships, and complete type checking remain
//! opaque. Derived attributes are reported by *name only* — their initialiser
//! expressions are not evaluated. Consumers needing language validation must
//! use a complete EXPRESS implementation.

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

/// One `WHERE` rule: a named constraint an instance must satisfy.
///
/// EXPRESS states these as `Label : expression;`. The expression is kept as
/// written rather than parsed: it is a full EXPRESS expression language
/// (TYPEOF, SIZEOF, QUERY, arithmetic), and evaluating it is a separate
/// concern from recording that the constraint exists and what it says.
///
/// Capturing them lets a consumer prove a claim like "no rule constrains
/// this attribute" instead of asserting it from prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhereRule {
    /// Rule label as declared, e.g. `CurveIs3D`.
    pub label: String,
    /// Constraint expression as written, whitespace-normalised.
    pub expression: String,
}

/// One explicit redeclaration of an inherited attribute: `SELF\X.a : T;`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redeclaration {
    /// The supertype named in the qualifier (`X`), as written.
    pub supertype: String,
    /// The inherited attribute (`a`), unqualified.
    pub name: String,
    /// The narrowed type token, extracted like [`Attribute::type_name`].
    pub type_name: String,
}

/// One structural entity declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityDef {
    /// Declared entity name.
    pub name: String,
    /// Direct supertypes in `SUBTYPE OF` order; empty for a root entity.
    ///
    /// EXPRESS allows several (`SUBTYPE OF (a, b)`), and the order is
    /// load-bearing: ISO 10303-21:2016 §12.2.5.2 lays inherited attributes
    /// out supertype by supertype in exactly this order. IFC never uses more
    /// than one; the AP schemas do (AP242: 248 entities).
    pub supertypes: Vec<String>,
    /// Whether the declaration includes `ABSTRACT`.
    pub abstract_: bool,
    /// Explicit attributes declared by this entity, excluding derived and
    /// inverse declarations.
    pub attributes: Vec<Attribute>,
    /// Names of attributes this entity declares in its `DERIVE` block.
    ///
    /// A subtype may redeclare an inherited explicit attribute as derived:
    ///
    /// ```text
    /// DERIVE
    ///   SELF\IfcGeometricRepresentationContext.Precision : IfcReal
    ///       := NVL(ParentContext.Precision, 1.E-5);
    /// ```
    ///
    /// The redeclaration keeps the attribute's inherited *position* but
    /// removes it from what an instance may state: Part 21 writes such a slot
    /// as `*`, not as a value and not as `$`. A consumer that does not know an
    /// attribute is derived cannot tell those apart, so this list is the
    /// minimum needed to write a conforming file.
    ///
    /// Names are stored unqualified — the `SELF\Entity.` prefix is stripped —
    /// because that is how they match the inherited attribute they redeclare.
    /// Entries are in declaration order. Derived attributes that are *new*
    /// rather than redeclarations appear here too; they occupy no positional
    /// slot, so consumers resolving slots should match against inherited
    /// attribute names rather than assuming every entry is positional.
    pub derived: Vec<String>,
    /// Explicit redeclarations of inherited attributes (`SELF\X.a : T;`).
    ///
    /// A redeclaration narrows the type of an inherited attribute; it is not
    /// a new attribute. ISO 10303-21:2016 §12.2.8: it has "no effect on the
    /// encoding" and "shall not be considered an attribute of the subtype for
    /// encoding purposes". It is therefore kept out of [`Self::attributes`],
    /// which would otherwise grow a positional slot no record contains.
    pub redeclared: Vec<Redeclaration>,
    /// `WHERE` rules declared by this entity, in declaration order.
    ///
    /// Only this entity's own rules: EXPRESS does not merge a subtype's
    /// rules with its supertype's, and a consumer checking an instance must
    /// walk the supertype chain itself.
    pub where_rules: Vec<WhereRule>,
}

impl EntityDef {
    /// Creates an empty concrete entity declaration.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            supertypes: Vec::new(),
            abstract_: false,
            attributes: Vec::new(),
            derived: Vec::new(),
            redeclared: Vec::new(),
            where_rules: Vec::new(),
        }
    }

    /// Appends a direct supertype, after any already declared.
    ///
    /// Call once per supertype in `SUBTYPE OF` order.
    #[must_use]
    pub fn with_supertype(mut self, supertype: impl Into<String>) -> Self {
        self.supertypes.push(supertype.into());
        self
    }

    /// The first direct supertype, if any.
    ///
    /// Sufficient for single-inheritance schemas such as IFC. Code that must
    /// be correct for any EXPRESS schema reads [`Self::supertypes`].
    #[must_use]
    pub fn supertype(&self) -> Option<&str> {
        self.supertypes.first().map(String::as_str)
    }

    /// Records an explicit redeclaration of an inherited attribute.
    #[must_use]
    pub fn with_redeclared(mut self, redeclaration: Redeclaration) -> Self {
        self.redeclared.push(redeclaration);
        self
    }

    /// Appends an explicit attribute in declaration order.
    #[must_use]
    pub fn with_attribute(mut self, attribute: Attribute) -> Self {
        self.attributes.push(attribute);
        self
    }

    /// Declares an attribute name as derived, as a `DERIVE` block would.
    #[must_use]
    pub fn with_derived(mut self, name: impl Into<String>) -> Self {
        self.derived.push(name.into());
        self
    }

    /// Whether `name` is declared derived by this entity.
    ///
    /// Comparison is ASCII case-insensitive: EXPRESS identifiers are
    /// case-sensitive in principle, but schema text and Part 21 keywords
    /// disagree on case often enough that matching exactly is a foot-gun.
    #[must_use]
    pub fn is_derived(&self, name: &str) -> bool {
        self.derived
            .iter()
            .any(|declared| declared.eq_ignore_ascii_case(name))
    }

    /// Whether this entity explicitly redeclares the inherited `name`.
    ///
    /// Case-insensitive, like [`Self::is_derived`].
    #[must_use]
    pub fn is_redeclared(&self, name: &str) -> bool {
        self.redeclared
            .iter()
            .any(|declared| declared.name.eq_ignore_ascii_case(name))
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

/// Find a block keyword (`DERIVE`, `INVERSE`, `UNIQUE`, `WHERE`) where it
/// actually opens a block, rather than where it merely appears as a word.
///
/// EXPRESS reuses these words inside declarations: `LIST [1:?] OF UNIQUE
/// IfcRepresentationMap` on `IfcTypeProduct` contains `UNIQUE` in the middle of
/// an attribute, and treating that as the start of a UNIQUE block truncates the
/// entity's attribute list. Both `IfcTypeProduct.RepresentationMaps` and `.Tag`
/// were lost that way, which silently made every product type unauthorable.
///
/// A block keyword only opens a block at statement level: the first token after
/// the previous statement's `;` (or after the entity header). Anything else is
/// part of a declaration.
fn find_block_keyword(source: &str, upper: &str, needle: &str, from: usize) -> Option<usize> {
    let mut cursor = from;
    while let Some(position) = find_keyword(upper, needle, cursor) {
        let preceding = source[from..position]
            .rfind(';')
            .map_or(&source[from..position], |offset| {
                &source[from + offset + 1..position]
            });
        if preceding.trim().is_empty() {
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
    let supertypes = clause_names(header, header_upper, "SUBTYPE OF");
    let abstract_ = find_keyword(header_upper, "ABSTRACT", 0).is_some();

    let body_end = ["DERIVE", "INVERSE", "UNIQUE", "WHERE", "END_ENTITY"]
        .into_iter()
        .filter_map(|keyword| find_block_keyword(block, &upper, keyword, header_end + 1))
        .min()
        .unwrap_or(block.len());
    let mut attributes = Vec::new();
    let mut redeclared = Vec::new();
    for statement in block[header_end + 1..body_end].split(';') {
        if let Some(redeclaration) = parse_redeclaration(statement) {
            redeclared.push(redeclaration);
        } else if let Some(attribute) = parse_attribute(statement) {
            attributes.push(attribute);
        }
    }
    let derived = parse_derive_block(block, &upper, header_end + 1);
    let where_rules = parse_where_block(block, &upper, header_end + 1);

    Some(EntityDef {
        name,
        supertypes,
        abstract_,
        attributes,
        derived,
        redeclared,
        where_rules,
    })
}

/// Collect the `WHERE` rules declared by one entity.
///
/// The block runs from a statement-level `WHERE` to `END_ENTITY`. Each rule is
/// `Label : expression;`. `find_block_keyword` is required rather than a plain
/// keyword search: `WHERE` also appears inside QUERY expressions
/// (`QUERY(t <* Types | WHERE ...)`), and treating one of those as the block
/// start would drop every rule declared before it.
///
/// Splitting rules on `;` is safe because EXPRESS expressions contain no
/// semicolons; a rule's expression may still span lines, so whitespace is
/// normalised to keep the stored text comparable.
/// Parse one `Label : expression` statement from a `WHERE` block.
///
/// The label ends at the first `:`. A rule expression may itself contain `:`
/// (`a <= b : c` does not occur, but qualified enum references like
/// `IfcEnum.VALUE` and ranges do), so only the first is a separator.
fn parse_where_rule(statement: &str) -> Option<WhereRule> {
    let (label, expression) = statement.split_once(':')?;
    let label = label.trim();
    if label.is_empty() || !label.bytes().all(is_identifier_byte) {
        return None;
    }
    let expression = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    if expression.is_empty() {
        return None;
    }
    Some(WhereRule {
        label: label.to_owned(),
        expression,
    })
}

fn parse_where_block(block: &str, upper: &str, from: usize) -> Vec<WhereRule> {
    let Some(start) = find_block_keyword(block, upper, "WHERE", from) else {
        return Vec::new();
    };
    let start = start + "WHERE".len();
    let end = find_keyword(upper, "END_ENTITY", start).unwrap_or(block.len());
    if end <= start {
        return Vec::new();
    }
    block[start..end]
        .split(';')
        .filter_map(parse_where_rule)
        .collect()
}

/// Collect the attribute names declared in an entity's `DERIVE` block.
///
/// The block runs from `DERIVE` to whichever of `INVERSE`/`UNIQUE`/`WHERE`/
/// `END_ENTITY` comes first. Each statement looks like
/// `SELF\Super.Name : Type := expression;` for a redeclaration, or
/// `Name : Type := expression;` for a new derived attribute.
///
/// Splitting on `;` is safe here because the initialiser expressions in a
/// DERIVE block are EXPRESS expressions, which do not contain semicolons.
fn parse_derive_block(block: &str, upper: &str, from: usize) -> Vec<String> {
    let Some(start) = find_keyword(upper, "DERIVE", from) else {
        return Vec::new();
    };
    let start = start + "DERIVE".len();
    let end = ["INVERSE", "UNIQUE", "WHERE", "END_ENTITY"]
        .into_iter()
        .filter_map(|keyword| find_keyword(upper, keyword, start))
        .min()
        .unwrap_or(block.len());
    if end <= start {
        return Vec::new();
    }

    block[start..end]
        .split(';')
        .filter_map(derived_attribute_name)
        .collect()
}

/// Extract the attribute name from one `DERIVE` statement.
///
/// `SELF\IfcGeometricRepresentationContext.Precision : IfcReal := ...` yields
/// `Precision`: the qualifying `SELF\Entity.` prefix names the supertype the
/// attribute is inherited from, not the attribute.
fn derived_attribute_name(statement: &str) -> Option<String> {
    let (target, _) = statement.split_once(':')?;
    let target = target.trim();
    // A redeclaration qualifies the name with the declaring supertype; the
    // attribute itself is the final dotted segment.
    let name = target.rsplit('.').next()?.trim();
    let name = name.rsplit('\\').next()?.trim();
    if name.is_empty() || !name.bytes().all(is_identifier_byte) {
        return None;
    }
    Some(name.to_owned())
}

/// Every name in a `CLAUSE (a, b, ...)` list, in written order.
fn clause_names(header: &str, upper: &str, clause: &str) -> Vec<String> {
    let Some(position) = find_keyword(upper, clause, 0).map(|p| p + clause.len()) else {
        return Vec::new();
    };
    let Some(open) = header[position..].find('(').map(|o| position + o + 1) else {
        return Vec::new();
    };
    let Some(close) = header[open..].find(')').map(|o| open + o) else {
        return Vec::new();
    };
    header[open..close]
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Parse `SELF\X.a : T` as an explicit redeclaration, or `None` for an
/// ordinary attribute statement.
fn parse_redeclaration(statement: &str) -> Option<Redeclaration> {
    let (target, _) = statement.split_once(':')?;
    let target = target.trim();
    let qualified = target
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("SELF\\"))
        .map(|_| &target[5..])?;
    let (supertype, name) = qualified.split_once('.')?;
    let (supertype, name) = (supertype.trim(), name.trim());
    if supertype.is_empty()
        || name.is_empty()
        || !supertype.bytes().all(is_identifier_byte)
        || !name.bytes().all(is_identifier_byte)
    {
        return None;
    }
    // Reuse the attribute type extraction on a synthetic plain statement.
    let type_name =
        parse_attribute(&format!("{name}{}", &statement[statement.find(':')?..]))?.type_name;
    Some(Redeclaration {
        supertype: supertype.to_owned(),
        name: name.to_owned(),
        type_name,
    })
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
