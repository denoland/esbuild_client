use std::collections::HashMap;

use indexmap::IndexMap;

use super::{
    AnyRequest, AnyResponse, AnyValue, BuildRequest, ImportKind, MangleCacheEntry, Packet,
    ProtocolMessage, ProtocolPacket,
};

#[macro_export]
macro_rules! count_idents {
  () => {0};
  ($last_ident:ident, $($idents:ident),* $(,)?) => {
      {
          #[allow(dead_code, non_camel_case_types)]
          enum Idents { $($idents,)* $last_ident }
          const COUNT: u32 = Idents::$last_ident as u32 + 1;
          COUNT
      }
  };
  ($last_ident: ident) => {
    1
  };
}

pub fn snake_to_camel(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize = false;

    for c in s.chars() {
        if c == '_' {
            capitalize = true;
        } else {
            if capitalize {
                result.push(c.to_ascii_uppercase());
                capitalize = false;
            } else {
                result.push(c);
            }
        }
    }
    result
}

#[macro_export]
macro_rules! delegate {
  ($buf: ident, $self: ident; $($field: ident),*) => {
    $(
        paste::paste! {
            if $self.$field.should_encode() {
                encode_key($buf, &$crate::protocol::encode::snake_to_camel(stringify!($field)));
                $self.$field.encode_into($buf);
            }
        }
    )*
  };
}

#[macro_export]
macro_rules! impl_encode_command {
  (for $name: ident { const Command = $command_name: literal;
    $($field: ident),*
  }) => {
    impl Encode for $name {
      fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(6); // discrim
        let dont_count = {
            $(!self.$field.should_encode() as u32 +)* 0
        };
        encode_u32_raw(buf, 1 + $crate::count_idents!($($field),*) - dont_count); // num fields
        encode_key(buf, "command");
        $command_name.encode_into(buf);
        $crate::delegate!(buf, self; $($field),*);
      }
    }
  };
}

#[macro_export]
macro_rules! impl_encode_struct {
  (for $name: ty {
    $($field: ident),*
  }) => {
    impl Encode for $name {
      fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(6); // discrim
        let dont_count = {
            $(!self.$field.should_encode() as u32 +)* 0
        };
        encode_u32_raw(buf, $crate::count_idents!($($field),*) - dont_count); // num fields
        $crate::delegate!(buf, self; $($field),*);
      }
    }
  };
}

pub trait Encode {
    fn encode_into(&self, buf: &mut Vec<u8>);

    fn should_encode(&self) -> bool {
        true
    }
}

pub fn encode_length_delimited(buf: &mut Vec<u8>, value: &[u8]) {
    encode_u32_raw(buf, value.len() as u32);
    buf.extend_from_slice(value);
}

pub fn encode_key(buf: &mut Vec<u8>, key: &str) {
    encode_length_delimited(buf, key.as_bytes());
}

impl<T: Encode> Encode for super::OptionNull<T> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        self.0.encode_into(buf);
    }
    fn should_encode(&self) -> bool {
        true
    }
}

impl<T: Encode> Encode for Option<T> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        if let Some(value) = self {
            value.encode_into(buf);
        } else {
            buf.push(0);
        }
    }

    fn should_encode(&self) -> bool {
        self.is_some()
    }
}

impl Encode for bool {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(1);
        buf.push(*self as u8);
    }
}

impl Encode for u32 {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(2);
        encode_u32_raw(buf, *self);
    }
}

impl Encode for str {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(3);
        encode_length_delimited(buf, self.as_bytes());
    }
}
impl Encode for String {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        self.as_str().encode_into(buf);
    }
}

impl Encode for [u8] {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(4);
        encode_length_delimited(buf, self);
    }
}

impl Encode for Vec<u8> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(4);
        encode_length_delimited(buf, self);
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(5);
        encode_u32_raw(buf, self.len() as u32);
        for item in self {
            item.encode_into(buf);
        }
    }
}

impl<T: Encode> Encode for (T, T) {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(5);
        encode_u32_raw(buf, 2);
        self.0.encode_into(buf);
        self.1.encode_into(buf);
    }
}

impl<V: Encode> Encode for HashMap<String, V> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(6);
        encode_u32_raw(buf, self.len() as u32);
        for (key, value) in self {
            encode_key(buf, key);
            value.encode_into(buf);
        }
    }
}

impl<V: Encode> Encode for IndexMap<String, V> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        buf.push(6);
        encode_u32_raw(buf, self.len() as u32);
        for (key, value) in self {
            encode_key(buf, key);
            value.encode_into(buf);
        }
    }
}

pub fn encode_u32_raw(buf: &mut Vec<u8>, value: u32) {
    buf.extend(value.to_le_bytes());
}

// Implementations moved from protocol.rs
//
//

impl Encode for ProtocolMessage {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ProtocolMessage::Request(req) => req.encode_into(buf),
            ProtocolMessage::Response(res) => res.encode_into(buf),
        }
    }
}

impl Encode for AnyRequest {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            AnyRequest::Build(build) => build.encode_into(buf),
            // AnyRequest::Import(import) => import.encode_into(buf),
        }
    }
}

impl Encode for AnyResponse {
    #[allow(unused)]
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            AnyResponse::Build(build_response) => todo!(),
            AnyResponse::Serve(serve_response) => todo!(),
            AnyResponse::OnEnd(on_end_response) => todo!(),
            AnyResponse::Rebuild(rebuild_response) => todo!(),
            AnyResponse::Transform(transform_response) => todo!(),
            AnyResponse::FormatMsgs(format_msgs_response) => todo!(),
            AnyResponse::AnalyzeMetafile(analyze_metafile_response) => todo!(),
            AnyResponse::OnStart(on_start_response) => on_start_response.encode_into(buf),
            AnyResponse::Resolve(resolve_response) => todo!(),
            AnyResponse::OnResolve(on_resolve_response) => on_resolve_response.encode_into(buf),
            AnyResponse::OnLoad(on_load_response) => todo!(),
        }
    }
}

impl Encode for ProtocolPacket {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        let idx = buf.len();
        buf.extend(std::iter::repeat(0).take(4));
        // eprintln!("tag: {}", (self.id << 1) | !self.is_request as u32);
        // eprintln!("len: {}", buf.len());
        encode_u32_raw(buf, (self.id << 1) | !self.is_request as u32);
        // eprintln!("encoding packet: {buf:?}");
        self.value.encode_into(buf);
        let end: u32 = buf.len() as u32;
        let len: u32 = end - (idx as u32) - 4;
        // eprintln!("len: {len}");
        buf[idx..idx + 4].copy_from_slice(&len.to_le_bytes());
    }
}

impl<T: Encode> Encode for Packet<T> {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        let idx = buf.len();
        buf.extend(std::iter::repeat(0).take(4));
        // eprintln!("tag: {}", (self.id << 1) | !self.is_request as u32);
        // eprintln!("len: {}", buf.len());
        encode_u32_raw(buf, (self.id << 1) | !self.is_request as u32);
        // eprintln!("encoding packet: {buf:?}");
        self.value.encode_into(buf);
        let end: u32 = buf.len() as u32;
        let len: u32 = end - (idx as u32) - 4;
        // eprintln!("len: {len}");
        buf[idx..idx + 4].copy_from_slice(&len.to_le_bytes());
    }
}

impl_encode_command!(for BuildRequest {
  const Command = "build";
  key, entries, flags, write, stdin_contents, stdin_resolve_dir, abs_working_dir, node_paths, context, plugins, mangle_cache
});

impl Encode for ImportKind {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            ImportKind::EntryPoint => "entry_point".encode_into(buf),
            ImportKind::ImportStatement => "import_statement".encode_into(buf),
            ImportKind::RequireCall => "require_call".encode_into(buf),
            ImportKind::DynamicImport => "dynamic_import".encode_into(buf),
            ImportKind::RequireResolve => "require_resolve".encode_into(buf),
            ImportKind::ImportRule => "import_rule".encode_into(buf),
            ImportKind::ComposesFrom => "composes_from".encode_into(buf),
            ImportKind::UrlToken => "url_token".encode_into(buf),
        }
    }
}

impl Encode for MangleCacheEntry {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            MangleCacheEntry::StringValue(s) => {
                s.encode_into(buf);
            }
            MangleCacheEntry::BoolValue(b) => {
                b.encode_into(buf);
            }
        }
    }
}

impl Encode for AnyValue {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        match self {
            AnyValue::Null => {
                buf.push(0);
            }
            AnyValue::Bool(b) => b.encode_into(buf),
            AnyValue::U32(n) => n.encode_into(buf),
            AnyValue::String(s) => s.encode_into(buf),
            AnyValue::Bytes(items) => items.encode_into(buf),
            AnyValue::Vec(any_values) => any_values.encode_into(buf),
            AnyValue::Map(hash_map) => hash_map.encode_into(buf),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn encode_protocol_packet() {
        let packet = ProtocolPacket {
            id: 0,
            is_request: false,
            value: ProtocolMessage::Response(AnyResponse::OnStart(OnStartResponse {
                errors: vec![],
                warnings: vec![],
            })),
        };
        let mut buf = Vec::new();
        packet.encode_into(&mut buf);
        assert_eq!(
            buf,
            vec![
                41, 0, 0, 0, 1, 0, 0, 0, 6, 2, 0, 0, 0, 6, 0, 0, 0, 101, 114, 114, 111, 114, 115,
                5, 0, 0, 0, 0, 8, 0, 0, 0, 119, 97, 114, 110, 105, 110, 103, 115, 5, 0, 0, 0, 0
            ]
        );
    }
}
