//! Explicitly structural, partial EXPRESS declaration extraction.
//!
//! This module extracts schema names, entity headers, explicit positional
//! attributes with their aggregate shape, the names of derived attributes,
//! inverse attributes, uniqueness rules, `WHERE` rules as text, defined types,
//! enumerations, and selects. It deliberately does **not** implement full
//! EXPRESS semantics: expressions, rules, functions, procedures, constants,
//! and complete type checking remain opaque. Derived attributes are reported
//! by *name only* — their initialiser expressions are not evaluated, and
//! neither are bound expressions or `WHERE` rules. Consumers needing language
//! validation must use a complete EXPRESS implementation.
//!
//! Every declaration type is `#[non_exhaustive]`: extracting more of the
//! language later adds fields without breaking callers. Build values with the
//! constructors and `with_*` methods rather than struct literals.

/// A structurally parsed schema.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParsedSchema {
    /// Declared schema name, or an empty string when absent.
    pub name: String,
    /// Entity declarations in source order.
    pub entities: Vec<EntityDef>,
    /// Type declarations in source order.
    pub types: Vec<TypeDef>,
}

impl ParsedSchema {
    /// Creates a schema from its declarations.
    #[must_use]
    pub fn new(name: impl Into<String>, entities: Vec<EntityDef>, types: Vec<TypeDef>) -> Self {
        Self {
            name: name.into(),
            entities,
            types,
        }
    }
}

/// The four EXPRESS aggregation types (ISO 10303-11 §8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AggregateKind {
    /// `LIST`: ordered, duplicates allowed unless `UNIQUE`.
    List,
    /// `SET`: unordered, no duplicates.
    Set,
    /// `BAG`: unordered, duplicates allowed.
    Bag,
    /// `ARRAY`: fixed-size, indexed by its bounds.
    Array,
}

impl AggregateKind {
    fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "LIST" => Some(Self::List),
            "SET" => Some(Self::Set),
            "BAG" => Some(Self::Bag),
            "ARRAY" => Some(Self::Array),
            _ => None,
        }
    }
}

/// One bound of an aggregation, as declared.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Bound {
    /// An integer literal.
    Integer(u64),
    /// `?`: no upper limit.
    Unbounded,
    /// Any other bound expression, as written with whitespace normalised,
    /// e.g. `SELF\IfcBSplineCurve.UpperIndexOnControlPoints`. It is not
    /// evaluated.
    Expression(String),
}

impl Bound {
    fn parse(text: &str) -> Self {
        let text = text.trim();
        if text == "?" {
            Self::Unbounded
        } else if let Ok(value) = text.parse() {
            Self::Integer(value)
        } else {
            Self::Expression(text.split_whitespace().collect::<Vec<_>>().join(" "))
        }
    }
}

/// One aggregation level of a declared type: `LIST [1:3] OF ...`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Aggregation {
    /// Which aggregation type.
    pub kind: AggregateKind,
    /// Lower bound. `0` when `LIST`, `SET` or `BAG` omit their bounds, which
    /// ISO 10303-11 defines as `[0:?]`.
    pub lower: Bound,
    /// Upper bound. [`Bound::Unbounded`] when `LIST`, `SET` or `BAG` omit
    /// their bounds.
    pub upper: Bound,
    /// Whether the elements are declared `UNIQUE` (`OF UNIQUE ...`).
    pub unique: bool,
    /// Whether an `ARRAY`'s elements are declared `OPTIONAL`
    /// (`ARRAY [1:3] OF OPTIONAL ...`).
    pub optional_elements: bool,
}

impl Aggregation {
    /// Creates an aggregation level with explicit bounds.
    #[must_use]
    pub const fn new(kind: AggregateKind, lower: Bound, upper: Bound) -> Self {
        Self {
            kind,
            lower,
            upper,
            unique: false,
            optional_elements: false,
        }
    }

    /// Marks the elements `UNIQUE`.
    #[must_use]
    pub const fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    /// Marks an `ARRAY`'s elements `OPTIONAL`.
    #[must_use]
    pub const fn optional_elements(mut self) -> Self {
        self.optional_elements = true;
        self
    }
}

/// One explicit positional attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attribute {
    /// Declared attribute name.
    pub name: String,
    /// Declared scalar type, or the innermost element type of an aggregate.
    pub type_name: String,
    /// Whether `OPTIONAL` was present.
    pub optional: bool,
    /// Whether a `LIST`, `SET`, `ARRAY`, or `BAG` wrapper was present. Equal
    /// to `!aggregation.is_empty()`.
    pub aggregate: bool,
    /// Aggregation levels, outermost first: `LIST [2:3] OF UNIQUE LIST [1:?]
    /// OF IfcLengthMeasure` has two, the first `unique`. Empty for a scalar.
    pub aggregation: Vec<Aggregation>,
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
            aggregation: Vec::new(),
        }
    }

    /// Marks the attribute as optional.
    #[must_use]
    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Marks the attribute as an aggregate without recording its shape.
    ///
    /// Prefer [`Self::with_aggregation`], which records the kind and bounds.
    #[must_use]
    pub const fn aggregate(mut self) -> Self {
        self.aggregate = true;
        self
    }

    /// Wraps the type in one more aggregation level, inside any already
    /// added: call it outermost first.
    #[must_use]
    pub fn with_aggregation(mut self, aggregation: Aggregation) -> Self {
        self.aggregation.push(aggregation);
        self.aggregate = true;
        self
    }
}

/// One `INVERSE` attribute: `Name : SET [0:1] OF Entity FOR Attribute;`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InverseAttribute {
    /// Declared name, unqualified.
    pub name: String,
    /// The supertype named when a subtype redeclares an inherited inverse
    /// (`SELF\X.Name : ...`); `None` for a new inverse attribute.
    pub redeclares: Option<String>,
    /// The entity whose attribute points back at this one.
    pub entity: String,
    /// The attribute named after `FOR`, as written (it may be qualified as
    /// `Entity.Attribute`).
    pub for_attribute: String,
    /// `SET` or `BAG` with its bounds; `None` for a single-valued inverse.
    pub aggregation: Option<Aggregation>,
}

impl InverseAttribute {
    /// Creates a single-valued inverse attribute.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        entity: impl Into<String>,
        for_attribute: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            redeclares: None,
            entity: entity.into(),
            for_attribute: for_attribute.into(),
            aggregation: None,
        }
    }

    /// Makes the inverse an aggregate (`SET` or `BAG`).
    #[must_use]
    pub fn with_aggregation(mut self, aggregation: Aggregation) -> Self {
        self.aggregation = Some(aggregation);
        self
    }
}

/// One `UNIQUE` rule: the named attributes are unique, jointly, across all
/// instances of the entity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UniqueRule {
    /// Rule label, e.g. `UR1`; `None` when the rule is unlabelled.
    pub label: Option<String>,
    /// Attribute names as written; qualified names (`SELF\X.Y`) are kept.
    pub attributes: Vec<String>,
}

impl UniqueRule {
    /// Creates a labelled rule over `attributes`.
    #[must_use]
    pub fn new(label: impl Into<String>, attributes: Vec<String>) -> Self {
        Self {
            label: Some(label.into()),
            attributes,
        }
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
#[non_exhaustive]
pub struct WhereRule {
    /// Rule label as declared, e.g. `CurveIs3D`.
    pub label: String,
    /// Constraint expression as written, whitespace-normalised.
    pub expression: String,
}

impl WhereRule {
    /// Creates a rule.
    #[must_use]
    pub fn new(label: impl Into<String>, expression: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            expression: expression.into(),
        }
    }
}

/// One explicit redeclaration of an inherited attribute: `SELF\X.a : T;`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Redeclaration {
    /// The supertype named in the qualifier (`X`), as written.
    pub supertype: String,
    /// The inherited attribute (`a`), unqualified.
    pub name: String,
    /// The narrowed type token, extracted like [`Attribute::type_name`].
    pub type_name: String,
}

impl Redeclaration {
    /// Creates a redeclaration of the inherited `supertype.name`.
    #[must_use]
    pub fn new(
        supertype: impl Into<String>,
        name: impl Into<String>,
        type_name: impl Into<String>,
    ) -> Self {
        Self {
            supertype: supertype.into(),
            name: name.into(),
            type_name: type_name.into(),
        }
    }
}

/// One structural entity declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// `INVERSE` attributes declared by this entity, in declaration order.
    pub inverses: Vec<InverseAttribute>,
    /// `UNIQUE` rules declared by this entity, in declaration order.
    pub unique_rules: Vec<UniqueRule>,
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
            inverses: Vec::new(),
            unique_rules: Vec::new(),
        }
    }

    /// Marks the entity `ABSTRACT`.
    #[must_use]
    pub const fn abstract_(mut self) -> Self {
        self.abstract_ = true;
        self
    }

    /// Appends a `WHERE` rule.
    #[must_use]
    pub fn with_where_rule(mut self, rule: WhereRule) -> Self {
        self.where_rules.push(rule);
        self
    }

    /// Appends an `INVERSE` attribute.
    #[must_use]
    pub fn with_inverse(mut self, inverse: InverseAttribute) -> Self {
        self.inverses.push(inverse);
        self
    }

    /// Appends a `UNIQUE` rule.
    #[must_use]
    pub fn with_unique_rule(mut self, rule: UniqueRule) -> Self {
        self.unique_rules.push(rule);
        self
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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct TypeDef {
    /// Declared type name.
    pub name: String,
    /// Structurally recognized declaration kind.
    pub kind: TypeKind,
}

impl TypeDef {
    /// Creates a type declaration.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: TypeKind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }

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
    let inverses = block_statements(block, &upper, "INVERSE", header_end + 1)
        .filter_map(parse_inverse)
        .collect();
    let unique_rules = block_statements(block, &upper, "UNIQUE", header_end + 1)
        .filter_map(parse_unique_rule)
        .collect();

    Some(EntityDef {
        name,
        supertypes,
        abstract_,
        attributes,
        derived,
        redeclared,
        where_rules,
        inverses,
        unique_rules,
    })
}

/// The `;`-separated statements of one entity block (`INVERSE`, `UNIQUE`),
/// from its statement-level keyword to the next block keyword or
/// `END_ENTITY`. Blocks appear in the order DERIVE, INVERSE, UNIQUE, WHERE.
fn block_statements<'a>(
    block: &'a str,
    upper: &str,
    keyword: &'static str,
    from: usize,
) -> impl Iterator<Item = &'a str> {
    let range = find_block_keyword(block, upper, keyword, from).and_then(|start| {
        let start = start + keyword.len();
        let end = ["INVERSE", "UNIQUE", "WHERE", "END_ENTITY"]
            .into_iter()
            .filter(|next| *next != keyword)
            .filter_map(|next| find_block_keyword(block, upper, next, start))
            .min()
            .unwrap_or(block.len());
        (start < end).then_some(start..end)
    });
    range
        .map_or("", |range| &block[range])
        .split(';')
        .filter(|statement| !statement.trim().is_empty())
}

/// Parse `Name : [SET|BAG [l:u] OF] Entity FOR Attribute`.
fn parse_inverse(statement: &str) -> Option<InverseAttribute> {
    let (target, declaration) = statement.split_once(':')?;
    let target = target.trim();
    let (redeclares, name) = match target
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("SELF\\"))
    {
        Some(_) => {
            let (supertype, name) = target[5..].split_once('.')?;
            (Some(supertype.trim().to_owned()), name.trim())
        }
        None => (None, target),
    };
    if name.is_empty() || !name.bytes().all(is_identifier_byte) {
        return None;
    }
    let upper = ascii_uppercase(declaration);
    let for_position = find_keyword(&upper, "FOR", 0)?;
    let for_attribute = declaration[for_position + "FOR".len()..].trim();
    let (aggregation, element) = aggregation_levels(&declaration[..for_position]);
    let entity = element.split_whitespace().next()?;
    if for_attribute.is_empty() || aggregation.len() > 1 {
        return None;
    }
    Some(InverseAttribute {
        name: name.to_owned(),
        redeclares,
        entity: entity.to_owned(),
        for_attribute: for_attribute.split_whitespace().collect(),
        aggregation: aggregation.into_iter().next(),
    })
}

/// Parse `[Label :] a, b, ...` from a `UNIQUE` block.
fn parse_unique_rule(statement: &str) -> Option<UniqueRule> {
    // A qualified attribute (`SELF\X.Y`) contains no `:`, so a `:` can only
    // end the label.
    let (label, attributes) = match statement.split_once(':') {
        Some((label, attributes)) => {
            let label = label.trim();
            if label.is_empty() || !label.bytes().all(is_identifier_byte) {
                return None;
            }
            (Some(label.to_owned()), attributes)
        }
        None => (None, statement),
    };
    let attributes: Vec<String> = attributes
        .split(',')
        .map(|attribute| attribute.split_whitespace().collect::<String>())
        .filter(|attribute| !attribute.is_empty())
        .collect();
    (!attributes.is_empty()).then_some(UniqueRule { label, attributes })
}

/// Split the aggregation levels off a type expression, outermost first,
/// returning them and the remaining element type text.
///
/// `LIST [2:3] OF UNIQUE LIST [1:?] OF IfcLengthMeasure` yields two levels
/// and `IfcLengthMeasure`. Bounds are optional for `LIST`, `SET` and `BAG`
/// (then `[0:?]`); `ARRAY` requires them but is not rejected without.
fn aggregation_levels(declaration: &str) -> (Vec<Aggregation>, &str) {
    let mut levels = Vec::new();
    let mut rest = declaration.trim_start();
    loop {
        let word_end = rest
            .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .unwrap_or(rest.len());
        let Some(kind) = AggregateKind::from_keyword(&rest[..word_end].to_ascii_uppercase()) else {
            return (levels, rest);
        };
        let mut after = rest[word_end..].trim_start();
        let (lower, upper) = if let Some(inner) = after.strip_prefix('[') {
            let Some(close) = inner.find(']') else {
                return (levels, rest);
            };
            let bounds = &inner[..close];
            after = inner[close + 1..].trim_start();
            match bounds.split_once(':') {
                Some((lower, upper)) => (Bound::parse(lower), Bound::parse(upper)),
                None => (Bound::parse(bounds), Bound::parse(bounds)),
            }
        } else {
            (Bound::Integer(0), Bound::Unbounded)
        };
        let Some(of) = after
            .get(..2)
            .filter(|word| word.eq_ignore_ascii_case("OF"))
            .map(|_| after[2..].trim_start())
        else {
            return (levels, rest);
        };
        let mut level = Aggregation::new(kind, lower, upper);
        rest = of;
        for (keyword, mark) in [
            (
                "OPTIONAL",
                Aggregation::optional_elements as fn(Aggregation) -> Aggregation,
            ),
            ("UNIQUE", Aggregation::unique),
        ] {
            if rest
                .get(..keyword.len())
                .is_some_and(|word| word.eq_ignore_ascii_case(keyword))
                && !rest[keyword.len()..]
                    .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            {
                level = mark(level);
                rest = rest[keyword.len()..].trim_start();
            }
        }
        levels.push(level);
    }
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
    let mut declaration = declaration.trim();
    let optional = declaration
        .get(.."OPTIONAL".len())
        .is_some_and(|word| word.eq_ignore_ascii_case("OPTIONAL"))
        && !declaration["OPTIONAL".len()..]
            .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
    if optional {
        declaration = declaration["OPTIONAL".len()..].trim_start();
    }
    let (aggregation, element) = aggregation_levels(declaration);
    let type_name = element
        .split_whitespace()
        .next()?
        .trim_matches(|character: char| matches!(character, '(' | ')' | ';'))
        .to_owned();
    Some(Attribute {
        name: name.to_owned(),
        type_name,
        optional,
        aggregate: !aggregation.is_empty(),
        aggregation,
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
