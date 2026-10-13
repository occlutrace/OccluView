//! Streaming estimate of the heap retained by the typed glTF JSON document.
//!
//! This pass walks the JSON with a visitor and never constructs a `Value` tree
//! or a `GltfDoc`. Its counters follow the typed `Vec` fields in `json.rs` and
//! recurse through the fields retained as `serde_json::Value`.
//! glTF structs must use their specified JSON object form; positional arrays
//! accepted by serde's derived struct deserializer are rejected here before
//! they can bypass accounting for retained fields.
//!
//! The estimate bounds vector growth with four initial slots and two more
//! slots per later item, and string growth with twice the decoded bytes plus
//! the small-string starting capacity. Opaque object maps use the default
//! `serde_json::Map` B-tree node layout (11 key/value slots, 12 child pointers,
//! and at least five entries in each non-root node). Allocator bookkeeping,
//! allocator size-class rounding, and the JSON deserializer's temporary string
//! scratch are outside this retained-storage subtotal; the caller's existing
//! source/JSON allowance represents parser working data separately. This is
//! an admission estimate, not a hard process-RSS guarantee.
//!
//! Opaque map keys are not retained in a temporary set, so repeated keys are
//! charged as separate possible entries. `serde_json` still applies its normal
//! last-value behavior when the typed document is parsed.

use std::fmt;
use std::mem::size_of;

use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::json;

const VEC_FIRST_CAPACITY: u64 = 4;
const BTREE_DEGREE: u64 = 6;
const BTREE_MAX_KEYS: u64 = 2 * BTREE_DEGREE - 1;
const BTREE_MIN_NON_ROOT_KEYS: u64 = BTREE_DEGREE - 1;
const BTREE_MAX_CHILDREN: u64 = 2 * BTREE_DEGREE;

/// Why the streaming metadata estimate stopped.
pub(super) enum MetadataAdmissionError {
    /// The input is not valid JSON.
    Json(serde_json::Error),
    /// The retained metadata estimate crossed the available budget.
    BudgetExceeded(u64),
}

/// Estimate dynamic storage retained by the typed document without building it.
pub(super) fn estimate_heap_bytes(
    json_bytes: &[u8],
    additional_budget: u64,
) -> Result<u64, MetadataAdmissionError> {
    let mut budget = Budget {
        used: 0,
        limit: additional_budget,
        exceeded: None,
    };
    let mut deserializer = serde_json::Deserializer::from_slice(json_bytes);
    let scan_result = ScanSeed {
        budget: &mut budget,
        schema: Schema::Object(ObjectKind::Root),
        vector_slot: None,
    }
    .deserialize(&mut deserializer);
    if let Err(error) = scan_result {
        return match budget.exceeded {
            Some(estimate) => Err(MetadataAdmissionError::BudgetExceeded(estimate)),
            None => Err(MetadataAdmissionError::Json(error)),
        };
    }
    if let Err(error) = deserializer.end() {
        return Err(MetadataAdmissionError::Json(error));
    }
    Ok(budget.used)
}

struct Budget {
    used: u64,
    limit: u64,
    exceeded: Option<u64>,
}

impl Budget {
    fn charge<E: de::Error>(&mut self, bytes: u64) -> Result<(), E> {
        self.used = self.used.saturating_add(bytes);
        if self.used > self.limit {
            self.exceeded = Some(self.used);
            return Err(E::custom("glTF metadata memory budget exceeded"));
        }
        Ok(())
    }

    fn charge_vec_slot<E: de::Error>(&mut self, element_bytes: u64, index: u64) -> Result<(), E> {
        let slots = if index == 0 { VEC_FIRST_CAPACITY } else { 2 };
        self.charge(element_bytes.saturating_mul(slots))
    }

    fn charge_string<E: de::Error>(&mut self, decoded_bytes: usize) -> Result<(), E> {
        let decoded_bytes = byte_count(decoded_bytes);
        if decoded_bytes == 0 {
            return Ok(());
        }
        self.charge(decoded_bytes.saturating_mul(2).saturating_add(8))
    }

    fn charge_value_map_entry<E: de::Error>(
        &mut self,
        entry_count: u64,
        key_bytes: usize,
    ) -> Result<(), E> {
        self.charge_string(key_bytes)?;
        let old_nodes = btree_node_count(entry_count.saturating_sub(1));
        let new_nodes = btree_node_count(entry_count);
        self.charge(
            new_nodes
                .saturating_sub(old_nodes)
                .saturating_mul(btree_node_upper_bytes()),
        )
    }
}

#[derive(Copy, Clone)]
enum Schema {
    Object(ObjectKind),
    Sequence(SequenceKind),
    FixedArray,
    String,
    Value,
    Ignore,
}

#[derive(Copy, Clone)]
enum ObjectKind {
    Root,
    Scene,
    Node,
    Mesh,
    Primitive,
    Attributes,
    Accessor,
    BufferView,
    Buffer,
    Asset,
    Image,
    Texture,
    Sampler,
}

#[derive(Copy, Clone)]
enum SequenceKind {
    String,
    Value,
    Scene,
    Node,
    Mesh,
    Primitive,
    Accessor,
    BufferView,
    Buffer,
    Image,
    Texture,
    Sampler,
    Usize,
    F32,
}

impl SequenceKind {
    fn element_bytes(self) -> u64 {
        let bytes = match self {
            Self::String => size_of::<String>(),
            Self::Value => size_of::<serde_json::Value>(),
            Self::Scene => size_of::<json::Scene>(),
            Self::Node => size_of::<json::Node>(),
            Self::Mesh => size_of::<json::Mesh>(),
            Self::Primitive => size_of::<json::Primitive>(),
            Self::Accessor => size_of::<json::Accessor>(),
            Self::BufferView => size_of::<json::BufferView>(),
            Self::Buffer => size_of::<json::Buffer>(),
            Self::Image => size_of::<json::GltfImage>(),
            Self::Texture => size_of::<json::GltfTexture>(),
            Self::Sampler => size_of::<json::GltfSampler>(),
            Self::Usize => size_of::<usize>(),
            Self::F32 => size_of::<f32>(),
        };
        byte_count(bytes)
    }

    fn element_schema(self) -> Schema {
        match self {
            Self::String => Schema::String,
            Self::Value => Schema::Value,
            Self::Scene => Schema::Object(ObjectKind::Scene),
            Self::Node => Schema::Object(ObjectKind::Node),
            Self::Mesh => Schema::Object(ObjectKind::Mesh),
            Self::Primitive => Schema::Object(ObjectKind::Primitive),
            Self::Accessor => Schema::Object(ObjectKind::Accessor),
            Self::BufferView => Schema::Object(ObjectKind::BufferView),
            Self::Buffer => Schema::Object(ObjectKind::Buffer),
            Self::Image => Schema::Object(ObjectKind::Image),
            Self::Texture => Schema::Object(ObjectKind::Texture),
            Self::Sampler => Schema::Object(ObjectKind::Sampler),
            Self::Usize | Self::F32 => Schema::Ignore,
        }
    }
}

#[derive(Copy, Clone)]
enum Field {
    Unknown,
    ExtensionsRequired,
    Animations,
    Scenes,
    Nodes,
    Meshes,
    Materials,
    Accessors,
    BufferViews,
    Buffers,
    Images,
    Textures,
    Samplers,
    Asset,
    Children,
    Name,
    Matrix,
    Translation,
    Rotation,
    Scale,
    Primitives,
    Targets,
    Attributes,
    Sparse,
    Type,
    Version,
    Generator,
    Uri,
    MimeType,
}

#[derive(Copy, Clone)]
struct FieldKey {
    field: Field,
    byte_len: usize,
}

struct FieldKeySeed;

impl<'de> DeserializeSeed<'de> for FieldKeySeed {
    type Value = FieldKey;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_identifier(FieldKeyVisitor)
    }
}

struct FieldKeyVisitor;

impl<'de> Visitor<'de> for FieldKeyVisitor {
    type Value = FieldKey;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a glTF JSON field name")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(FieldKey {
            field: field_for_name(value),
            byte_len: value.len(),
        })
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }
}

fn field_for_name(name: &str) -> Field {
    match name {
        "extensionsRequired" => Field::ExtensionsRequired,
        "animations" => Field::Animations,
        "scenes" => Field::Scenes,
        "nodes" => Field::Nodes,
        "meshes" => Field::Meshes,
        "materials" => Field::Materials,
        "accessors" => Field::Accessors,
        "bufferViews" => Field::BufferViews,
        "buffers" => Field::Buffers,
        "images" => Field::Images,
        "textures" => Field::Textures,
        "samplers" => Field::Samplers,
        "asset" => Field::Asset,
        "children" => Field::Children,
        "name" => Field::Name,
        "matrix" => Field::Matrix,
        "translation" => Field::Translation,
        "rotation" => Field::Rotation,
        "scale" => Field::Scale,
        "primitives" => Field::Primitives,
        "targets" => Field::Targets,
        "attributes" => Field::Attributes,
        "sparse" => Field::Sparse,
        "type" => Field::Type,
        "version" => Field::Version,
        "generator" => Field::Generator,
        "uri" => Field::Uri,
        "mimeType" => Field::MimeType,
        _ => Field::Unknown,
    }
}

fn field_schema(object: ObjectKind, field: Field) -> Schema {
    match (object, field) {
        (ObjectKind::Root, Field::ExtensionsRequired) => Schema::Sequence(SequenceKind::String),
        (ObjectKind::Root, Field::Animations | Field::Materials) => {
            Schema::Sequence(SequenceKind::Value)
        }
        (ObjectKind::Root, Field::Scenes) => Schema::Sequence(SequenceKind::Scene),
        (ObjectKind::Root, Field::Nodes) => Schema::Sequence(SequenceKind::Node),
        (ObjectKind::Root, Field::Meshes) => Schema::Sequence(SequenceKind::Mesh),
        (ObjectKind::Root, Field::Accessors) => Schema::Sequence(SequenceKind::Accessor),
        (ObjectKind::Root, Field::BufferViews) => Schema::Sequence(SequenceKind::BufferView),
        (ObjectKind::Root, Field::Buffers) => Schema::Sequence(SequenceKind::Buffer),
        (ObjectKind::Root, Field::Images) => Schema::Sequence(SequenceKind::Image),
        (ObjectKind::Root, Field::Textures) => Schema::Sequence(SequenceKind::Texture),
        (ObjectKind::Root, Field::Samplers) => Schema::Sequence(SequenceKind::Sampler),
        (ObjectKind::Root, Field::Asset) => Schema::Object(ObjectKind::Asset),
        (ObjectKind::Scene, Field::Nodes) => Schema::Sequence(SequenceKind::Usize),
        (ObjectKind::Node, Field::Children) => Schema::Sequence(SequenceKind::Usize),
        (ObjectKind::Node, Field::Name) | (ObjectKind::Mesh, Field::Name) => Schema::String,
        (ObjectKind::Node, Field::Matrix) => Schema::Sequence(SequenceKind::F32),
        (ObjectKind::Node, Field::Translation | Field::Rotation | Field::Scale) => {
            Schema::FixedArray
        }
        (ObjectKind::Mesh, Field::Primitives) => Schema::Sequence(SequenceKind::Primitive),
        (ObjectKind::Primitive, Field::Targets) => Schema::Sequence(SequenceKind::Value),
        (ObjectKind::Primitive, Field::Attributes) => Schema::Object(ObjectKind::Attributes),
        (ObjectKind::Accessor, Field::Sparse) => Schema::Value,
        (ObjectKind::Accessor, Field::Type) | (ObjectKind::Asset, Field::Version) => {
            Schema::String
        }
        (ObjectKind::Asset, Field::Generator) => Schema::String,
        (ObjectKind::Buffer, Field::Uri) | (ObjectKind::Image, Field::Uri) => Schema::String,
        (ObjectKind::Image, Field::MimeType) => Schema::String,
        _ => Schema::Ignore,
    }
}

struct VectorSlot {
    element_bytes: u64,
    index: u64,
}

struct ScanSeed<'a> {
    budget: &'a mut Budget,
    schema: Schema,
    vector_slot: Option<VectorSlot>,
}

impl<'de> DeserializeSeed<'de> for ScanSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        let Self {
            budget,
            schema,
            vector_slot,
        } = self;
        if let Some(slot) = vector_slot {
            budget.charge_vec_slot::<D::Error>(slot.element_bytes, slot.index)?;
        }
        if let Schema::Ignore = schema {
            return IgnoredAny::deserialize(deserializer).map(drop);
        }
        deserializer.deserialize_any(ScanVisitor { budget, schema })
    }
}

struct ScanVisitor<'a> {
    budget: &'a mut Budget,
    schema: Schema,
}

impl ScanVisitor<'_> {
    fn skip_sequence<'de, A>(mut sequence: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }

    fn skip_map<'de, A>(mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_key::<IgnoredAny>()?.is_some() {
            map.next_value::<IgnoredAny>()?;
        }
        Ok(())
    }
}

impl<'de> Visitor<'de> for ScanVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a glTF JSON value")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_i128<E>(self, _value: i128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_u128<E>(self, _value: u128) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if matches!(self.schema, Schema::String | Schema::Value) {
            self.budget.charge_string::<E>(value.len())?;
        }
        Ok(())
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(value)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_str(&value)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        match self.schema {
            Schema::Sequence(kind) => {
                let mut index = 0_u64;
                while sequence
                    .next_element_seed(ScanSeed {
                        budget: &mut *self.budget,
                        schema: kind.element_schema(),
                        vector_slot: Some(VectorSlot {
                            element_bytes: kind.element_bytes(),
                            index,
                        }),
                    })?
                    .is_some()
                {
                    index = index.saturating_add(1);
                }
                Ok(())
            }
            Schema::Value => {
                let kind = SequenceKind::Value;
                let mut index = 0_u64;
                while sequence
                    .next_element_seed(ScanSeed {
                        budget: &mut *self.budget,
                        schema: Schema::Value,
                        vector_slot: Some(VectorSlot {
                            element_bytes: kind.element_bytes(),
                            index,
                        }),
                    })?
                    .is_some()
                {
                    index = index.saturating_add(1);
                }
                Ok(())
            }
            Schema::Object(_) => Err(<A::Error as de::Error>::custom(
                "glTF records must use JSON objects",
            )),
            Schema::FixedArray => Self::skip_sequence(sequence),
            _ => Self::skip_sequence(sequence),
        }
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        match self.schema {
            Schema::Value => {
                let mut entries = 0_u64;
                while let Some(key) = map.next_key_seed(FieldKeySeed)? {
                    entries = entries.saturating_add(1);
                    self.budget
                        .charge_value_map_entry::<A::Error>(entries, key.byte_len)?;
                    map.next_value_seed(ScanSeed {
                        budget: &mut *self.budget,
                        schema: Schema::Value,
                        vector_slot: None,
                    })?;
                }
                Ok(())
            }
            Schema::Object(object) => {
                while let Some(key) = map.next_key_seed(FieldKeySeed)? {
                    let schema = field_schema(object, key.field);
                    map.next_value_seed(ScanSeed {
                        budget: &mut *self.budget,
                        schema,
                        vector_slot: None,
                    })?;
                }
                Ok(())
            }
            _ => Self::skip_map(map),
        }
    }
}

fn btree_node_count(entries: u64) -> u64 {
    if entries == 0 {
        0
    } else {
        1_u64.saturating_add((entries - 1) / BTREE_MIN_NON_ROOT_KEYS)
    }
}

fn btree_node_upper_bytes() -> u64 {
    let key_value_slots = BTREE_MAX_KEYS.saturating_mul(byte_count(
        size_of::<String>().saturating_add(size_of::<serde_json::Value>()),
    ));
    let child_slots = BTREE_MAX_CHILDREN.saturating_mul(byte_count(size_of::<usize>()));
    let header_slack = 4_u64.saturating_mul(byte_count(size_of::<usize>()));
    key_value_slots
        .saturating_add(child_slots)
        .saturating_add(header_slack)
}

fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}
