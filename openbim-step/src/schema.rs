//! The parsed schema as a queryable graph.
//!
//! # Why this is not in `express`
//!
//! [`express::parse`](crate::express::parse) answers "what does this source
//! text declare?". This module answers "given those declarations, what is
//! true of entity X?" -- supertype chains, positional attribute order, type
//! resolution. Those are questions about ISO 10303-11 semantics rather than
//! syntax, and every schema-aware consumer needs them before it can interpret
//! a positional Part 21 record.
//!
//! # Why it is not in a downstream crate
//!
//! It was in one. An application-schema crate carried this logic, but nothing
//! here is specific to any single schema: AP203, AP214, AP242 and IFC are all
//! EXPRESS schemas serialized as Part 21 records with identical inheritance
//! and positional rules. Every consumer would otherwise reimplement it.
//! Application layers keep what is genuinely theirs -- which schema version a
//! file declares, and any bundled copy of their own tables.
//!
//! # The positional rule this exists to enforce
//!
//! A Part 21 record lists attributes **supertype-first, most general first**:
//!
//! ```text
//! ENTITY Base;                    Id, Owner, Name
//! ENTITY Derived SUBTYPE OF (Base);   ... then Derived's own
//!
//! #1=DERIVED('id',$,'Name',$, ...);
//!             ^0   ^1  ^2    ^3     inherited slots come first
//! ```
//!
//! Reversing that order misreads every attribute of every inheriting entity,
//! silently, and the values still look plausible. That is why
//! [`SchemaGraph::attributes`] is tested against a three-level chain rather
//! than a synthetic two-level one -- two levels cannot distinguish
//! "inherited first" from "declaring entity first".

use std::collections::HashMap;

use crate::express::{Attribute, EntityDef, ParsedSchema, TypeDef, TypeKind};

/// Longest supertype or alias chain this will walk before giving up.
///
/// A cyclic `SUBTYPE OF` is not legal EXPRESS, but this parser is deliberately
/// tolerant and a malformed source must not hang a consumer. Real schemas nest
/// around a dozen levels; 64 leaves generous room while still terminating.
const MAX_CHAIN_DEPTH: usize = 64;

/// Case-insensitive lookup over a parsed schema's declarations.
///
/// EXPRESS identifiers are case-insensitive, and Part 21 records conventionally
/// spell entity names in upper case (`MYENTITY`) while schema sources spell
/// them in mixed case (`MyEntity`). Every lookup here folds case so callers can
/// pass whichever spelling they hold.
#[derive(Debug, Clone)]
pub struct SchemaGraph {
    name: String,
    entities: HashMap<String, EntityDef>,
    types: HashMap<String, TypeDef>,
}

impl SchemaGraph {
    /// Indexes a parsed schema for querying.
    #[must_use]
    pub fn new(parsed: ParsedSchema) -> Self {
        let entities = parsed
            .entities
            .into_iter()
            .map(|entity| (entity.name.to_ascii_uppercase(), entity))
            .collect();
        let types = parsed
            .types
            .into_iter()
            .map(|type_def| (type_def.name.to_ascii_uppercase(), type_def))
            .collect();
        Self {
            name: parsed.name,
            entities,
            types,
        }
    }

    /// Parses EXPRESS source and indexes it in one step.
    #[must_use]
    pub fn from_express(source: &str) -> Self {
        Self::new(crate::express::parse(source))
    }

    /// The declared schema name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// How many entity declarations the schema holds.
    #[must_use]
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// How many type declarations the schema holds.
    #[must_use]
    pub fn type_count(&self) -> usize {
        self.types.len()
    }

    /// The entity declaration for `name`, if the schema declares one.
    #[must_use]
    pub fn entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.get(&name.to_ascii_uppercase())
    }

    /// The type declaration for `name`, if the schema declares one.
    #[must_use]
    pub fn type_def(&self, name: &str) -> Option<&TypeDef> {
        self.types.get(&name.to_ascii_uppercase())
    }

    /// Every entity name the schema declares, in unspecified order.
    ///
    /// Callers needing determinism must sort: the underlying map order is
    /// deliberately not promised.
    pub fn entity_names(&self) -> impl Iterator<Item = &str> {
        self.entities.values().map(|entity| entity.name.as_str())
    }

    /// Whether `name` is `ancestor`, or inherits from it.
    ///
    /// Reflexive for declared entities, matching EXPRESS subtype semantics
    /// where a type belongs to its own subtype set. An entity the schema never
    /// declares is not a subtype of anything, including itself -- otherwise a
    /// typo would silently satisfy every check made against it.
    #[must_use]
    pub fn is_a(&self, name: &str, ancestor: &str) -> bool {
        if name.eq_ignore_ascii_case(ancestor) {
            return self.entities.contains_key(&name.to_ascii_uppercase());
        }
        self.supertypes(name)
            .iter()
            .any(|super_name| super_name.eq_ignore_ascii_case(ancestor))
    }

    /// The supertype chain above `name`, nearest parent first.
    ///
    /// Excludes `name` itself. Bounded by a fixed depth limit so a malformed
    /// cyclic schema terminates instead of hanging.
    #[must_use]
    pub fn supertypes(&self, name: &str) -> Vec<&str> {
        let mut chain = Vec::new();
        let mut current = self.entities.get(&name.to_ascii_uppercase());
        for _ in 0..MAX_CHAIN_DEPTH {
            let Some(def) = current else { break };
            let Some(supertype) = def.supertype.as_ref() else {
                break;
            };
            let Some(parent) = self.entities.get(&supertype.to_ascii_uppercase()) else {
                // The source names a supertype it never declares. Report the
                // name anyway: a consumer checking `is_a` against a partial
                // schema should still see the declared relationship.
                chain.push(supertype.as_str());
                break;
            };
            chain.push(parent.name.as_str());
            current = Some(parent);
        }
        chain
    }

    /// Every attribute slot in **Part 21 positional order**, inherited first.
    ///
    /// See the module documentation for why this ordering is load-bearing.
    ///
    /// Derived redeclarations are *included*: they keep their inherited
    /// position and are written `*` in a Part 21 record. Use
    /// [`EntityDef::is_derived`] on the owning entity to tell them apart.
    #[must_use]
    pub fn attributes(&self, name: &str) -> Vec<&Attribute> {
        let mut chain: Vec<&EntityDef> = Vec::new();
        let mut current = self.entities.get(&name.to_ascii_uppercase());
        for _ in 0..MAX_CHAIN_DEPTH {
            let Some(def) = current else { break };
            chain.push(def);
            let Some(supertype) = def.supertype.as_ref() else {
                break;
            };
            current = self.entities.get(&supertype.to_ascii_uppercase());
        }
        chain.reverse();
        chain.iter().flat_map(|def| def.attributes.iter()).collect()
    }

    /// Attribute names in positional order.
    ///
    /// The bridge a positional-to-named mapping needs: slot `i` is called
    /// `names[i]`.
    #[must_use]
    pub fn attribute_names(&self, name: &str) -> Vec<&str> {
        self.attributes(name)
            .into_iter()
            .map(|attribute| attribute.name.as_str())
            .collect()
    }

    /// Resolves a defined type to the base it ultimately aliases.
    ///
    /// A chain such as `PositiveCount -> Count -> INTEGER` resolves to
    /// `INTEGER`. Returns the final right-hand side; for a declaration that is
    /// not an alias, that is the type's own name. Bounded like the supertype
    /// walk so a cyclic alias cannot hang.
    ///
    /// The right-hand side is returned verbatim, including any aggregate
    /// syntax (`LIST [1:?] OF X`): discarding it here would throw away the
    /// aggregate fact entirely, and callers that want the base scalar can
    /// match on the text they receive.
    #[must_use]
    pub fn resolve_defined(&self, name: &str) -> String {
        let mut current = name.to_string();
        for _ in 0..MAX_CHAIN_DEPTH {
            let Some(def) = self.type_def(&current) else {
                return current;
            };
            let TypeKind::Defined(target) = &def.kind else {
                return current;
            };
            let next = target.trim().to_string();
            if next.eq_ignore_ascii_case(&current) {
                return current;
            }
            current = next;
        }
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real three-level chain: two levels cannot catch ordering bugs.
    const CHAIN: &str = "\
SCHEMA DEMO;
ENTITY Base
 ABSTRACT SUPERTYPE OF (ONEOF(Middle));
  Id : Identifier;
  Owner : OPTIONAL Party;
  Name : OPTIONAL Label;
  Description : OPTIONAL Text;
END_ENTITY;
ENTITY Middle
 ABSTRACT SUPERTYPE OF (ONEOF(Leaf))
 SUBTYPE OF (Base);
END_ENTITY;
ENTITY Leaf
 SUBTYPE OF (Middle);
  Kind : OPTIONAL Label;
END_ENTITY;
TYPE Count = INTEGER; END_TYPE;
TYPE PositiveCount = Count; END_TYPE;
TYPE Colour = ENUMERATION OF (RED, GREEN, NOTDEFINED); END_TYPE;
END_SCHEMA;";

    fn graph() -> SchemaGraph {
        SchemaGraph::from_express(CHAIN)
    }

    #[test]
    fn inherited_attributes_come_first_in_positional_order() {
        assert_eq!(
            graph().attribute_names("LEAF"),
            ["Id", "Owner", "Name", "Description", "Kind"],
            "Base's slots must precede Leaf's own"
        );
    }

    #[test]
    fn subtype_tests_cross_intermediate_levels() {
        let schema = graph();
        assert!(schema.is_a("LEAF", "Base"), "grandparent");
        assert!(schema.is_a("Leaf", "Middle"), "parent");
        assert!(schema.is_a("Leaf", "Leaf"), "reflexive");
        assert!(!schema.is_a("Base", "Leaf"), "not upward");
    }

    /// An entity the schema never declares is not a subtype even of itself.
    #[test]
    fn an_undeclared_entity_is_not_a_subtype_even_of_itself() {
        assert!(!graph().is_a("NotAThing", "NotAThing"));
    }

    #[test]
    fn defined_types_resolve_through_the_alias_chain() {
        assert_eq!(graph().resolve_defined("PositiveCount"), "INTEGER");
    }

    /// A non-alias declaration resolves to itself.
    #[test]
    fn an_enumeration_resolves_to_its_own_name() {
        assert_eq!(graph().resolve_defined("Colour"), "Colour");
    }

    /// A cyclic supertype must terminate rather than hang.
    #[test]
    fn a_cyclic_supertype_chain_terminates() {
        let schema = SchemaGraph::from_express(
            "SCHEMA S;\
             ENTITY A SUBTYPE OF (B); END_ENTITY;\
             ENTITY B SUBTYPE OF (A); END_ENTITY;\
             END_SCHEMA;",
        );
        assert!(schema.supertypes("A").len() <= MAX_CHAIN_DEPTH);
    }

    /// A cyclic alias must terminate rather than hang.
    #[test]
    fn a_cyclic_alias_chain_terminates() {
        let schema = SchemaGraph::from_express(
            "SCHEMA S; TYPE A = B; END_TYPE; TYPE B = A; END_TYPE; END_SCHEMA;",
        );
        let resolved = schema.resolve_defined("A");
        assert!(resolved == "A" || resolved == "B");
    }

    /// A supertype the schema never declares is still reported.
    #[test]
    fn an_undeclared_supertype_is_still_named() {
        let schema = SchemaGraph::from_express(
            "SCHEMA S; ENTITY A SUBTYPE OF (Missing); END_ENTITY; END_SCHEMA;",
        );
        assert_eq!(schema.supertypes("A"), ["Missing"]);
        assert!(schema.is_a("A", "Missing"));
    }
}
