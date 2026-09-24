//! Generic, owned semantic data for a STEP physical file.
//!
//! The string storage type is generic. The default, [`String`], is convenient
//! for parsing owned exchanges; applications may construct records with an
//! interned string type of their choice.

use std::fmt;

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
        (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| Self::from_digits(value))
    }

    /// Stores already-validated digits.
    fn from_digits(value: &str) -> Self {
        if value.len() <= INLINE_DIGITS {
            let mut bytes = [0; INLINE_DIGITS];
            bytes[..value.len()].copy_from_slice(value.as_bytes());
            Self(Digits::Inline {
                len: u8::try_from(value.len()).expect("INLINE_DIGITS fits in u8"),
                bytes,
            })
        } else {
            Self(Digits::Heap(value.into()))
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
        Self::from_digits(&value.to_string())
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{}", self.as_str())
    }
}

/// One generic parameter value.
#[derive(Debug, Clone, PartialEq)]
pub enum Parameter<S = String> {
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
    /// A parenthesized aggregate.
    List(Vec<Self>),
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
pub struct HeaderRecord<S = String> {
    /// Record name.
    pub name: S,
    /// Positional parameters.
    pub parameters: Vec<Parameter<S>>,
}

/// One named parameter record within a DATA instance.
#[derive(Debug, Clone, PartialEq)]
pub struct Record<S = String> {
    /// Record name.
    pub name: S,
    /// Positional parameters.
    pub parameters: Vec<Parameter<S>>,
}

/// A simple or complex `#id=...;` instance in `DATA;`.
#[derive(Debug, Clone, PartialEq)]
pub struct DataRecord<S = String> {
    /// Source instance id.
    pub id: InstanceId,
    /// One record for a simple instance, multiple for a complex instance.
    pub records: Vec<Record<S>>,
}

impl<S> DataRecord<S> {
    /// Creates a simple instance containing one named record.
    #[must_use]
    pub fn simple(id: InstanceId, name: S, parameters: Vec<Parameter<S>>) -> Self {
        Self {
            id,
            records: vec![Record { name, parameters }],
        }
    }

    /// Returns the sole record of a simple instance.
    #[must_use]
    pub fn as_simple(&self) -> Option<&Record<S>> {
        (self.records.len() == 1).then(|| &self.records[0])
    }
}

/// The generic `HEADER; ... ENDSEC;` section.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeaderSection<S = String> {
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
pub struct DataSection<S = String> {
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
pub struct Exchange<S = String> {
    /// Header section.
    pub header: HeaderSection<S>,
    /// Data section.
    pub data: DataSection<S>,
}
