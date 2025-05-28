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
        } else if capitalize {
            result.push(c.to_ascii_uppercase());
            capitalize = false;
        } else {
            result.push(c);
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
            AnyResponse::OnLoad(on_load_response) => on_load_response.encode_into(buf),
        }
    }
}

impl Encode for ProtocolPacket {
    fn encode_into(&self, buf: &mut Vec<u8>) {
        let idx = buf.len();
        buf.extend(std::iter::repeat_n(0, 4));
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
        buf.extend(std::iter::repeat_n(0, 4));
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
    #[test]
    fn test_decode_packet_build() {
        let input = Packet {
            id: 0,
            is_request: true,
            value: BuildRequest {
                key: 0,
                entries: vec![("".to_string(), "./testing.ts".to_string())],
                flags: vec![
                    "--color=true".to_string(),
                    "--log-level=warning".to_string(),
                    "--log-limit=0".to_string(),
                    "--format=esm".to_string(),
                    "--platform=node".to_string(),
                    "--tree-shaking=true".to_string(),
                    "--bundle".to_string(),
                    "--outfile=./temp/mod.js".to_string(),
                    "--packages=bundle".to_string(),
                ],
                write: true,
                stdin_contents: OptionNull::new(None),
                stdin_resolve_dir: OptionNull::new(None),
                abs_working_dir: "/Users/nathanwhit/Documents/Code/esbuild-at-home".to_string(),
                node_paths: vec![],
                context: false,
                plugins: Some(vec![
                    BuildPlugin {
                        name: "deno".to_string(),
                        on_start: false,
                        on_end: false,
                        on_resolve: vec![OnResolveSetupOptions {
                            id: 0,
                            filter: ".*".to_string(),
                            namespace: "deno".to_string(),
                        }],
                        on_load: vec![OnLoadSetupOptions {
                            id: 0,
                            filter: ".*".to_string(),
                            namespace: "deno".to_string(),
                        }],
                    },
                    BuildPlugin {
                        name: "test".to_string(),
                        on_start: false,
                        on_end: false,
                        on_resolve: vec![OnResolveSetupOptions {
                            id: 1,
                            filter: ".*$".to_string(),
                            namespace: "".to_string(),
                        }],
                        on_load: vec![OnLoadSetupOptions {
                            id: 2,
                            filter: ".*$".to_string(),
                            namespace: "".to_string(),
                        }],
                    },
                ]),
                mangle_cache: None,
            },
        };
        let mut buf = Vec::new();
        input.encode_into(&mut buf);
        eprintln!("buf: {:?}", buf);
        assert_eq!(
            buf,
            vec![
                52, 3, 0, 0, 0, 0, 0, 0, 6, 11, 0, 0, 0, 7, 0, 0, 0, 99, 111, 109, 109, 97, 110,
                100, 3, 5, 0, 0, 0, 98, 117, 105, 108, 100, 3, 0, 0, 0, 107, 101, 121, 2, 0, 0, 0,
                0, 7, 0, 0, 0, 101, 110, 116, 114, 105, 101, 115, 5, 1, 0, 0, 0, 5, 2, 0, 0, 0, 3,
                0, 0, 0, 0, 3, 12, 0, 0, 0, 46, 47, 116, 101, 115, 116, 105, 110, 103, 46, 116,
                115, 5, 0, 0, 0, 102, 108, 97, 103, 115, 5, 9, 0, 0, 0, 3, 12, 0, 0, 0, 45, 45, 99,
                111, 108, 111, 114, 61, 116, 114, 117, 101, 3, 19, 0, 0, 0, 45, 45, 108, 111, 103,
                45, 108, 101, 118, 101, 108, 61, 119, 97, 114, 110, 105, 110, 103, 3, 13, 0, 0, 0,
                45, 45, 108, 111, 103, 45, 108, 105, 109, 105, 116, 61, 48, 3, 12, 0, 0, 0, 45, 45,
                102, 111, 114, 109, 97, 116, 61, 101, 115, 109, 3, 15, 0, 0, 0, 45, 45, 112, 108,
                97, 116, 102, 111, 114, 109, 61, 110, 111, 100, 101, 3, 19, 0, 0, 0, 45, 45, 116,
                114, 101, 101, 45, 115, 104, 97, 107, 105, 110, 103, 61, 116, 114, 117, 101, 3, 8,
                0, 0, 0, 45, 45, 98, 117, 110, 100, 108, 101, 3, 23, 0, 0, 0, 45, 45, 111, 117,
                116, 102, 105, 108, 101, 61, 46, 47, 116, 101, 109, 112, 47, 109, 111, 100, 46,
                106, 115, 3, 17, 0, 0, 0, 45, 45, 112, 97, 99, 107, 97, 103, 101, 115, 61, 98, 117,
                110, 100, 108, 101, 5, 0, 0, 0, 119, 114, 105, 116, 101, 1, 1, 13, 0, 0, 0, 115,
                116, 100, 105, 110, 67, 111, 110, 116, 101, 110, 116, 115, 0, 15, 0, 0, 0, 115,
                116, 100, 105, 110, 82, 101, 115, 111, 108, 118, 101, 68, 105, 114, 0, 13, 0, 0, 0,
                97, 98, 115, 87, 111, 114, 107, 105, 110, 103, 68, 105, 114, 3, 48, 0, 0, 0, 47,
                85, 115, 101, 114, 115, 47, 110, 97, 116, 104, 97, 110, 119, 104, 105, 116, 47, 68,
                111, 99, 117, 109, 101, 110, 116, 115, 47, 67, 111, 100, 101, 47, 101, 115, 98,
                117, 105, 108, 100, 45, 97, 116, 45, 104, 111, 109, 101, 9, 0, 0, 0, 110, 111, 100,
                101, 80, 97, 116, 104, 115, 5, 0, 0, 0, 0, 7, 0, 0, 0, 99, 111, 110, 116, 101, 120,
                116, 1, 0, 7, 0, 0, 0, 112, 108, 117, 103, 105, 110, 115, 5, 2, 0, 0, 0, 6, 5, 0,
                0, 0, 4, 0, 0, 0, 110, 97, 109, 101, 3, 4, 0, 0, 0, 100, 101, 110, 111, 7, 0, 0, 0,
                111, 110, 83, 116, 97, 114, 116, 1, 0, 5, 0, 0, 0, 111, 110, 69, 110, 100, 1, 0, 9,
                0, 0, 0, 111, 110, 82, 101, 115, 111, 108, 118, 101, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0,
                2, 0, 0, 0, 105, 100, 2, 0, 0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3,
                2, 0, 0, 0, 46, 42, 9, 0, 0, 0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 4, 0,
                0, 0, 100, 101, 110, 111, 6, 0, 0, 0, 111, 110, 76, 111, 97, 100, 5, 1, 0, 0, 0, 6,
                3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 0, 0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116,
                101, 114, 3, 2, 0, 0, 0, 46, 42, 9, 0, 0, 0, 110, 97, 109, 101, 115, 112, 97, 99,
                101, 3, 4, 0, 0, 0, 100, 101, 110, 111, 6, 5, 0, 0, 0, 4, 0, 0, 0, 110, 97, 109,
                101, 3, 4, 0, 0, 0, 116, 101, 115, 116, 7, 0, 0, 0, 111, 110, 83, 116, 97, 114,
                116, 1, 0, 5, 0, 0, 0, 111, 110, 69, 110, 100, 1, 0, 9, 0, 0, 0, 111, 110, 82, 101,
                115, 111, 108, 118, 101, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 1,
                0, 0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3, 3, 0, 0, 0, 46, 42, 36, 9, 0,
                0, 0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 0, 0, 0, 0, 6, 0, 0, 0, 111,
                110, 76, 111, 97, 100, 5, 1, 0, 0, 0, 6, 3, 0, 0, 0, 2, 0, 0, 0, 105, 100, 2, 2, 0,
                0, 0, 6, 0, 0, 0, 102, 105, 108, 116, 101, 114, 3, 3, 0, 0, 0, 46, 42, 36, 9, 0, 0,
                0, 110, 97, 109, 101, 115, 112, 97, 99, 101, 3, 0, 0, 0, 0
            ]
        );
    }

    #[test]
    fn test_encode_packet() {
        let v = vec![
            ("a".into(), AnyValue::String("b".into())),
            ("c".into(), AnyValue::String("d".into())),
            ("e".into(), AnyValue::Bytes(vec![1u8, 2, 3, 4])),
        ]
        .into_iter()
        .collect::<IndexMap<String, AnyValue>>();
        let mut buf = Vec::new();
        let packet = Packet {
            id: 1,
            is_request: true,
            value: AnyValue::Map(v),
        };
        packet.encode_into(&mut buf);

        eprintln!("buf: {:?}", buf);
        // assert_eq!(
        //     buf,
        //     vec![
        //         45, 0, 0, 0, 0, 0, 0, 0, 6, 3, 0, 0, 0, 1, 0, 0, 0, 97, 3, 1, 0, 0, 0, 98, 1, 0, 0, 0,
        //         99, 3, 1, 0, 0, 0, 100, 1, 0, 0, 0, 101, 4, 4, 0, 0, 0, 1, 2, 3, 4
        //     ]
        // );
        assert_eq!(
            buf,
            vec![
                45, 0, 0, 0, 2, 0, 0, 0, 6, 3, 0, 0, 0, 1, 0, 0, 0, 97, 3, 1, 0, 0, 0, 98, 1, 0, 0,
                0, 99, 3, 1, 0, 0, 0, 100, 1, 0, 0, 0, 101, 4, 4, 0, 0, 0, 1, 2, 3, 4
            ]
        );
    }
}
