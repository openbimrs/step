//! Complex entity instances: the Part 21 external mapping.
//!
//! A complex instance (`#1=(A(..)B(..)C(..));`) combines several entity
//! types that no single declared entity covers -- how AP203/214/242 write a
//! rational B-spline curve, or a representation context with units and an
//! uncertainty. ISO 10303-21:2016 §12.2.5.3 fixes its shape:
//!
//! - one partial record per entity type in the instance, **including every
//!   supertype**, even one with no attributes (NOTE 2);
//! - partial records in ascending order of entity name, compared as
//!   upper-case octets (§5.2), so `_` (0x5F) sorts after every letter;
//! - each partial record carries only the explicit attributes **its own**
//!   entity declares, in declaration order -- never the inherited ones.
//!
//! The last rule is why [`SchemaGraph::attributes`] is the wrong query here:
//! it answers the internal mapping, where one record holds the whole
//! inherited layout. [`SchemaGraph::resolve_complex`] answers this one.
//!
//! Problems are reported as [`ComplexIssue`]s rather than an error, and the
//! parts that did resolve are still returned: a consumer reading a slightly
//! non-conforming file can still interpret everything it recognises.

use std::collections::{BTreeSet, HashSet};

use super::SchemaGraph;
use crate::express::{Attribute, EntityDef};

/// A complex instance's partial records resolved against a schema.
///
/// Returned by [`SchemaGraph::resolve_complex`]. Parts are kept in the order
/// they were given, one per input name, so `parts()[i]` always describes the
/// `i`-th partial record of the instance.
#[derive(Debug, Clone)]
pub struct ComplexLayout<'s> {
    parts: Vec<ComplexPart<'s>>,
    types: Vec<&'s str>,
    issues: Vec<ComplexIssue>,
}

impl<'s> ComplexLayout<'s> {
    /// One entry per partial record, in input order.
    #[must_use]
    pub fn parts(&self) -> &[ComplexPart<'s>] {
        &self.parts
    }

    /// Every entity type the instance is: each known part plus all of its
    /// supertypes, once each, sorted by declared name.
    ///
    /// For a conforming instance this is exactly the set of parts. When a
    /// supertype is missing it still appears here, so `types` always
    /// describes the whole instance; the omission is reported as
    /// [`ComplexIssue::MissingSupertype`].
    #[must_use]
    pub fn types(&self) -> &[&'s str] {
        &self.types
    }

    /// Every way the instance departs from §12.2.5.3, in a stable order:
    /// per-part issues by index, then missing supertypes by name.
    #[must_use]
    pub fn issues(&self) -> &[ComplexIssue] {
        &self.issues
    }

    /// Whether the instance conforms: every part known, in order, no part
    /// repeated, and every supertype present.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

/// One partial record of a complex instance.
#[derive(Debug, Clone)]
pub struct ComplexPart<'s> {
    name: String,
    entity: Option<&'s EntityDef>,
    slots: Vec<ComplexSlot<'s>>,
}

impl<'s> ComplexPart<'s> {
    /// The partial record's name as given.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The entity declaration it names, or `None` if the schema has none
    /// (reported as [`ComplexIssue::UnknownType`]).
    #[must_use]
    pub fn entity(&self) -> Option<&'s EntityDef> {
        self.entity
    }

    /// The partial record's positional slots: the explicit attributes the
    /// entity itself declares, in declaration order. Empty for an unknown
    /// part and for an entity that declares none.
    #[must_use]
    pub fn slots(&self) -> &[ComplexSlot<'s>] {
        &self.slots
    }
}

/// One positional slot of a partial record.
#[derive(Debug, Clone, Copy)]
pub struct ComplexSlot<'s> {
    attribute: &'s Attribute,
    derived: bool,
}

impl<'s> ComplexSlot<'s> {
    /// The attribute declaration occupying this slot.
    #[must_use]
    pub fn attribute(&self) -> &'s Attribute {
        self.attribute
    }

    /// Whether another type in the instance redeclares this attribute as
    /// derived (`DERIVE SELF\X.a : T := ...;`).
    ///
    /// A conforming file writes such a slot as `*` (§12.2.6). This is a
    /// property of the whole instance, not of the declaring entity alone:
    /// `NAMED_UNIT(*)` in `(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(..))` is `*`
    /// because `si_unit` derives `named_unit.dimensions`.
    #[must_use]
    pub fn is_derived(&self) -> bool {
        self.derived
    }
}

/// A way a complex instance departs from ISO 10303-21:2016 §12.2.5.3.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ComplexIssue {
    /// Part `index` names no entity in the schema.
    UnknownType {
        /// Position of the part in the input.
        index: usize,
    },
    /// Part `index` does not sort after the part before it. Names must be in
    /// strictly ascending order of their upper-case octets.
    OutOfOrder {
        /// Position of the part in the input.
        index: usize,
    },
    /// Part `index` repeats an earlier part (names compare case-insensitively).
    Duplicate {
        /// Position of the part in the input.
        index: usize,
    },
    /// A supertype of some part has no partial record of its own. §12.2.5.3
    /// NOTE 2 requires one even when it declares no attributes.
    MissingSupertype {
        /// The supertype's declared name.
        name: String,
    },
}

impl SchemaGraph {
    /// Resolves a complex entity instance's partial records against this
    /// schema (ISO 10303-21:2016 §12.2.5.3, the external mapping).
    ///
    /// `names` are the partial record names in the order the instance
    /// writes them, e.g. `["BOUNDED_CURVE", "B_SPLINE_CURVE", ...]`. Matching
    /// is case-insensitive. Each resolved part's slots are the explicit
    /// attributes its entity *itself* declares -- compare them with the
    /// partial record's parameters one to one.
    ///
    /// Never fails: unknown, repeated, misordered or missing types are
    /// reported through [`ComplexLayout::issues`] and every part that could
    /// be resolved still is.
    #[must_use]
    pub fn resolve_complex(&self, names: &[&str]) -> ComplexLayout<'_> {
        let mut issues = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut previous: Option<String> = None;
        let entities: Vec<Option<&EntityDef>> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let upper = name.to_ascii_uppercase();
                let entity = self.entity(name);
                if entity.is_none() {
                    issues.push(ComplexIssue::UnknownType { index });
                }
                if !seen.insert(upper.clone()) {
                    issues.push(ComplexIssue::Duplicate { index });
                } else if previous.as_ref().is_some_and(|prev| upper <= *prev) {
                    issues.push(ComplexIssue::OutOfOrder { index });
                }
                previous = Some(upper);
                entity
            })
            .collect();

        // The supertype closure. Walking `supertypes` covers diamonds and
        // multiple inheritance, and is already cycle-safe.
        let mut types: BTreeSet<&str> = BTreeSet::new();
        for entity in entities.iter().flatten() {
            types.insert(entity.name.as_str());
            types.extend(self.supertypes(&entity.name));
        }
        let mut missing: Vec<&str> = types
            .iter()
            .copied()
            .filter(|name| !seen.contains(&name.to_ascii_uppercase()))
            .collect();
        missing.sort_unstable_by_key(|name| name.to_ascii_uppercase());
        issues.extend(
            missing
                .into_iter()
                .map(|name| ComplexIssue::MissingSupertype {
                    name: name.to_owned(),
                }),
        );

        // An attribute is written `*` when any type in the instance derives
        // it. Collected over the whole closure, not just the parts, so a
        // derivation declared on a missing supertype still counts.
        let derived: HashSet<(String, String)> = types
            .iter()
            .filter_map(|name| self.entity(name))
            .flat_map(|entity| self.derivations(entity))
            .collect();

        let parts = names
            .iter()
            .zip(entities)
            .map(|(name, entity)| ComplexPart {
                name: (*name).to_owned(),
                entity,
                slots: entity.map_or_else(Vec::new, |entity| {
                    let owner = entity.name.to_ascii_uppercase();
                    entity
                        .attributes
                        .iter()
                        .map(|attribute| ComplexSlot {
                            attribute,
                            derived: derived
                                .contains(&(owner.clone(), attribute.name.to_ascii_uppercase())),
                        })
                        .collect()
                }),
            })
            .collect();

        ComplexLayout {
            parts,
            types: types.into_iter().collect(),
            issues,
        }
    }

    /// `(declaring entity, attribute)` pairs, upper-cased, that `entity`
    /// redeclares as derived.
    ///
    /// `EntityDef::derived` stores names unqualified, so each is attributed
    /// to the nearest supertype that declares an explicit attribute of that
    /// name -- the one the `SELF\X.` prefix named. A derived name matching
    /// no inherited attribute is a new derived attribute, not a
    /// redeclaration, and occupies no slot.
    fn derivations(&self, entity: &EntityDef) -> Vec<(String, String)> {
        entity
            .derived
            .iter()
            .filter_map(|name| {
                self.supertypes(&entity.name)
                    .into_iter()
                    .filter_map(|ancestor| self.entity(ancestor))
                    .find(|ancestor| {
                        ancestor
                            .attributes
                            .iter()
                            .any(|attribute| attribute.name.eq_ignore_ascii_case(name))
                    })
                    .map(|ancestor| {
                        (
                            ancestor.name.to_ascii_uppercase(),
                            name.to_ascii_uppercase(),
                        )
                    })
            })
            .collect()
    }
}
