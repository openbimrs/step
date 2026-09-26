//! Generic, owned semantic data for a STEP physical file.
//!
//! The string storage type is generic. The default, [`Str`], is what
//! [`parse`](crate::parse) produces: short values inline, longer ones
//! shared. Borrowed APIs use `Cow<'a, str>`; applications may construct
//! records with any string type.

use std::fmt;
use std::sync::Arc;

/// An arbitrary-precision instance identifier as written by `#42`.
///
/// The Part 21 grammar does not impose a machine-integer bound, so the decimal
/// digits are retained lexically.
///
/// Up to 22 digits -- every id a `u64` can hold -- are stored in the value
/// itself; only longer ones allocate. A large file holds several
/// ids per record, so this removes millions of allocations per parse.
/// Comparison, hashing, ordering and `Debug` all go through [`Self::as_str`],
/// so the representation is invisible.
#[derive(Clone)]
pub struct InstanceId(Digits);

/// Digits stored inline; 22 keeps `InstanceId` at 24 bytes.
const INLINE_DIGITS: usize = 22;

#[derive(Clone)]
enum Digits {
    Inline { len: u8, bytes: [u8; INLINE_DIGITS] },
    Heap(Box<str>),
}

impl InstanceId {
    /// Creates an identifier from non-empty ASCII decimal digits.
    #[must_use]
    pub fn new(value: &str) -> Option<Self> {
        Self::from_ascii_digits(value.as_bytes())
    }

    /// [`Self::new`] over bytes, for the parser: the lexer hands ids over as
    /// bytes, and checking digits directly skips a separate UTF-8 pass.
    pub(crate) fn from_ascii_digits(value: &[u8]) -> Option<Self> {
        (!value.is_empty() && value.iter().all(u8::is_ascii_digit))
            .then(|| Self::from_digits(value))
    }

    /// Stores already-validated ASCII digits.
    pub(crate) fn from_digits(value: &[u8]) -> Self {
        if value.len() <= INLINE_DIGITS {
            let mut bytes = [0; INLINE_DIGITS];
            bytes[..value.len()].copy_from_slice(value);
            Self(Digits::Inline {
                len: u8::try_from(value.len()).expect("INLINE_DIGITS fits in u8"),
                bytes,
            })
        } else {
            // ASCII digits are valid UTF-8, so this cannot fail.
            let text = std::str::from_utf8(value).expect("ASCII digits are UTF-8");
            Self(Digits::Heap(text.into()))
        }
    }

    /// Returns the decimal digits without the leading `#`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            Digits::Inline { len, bytes } => {
                // Only `from_digits` builds `Inline`, from validated ASCII
                // digits, so this conversion cannot fail.
                std::str::from_utf8(&bytes[..usize::from(*len)]).unwrap_or_default()
            }
            Digits::Heap(digits) => digits,
        }
    }
}

impl PartialEq for InstanceId {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for InstanceId {}

impl std::hash::Hash for InstanceId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl PartialOrd for InstanceId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InstanceId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl fmt::Debug for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("InstanceId")
            .field(&self.as_str())
            .finish()
    }
}

impl From<u64> for InstanceId {
    fn from(value: u64) -> Self {
        Self::from_digits(value.to_string().as_bytes())
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}", self.as_str())
    }
}

/// The owned model's string: values of up to 22 bytes -- nearly every
/// number, GUID and entity name in a STEP file -- are stored inline without
/// an allocation, and longer ones are one shared `Arc<str>`.
///
/// [`parse`](crate::parse) hands out one allocation per distinct long name
/// (record names, typed-value names, enumeration values) and shares it, so a
/// file's millions of name uses cost a few hundred allocations. Cloning never
/// copies the text. Equality, ordering and hashing are those of the `str`,
/// so the representation is invisible.
#[derive(Clone)]
pub struct Str(StrRepr);

/// Bytes stored inline; 22 keeps [`Str`] at 24 bytes, the size of a `String`.
const INLINE_TEXT: usize = 22;

#[derive(Clone)]
enum StrRepr {
    Inline { len: u8, bytes: [u8; INLINE_TEXT] },
    Shared(Arc<str>),
}

impl Str {
    /// The text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match &self.0 {
            // Only `Str::from` builds `Inline`, from a `str`, so the bytes
            // are valid UTF-8 and this cannot fail.
            StrRepr::Inline { len, bytes } => {
                std::str::from_utf8(&bytes[..usize::from(*len)]).unwrap_or_default()
            }
            StrRepr::Shared(text) => text,
        }
    }

    /// Whether this value shares its text with other clones rather than
    /// storing it inline. Short values are never shared: copying them is
    /// cheaper than an allocation.
    #[must_use]
    pub fn is_shared(&self) -> bool {
        matches!(self.0, StrRepr::Shared(_))
    }

    /// Whether two values share one allocation.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (StrRepr::Shared(a), StrRepr::Shared(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Short text inline, `None` when it does not fit.
    fn inline(text: &str) -> Option<Self> {
        let mut bytes = [0; INLINE_TEXT];
        bytes
            .get_mut(..text.len())?
            .copy_from_slice(text.as_bytes());
        Some(Self(StrRepr::Inline {
            len: u8::try_from(text.len()).expect("INLINE_TEXT fits in u8"),
            bytes,
        }))
    }
}

impl From<&str> for Str {
    fn from(text: &str) -> Self {
        Self::inline(text).unwrap_or_else(|| Self(StrRepr::Shared(Arc::from(text))))
    }
}

impl From<String> for Str {
    fn from(text: String) -> Self {
        Self::inline(&text).unwrap_or_else(|| Self(StrRepr::Shared(Arc::from(text))))
    }
}

impl From<Arc<str>> for Str {
    /// Keeps sharing a long text; a short one is stored inline.
    fn from(text: Arc<str>) -> Self {
        Self::inline(&text).unwrap_or(Self(StrRepr::Shared(text)))
    }
}

impl From<&Str> for String {
    fn from(text: &Str) -> Self {
        text.as_str().to_owned()
    }
}

impl std::ops::Deref for Str {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Str {}

impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Str {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Str {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl std::hash::Hash for Str {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for Str {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl fmt::Display for Str {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Default for Str {
    fn default() -> Self {
        Self::from("")
    }
}

/// One generic parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Parameter<S = Str> {
    /// `$`, an omitted value.
    Null,
    /// `*`, a derived value.
    Derived,
    /// `.T.` or `.F.`.
    Bool(bool),
    /// `.U.`, the third logical state.
    LogicalUnknown,
    /// Integer lexical form, preserved without a fixed precision limit.
    Integer(S),
    /// Real lexical form, preserved without a fixed precision limit.
    Real(S),
    /// Decoded text.
    Text(S),
    /// Binary digits without surrounding quotes.
    Binary(S),
    /// An enumeration name without surrounding dots.
    Enum(S),
    /// A `#` reference.
    Ref(InstanceId),
    /// A parenthesized aggregate. Boxed rather than a `Vec`: an aggregate
    /// never grows after parsing, so it carries no capacity.
    List(Box<[Self]>),
    /// A named parameter wrapper.
    Typed {
        /// Wrapper name.
        type_name: S,
        /// Wrapped parameter. Multiple source arguments are represented by a
        /// [`Parameter::List`].
        value: Box<Self>,
    },
}

impl<S> Parameter<S> {
    /// Returns a referenced id when this is [`Parameter::Ref`].
    #[must_use]
    pub fn as_reference(&self) -> Option<InstanceId> {
        match self {
            Self::Ref(id) => Some(id.clone()),
            _ => None,
        }
    }

    /// Returns aggregate items when this is [`Parameter::List`].
    #[must_use]
    pub fn as_list(&self) -> Option<&[Self]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }

    /// Recursively removes typed wrappers.
    #[must_use]
    pub fn unwrap_typed(&self) -> &Self {
        match self {
            Self::Typed { value, .. } => value.unwrap_typed(),
            value => value,
        }
    }
}

impl<S: AsRef<str>> Parameter<S> {
    /// Returns text content when this is [`Parameter::Text`].
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text.as_ref()),
            _ => None,
        }
    }

    /// Returns a numeric approximation, accepting integer and real syntax.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Integer(value) | Self::Real(value) => value.as_ref().parse().ok(),
            _ => None,
        }
    }
}

/// A record in `HEADER;`.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderRecord<S = Str> {
    /// Record name.
    pub name: S,
    /// Positional parameters.
    pub parameters: Box<[Parameter<S>]>,
}

/// One named parameter record within a DATA instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Record<S = Str> {
    /// Record name.
    pub name: S,
    /// Positional parameters.
    pub parameters: Box<[Parameter<S>]>,
}

impl<S> Record<S> {
    /// Creates a record.
    #[must_use]
    pub fn new(name: S, parameters: impl Into<Box<[Parameter<S>]>>) -> Self {
        Self {
            name,
            parameters: parameters.into(),
        }
    }
}

/// The body of a `#id=...;` instance, as written.
///
/// Nearly every instance is simple, so a simple one holds its record inline
/// rather than in a one-element list. A complex instance keeps its records
/// in source order, and one written in complex form stays complex even when
/// it holds a single record: `#1=(A(1));` writes back as written.
#[derive(Debug, Clone, PartialEq)]
pub enum Instance<S = Str> {
    /// `NAME(...)`.
    Simple(Record<S>),
    /// `(A(...)B(...))`: the partial records of a complex instance.
    Complex(Box<[Record<S>]>),
}

/// A simple or complex `#id=...;` instance in `DATA;`.
#[derive(Debug, Clone, PartialEq)]
pub struct DataRecord<S = Str> {
    /// Source instance id.
    pub id: InstanceId,
    /// The instance body.
    pub instance: Instance<S>,
}

impl<S> DataRecord<S> {
    /// Creates a simple instance containing one named record.
    #[must_use]
    pub fn simple(id: InstanceId, name: S, parameters: impl Into<Box<[Parameter<S>]>>) -> Self {
        Self {
            id,
            instance: Instance::Simple(Record::new(name, parameters)),
        }
    }

    /// Creates a complex instance from its partial records.
    #[must_use]
    pub fn complex(id: InstanceId, records: impl Into<Box<[Record<S>]>>) -> Self {
        Self {
            id,
            instance: Instance::Complex(records.into()),
        }
    }

    /// The instance's records: one for a simple instance, the partial
    /// records in source order for a complex one.
    #[must_use]
    pub fn records(&self) -> &[Record<S>] {
        match &self.instance {
            Instance::Simple(record) => std::slice::from_ref(record),
            Instance::Complex(records) => records,
        }
    }

    /// Returns the record of a simple instance.
    #[must_use]
    pub fn as_simple(&self) -> Option<&Record<S>> {
        match &self.instance {
            Instance::Simple(record) => Some(record),
            Instance::Complex(_) => None,
        }
    }
}

/// The generic `HEADER; ... ENDSEC;` section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderSection<S = Str> {
    /// Records in source order, including unknown extension records.
    pub records: Vec<HeaderRecord<S>>,
}

/// Standard header fields projected from raw header records.
///
/// Every field is optional because the raw section may be incomplete. Calling
/// this projection never removes or rewrites the underlying records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StandardHeader {
    /// `FILE_DESCRIPTION` descriptions.
    pub description: Option<Vec<String>>,
    /// `FILE_DESCRIPTION` implementation level.
    pub implementation_level: Option<String>,
    /// `FILE_NAME` source name.
    pub name: Option<String>,
    /// `FILE_NAME` timestamp.
    pub time_stamp: Option<String>,
    /// `FILE_NAME` authors.
    pub author: Option<Vec<String>>,
    /// `FILE_NAME` organizations.
    pub organization: Option<Vec<String>>,
    /// `FILE_NAME` preprocessor.
    pub preprocessor_version: Option<String>,
    /// `FILE_NAME` originating system.
    pub originating_system: Option<String>,
    /// `FILE_NAME` authorization.
    pub authorization: Option<String>,
    /// `FILE_SCHEMA` schema identifiers.
    pub schema: Option<Vec<String>>,
}

impl<S: AsRef<str>> HeaderSection<S> {
    /// Projects the three standard header records into named fields.
    #[must_use]
    pub fn standard(&self) -> StandardHeader {
        let mut header = StandardHeader::default();
        for record in &self.records {
            match record.name.as_ref().to_ascii_uppercase().as_str() {
                "FILE_DESCRIPTION" => {
                    header.description = text_list(record.parameters.first());
                    header.implementation_level = text(record.parameters.get(1));
                }
                "FILE_NAME" => {
                    header.name = text(record.parameters.first());
                    header.time_stamp = text(record.parameters.get(1));
                    header.author = text_list(record.parameters.get(2));
                    header.organization = text_list(record.parameters.get(3));
                    header.preprocessor_version = text(record.parameters.get(4));
                    header.originating_system = text(record.parameters.get(5));
                    header.authorization = text(record.parameters.get(6));
                }
                "FILE_SCHEMA" => header.schema = text_list(record.parameters.first()),
                _ => {}
            }
        }
        header
    }
}

fn text<S: AsRef<str>>(parameter: Option<&Parameter<S>>) -> Option<String> {
    parameter?.as_text().map(ToOwned::to_owned)
}

fn text_list<S: AsRef<str>>(parameter: Option<&Parameter<S>>) -> Option<Vec<String>> {
    Some(
        parameter?
            .as_list()?
            .iter()
            .filter_map(Parameter::as_text)
            .map(ToOwned::to_owned)
            .collect(),
    )
}

/// The generic `DATA; ... ENDSEC;` section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DataSection<S = Str> {
    /// Records in source order.
    pub records: Vec<DataRecord<S>>,
}

impl<S> DataSection<S> {
    /// Finds a record by instance id.
    #[must_use]
    pub fn get(&self, id: &InstanceId) -> Option<&DataRecord<S>> {
        self.records.iter().find(|record| &record.id == id)
    }
}

/// A complete ISO 10303-21 exchange structure.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exchange<S = Str> {
    /// Header section.
    pub header: HeaderSection<S>,
    /// Data section.
    pub data: DataSection<S>,
}
