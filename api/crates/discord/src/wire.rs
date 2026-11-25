//! How Discord's JSON is read: snake_case names, unknown fields ignored,
//! absent fields left empty, and every string field willing to take a number.
//!
//! Discord sends snowflakes and permission sets as strings, but a template's
//! placeholder ids are integers, and the documentation's own example gives
//! permissions as a number. Rather than know which field does what, any text
//! field reads a number as its digits. Timestamps stay text too: the
//! archived-thread and pin pages are asked for with the timestamp Discord
//! gave, exactly as it gave it, and one odd date must not fail a whole page.
//!
//! A number field reads a number, or a string of its digits. A value of the
//! wrong kind (an object where a number is expected, a list where an object
//! is) fails the answer, since the rest of it cannot be trusted; an object or
//! a list where text is expected only reads as absent. Null is absent
//! everywhere.
//!
//! The shapes hold only the fields the import reads.

use std::fmt;
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

/// Text that may be sent as a number or a boolean; anything else reads as
/// absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Text(pub Option<String>);

impl Text {
    pub fn get(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub fn owned(&self) -> Option<String> {
        self.0.clone()
    }
}

struct TextVisitor;

impl<'de> Visitor<'de> for TextVisitor {
    type Value = Text;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("text, a number or a boolean")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Text, E> {
        Ok(Text(Some(value.to_owned())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Text, E> {
        Ok(Text(Some(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Text, E> {
        Ok(Text(Some(value.to_string())))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Text, E> {
        Ok(Text(Some(value.to_string())))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Text, E> {
        Ok(Text(Some(value.to_string())))
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Text, E> {
        Ok(Text(Some(value.to_string())))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Text, E> {
        Ok(Text(None))
    }

    fn visit_none<E: de::Error>(self) -> Result<Text, E> {
        Ok(Text(None))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Text, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Text(None))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Text, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Text(None))
    }
}

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Text, D::Error> {
        deserializer.deserialize_any(TextVisitor)
    }
}

/// A whole number that may be sent as a string of its digits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Int<T>(pub Option<T>);

struct IntVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for IntVisitor<T>
where
    T: TryFrom<i64> + std::str::FromStr,
{
    type Value = Int<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a whole number")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Int<T>, E> {
        T::try_from(value)
            .map(|number| Int(Some(number)))
            .map_err(|_| E::custom("number out of range"))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Int<T>, E> {
        i64::try_from(value)
            .map_err(|_| E::custom("number out of range"))
            .and_then(|value| self.visit_i64(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Int<T>, E> {
        let digits = value.strip_prefix('-').unwrap_or(value);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(E::custom("not a whole number"));
        }
        value
            .parse::<T>()
            .map(|number| Int(Some(number)))
            .map_err(|_| E::custom("number out of range"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Int<T>, E> {
        Ok(Int(None))
    }

    fn visit_none<E: de::Error>(self) -> Result<Int<T>, E> {
        Ok(Int(None))
    }
}

impl<'de, T> Deserialize<'de> for Int<T>
where
    T: TryFrom<i64> + std::str::FromStr,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Int<T>, D::Error> {
        deserializer.deserialize_any(IntVisitor(PhantomData))
    }
}

/// A number with a fraction that may be sent as a string.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Float(pub Option<f64>);

struct FloatVisitor;

impl<'de> Visitor<'de> for FloatVisitor {
    type Value = Float;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a number")
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Float, E> {
        Ok(Float(Some(value as f64)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Float, E> {
        Ok(Float(Some(value as f64)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Float, E> {
        Ok(Float(Some(value)))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Float, E> {
        match value.parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(Float(Some(number))),
            _ => Err(E::custom("not a number")),
        }
    }

    fn visit_unit<E: de::Error>(self) -> Result<Float, E> {
        Ok(Float(None))
    }

    fn visit_none<E: de::Error>(self) -> Result<Float, E> {
        Ok(Float(None))
    }
}

impl<'de> Deserialize<'de> for Float {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Float, D::Error> {
        deserializer.deserialize_any(FloatVisitor)
    }
}

/// A boolean, which is only ever sent as one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Flag(pub Option<bool>);

struct FlagVisitor;

impl<'de> Visitor<'de> for FlagVisitor {
    type Value = Flag;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("true or false")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Flag, E> {
        Ok(Flag(Some(value)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Flag, E> {
        Ok(Flag(None))
    }

    fn visit_none<E: de::Error>(self) -> Result<Flag, E> {
        Ok(Flag(None))
    }
}

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Flag, D::Error> {
        deserializer.deserialize_any(FlagVisitor)
    }
}

/// A deserializer that reads a struct only from a JSON object. The derived
/// code for a struct also accepts a list, field after field, which is not
/// what Discord sends and not what to trust.
struct ObjectsOnly<D>(D);

macro_rules! delegate {
    ($($method:ident),* $(,)?) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, D::Error> {
                self.0.$method(visitor)
            }
        )*
    };
}

impl<'de, D: Deserializer<'de>> Deserializer<'de> for ObjectsOnly<D> {
    type Error = D::Error;

    delegate!(
        deserialize_any,
        deserialize_bool,
        deserialize_i8,
        deserialize_i16,
        deserialize_i32,
        deserialize_i64,
        deserialize_i128,
        deserialize_u8,
        deserialize_u16,
        deserialize_u32,
        deserialize_u64,
        deserialize_u128,
        deserialize_f32,
        deserialize_f64,
        deserialize_char,
        deserialize_str,
        deserialize_string,
        deserialize_bytes,
        deserialize_byte_buf,
        deserialize_option,
        deserialize_unit,
        deserialize_seq,
        deserialize_map,
        deserialize_identifier,
        deserialize_ignored_any,
    );

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_unit_struct(name, visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_newtype_struct(name, visitor)
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple(len, visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_tuple_struct(name, len, visitor)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, D::Error> {
        self.0.deserialize_enum(name, variants, visitor)
    }

    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

/// An object, or null for none. Any other value fails the answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Obj<T>(pub Option<T>);

impl<T> Default for Obj<T> {
    fn default() -> Obj<T> {
        Obj(None)
    }
}

struct ObjVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjVisitor<T> {
    type Value = Obj<T>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object or null")
    }

    fn visit_none<E: de::Error>(self) -> Result<Obj<T>, E> {
        Ok(Obj(None))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Obj<T>, E> {
        Ok(Obj(None))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Obj<T>, D::Error> {
        T::deserialize(ObjectsOnly(deserializer)).map(|value| Obj(Some(value)))
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Obj<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Obj<T>, D::Error> {
        deserializer.deserialize_option(ObjVisitor(PhantomData))
    }
}

/// A list of objects, or null for none. Its items may be null. Any other
/// value fails the answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct List<T>(pub Vec<Option<T>>);

impl<T> Default for List<T> {
    fn default() -> List<T> {
        List(Vec::new())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for List<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<List<T>, D::Error> {
        Option::<Vec<Obj<T>>>::deserialize(deserializer).map(|list| {
            List(
                list.unwrap_or_default()
                    .into_iter()
                    .map(|item| item.0)
                    .collect(),
            )
        })
    }
}

impl<T> List<T> {
    /// The items that are not null.
    pub fn present(&self) -> impl Iterator<Item = &T> {
        self.0.iter().flatten()
    }
}

/// A list of text, or null for none. Items that are not text read as absent.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TextList(pub Vec<Text>);

impl<'de> Deserialize<'de> for TextList {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<TextList, D::Error> {
        Option::<Vec<Text>>::deserialize(deserializer)
            .map(|list| TextList(list.unwrap_or_default()))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireUser {
    pub id: Text,
    pub username: Text,
    pub global_name: Text,
    pub avatar: Text,
    pub bot: Flag,
    pub member: Obj<WireMentionMember>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireMentionMember {
    pub nick: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireApplication {
    pub id: Text,
    pub name: Text,
    pub flags: Int<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireGuild {
    pub id: Text,
    pub name: Text,
    pub description: Text,
    pub icon: Text,
    /// The icon in a template's copy of the server, which has no `icon`.
    pub icon_hash: Text,
    pub owner_id: Text,
    pub approximate_member_count: Int<i32>,
    pub approximate_presence_count: Int<i32>,
    pub roles: List<WireRole>,
    pub channels: List<WireChannel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireTemplate {
    pub code: Text,
    pub name: Text,
    pub source_guild_id: Text,
    pub serialized_source_guild: Obj<WireGuild>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireRole {
    pub id: Text,
    pub name: Text,
    pub color: Int<i32>,
    pub colors: Obj<WireRoleColors>,
    pub hoist: Flag,
    pub position: Int<i32>,
    pub permissions: Text,
    pub managed: Flag,
    pub mentionable: Flag,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireRoleColors {
    pub primary_color: Int<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireChannel {
    pub id: Text,
    #[serde(rename = "type")]
    pub kind: Int<i32>,
    pub position: Int<i32>,
    pub permission_overwrites: List<WireOverwrite>,
    pub name: Text,
    pub topic: Text,
    pub rate_limit_per_user: Int<i32>,
    pub user_limit: Int<i32>,
    pub parent_id: Text,
    pub thread_metadata: Obj<WireThreadMetadata>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireOverwrite {
    pub id: Text,
    #[serde(rename = "type")]
    pub kind: Int<i32>,
    pub allow: Text,
    pub deny: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireThreadMetadata {
    pub archived: Flag,
    pub locked: Flag,
    pub archive_timestamp: Text,
    pub create_timestamp: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireThreadList {
    pub threads: List<WireChannel>,
    pub has_more: Flag,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireMember {
    pub user: Obj<WireUser>,
    pub nick: Text,
    pub avatar: Text,
    pub roles: TextList,
    pub joined_at: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireEmoji {
    pub id: Text,
    pub name: Text,
    pub animated: Flag,
    pub available: Flag,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireSticker {
    pub id: Text,
    pub name: Text,
    pub description: Text,
    pub format_type: Int<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireMessage {
    pub id: Text,
    pub channel_id: Text,
    pub author: Obj<WireUser>,
    pub content: Text,
    pub mentions: List<WireUser>,
    pub timestamp: Text,
    pub edited_timestamp: Text,
    pub pinned: Flag,
    pub webhook_id: Text,
    #[serde(rename = "type")]
    pub kind: Int<i32>,
    pub flags: Int<i64>,
    pub message_reference: Obj<WireMessageReference>,
    pub message_snapshots: List<WireMessageSnapshot>,
    pub attachments: List<WireAttachment>,
    pub embeds: List<WireEmbed>,
    pub reactions: List<WireReaction>,
    pub sticker_items: List<WireSticker>,
    pub thread: Obj<WireChannel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireMessageReference {
    #[serde(rename = "type")]
    pub kind: Int<i32>,
    pub message_id: Text,
    pub channel_id: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireMessageSnapshot {
    pub message: Obj<WireMessage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireAttachment {
    pub id: Text,
    pub filename: Text,
    pub content_type: Text,
    pub size: Int<i64>,
    pub url: Text,
    pub width: Int<i32>,
    pub height: Int<i32>,
    pub duration_secs: Float,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireEmbed {
    #[serde(rename = "type")]
    pub kind: Text,
    pub title: Text,
    pub description: Text,
    pub url: Text,
    pub image: Obj<WireEmbedMedia>,
    pub thumbnail: Obj<WireEmbedMedia>,
    pub video: Obj<WireEmbedMedia>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireEmbedMedia {
    pub url: Text,
    pub proxy_url: Text,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireReaction {
    pub count: Int<i32>,
    pub count_details: Obj<WireReactionCounts>,
    pub emoji: Obj<WireEmoji>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireReactionCounts {
    pub burst: Int<i32>,
    pub normal: Int<i32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WirePin {
    pub pinned_at: Text,
    pub message: Obj<WireMessage>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WirePinList {
    pub items: List<WirePin>,
    pub has_more: Flag,
}

/// Discord's error body, which a 429 extends with how long to wait.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct WireError {
    pub message: Text,
    pub code: Int<i32>,
    pub retry_after: Float,
    pub global: Flag,
}

/// Whether the JSON nests deeper than `limit` arrays and objects. Discord's
/// answers are a few levels deep; anything past this is not one of them.
pub(crate) fn nests_deeper_than(bytes: &[u8], limit: usize) -> bool {
    let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limit {
                    return true;
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}
