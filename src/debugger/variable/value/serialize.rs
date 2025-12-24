use crate::debugger::debugee::dwarf::r#type::{
    ArrayType, ComplexType, MemberLocation, StructureMember, TypeDeclaration, TypeId,
};
use crate::debugger::variable::value::Value;
use indexmap::IndexMap;
use serde_json::Value as JsonValue;

#[derive(Debug, thiserror::Error)]
pub enum SerializeError {
    #[error("unsupported input: {0}")]
    UnsupportedInput(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("unexpected input for type: {0}")]
    UnexpectedInput(String),
    #[error("unknown type id")]
    UnknownType,
    #[error("unknown type size for type id {0:?}")]
    UnknownTypeSize(TypeId),
    #[error("enum variant '{0}' not found")]
    EnumVariantNotFound(String),
    #[error("enum discriminant type missing")]
    EnumDiscriminantMissing,
    #[error("member offset for '{0}' not available")]
    MemberOffsetMissing(String),
    #[error("array element type missing")]
    ArrayElementTypeMissing,
    #[error("array length mismatch: expected {expected}, got {actual}")]
    ArrayLengthMismatch { expected: usize, actual: usize },
    #[error("value size mismatch: expected {expected}, got {actual}")]
    SizeMismatch { expected: usize, actual: usize },
    #[error("scalar parse failed: {0}")]
    ScalarParse(String),
}

#[derive(Clone, Debug)]
pub struct WriteOffset {
    pub path: String,
    pub offset: usize,
    pub size: usize,
}

#[derive(Clone, Debug)]
pub struct SerializedValue {
    pub bytes: Vec<u8>,
    pub size: usize,
    pub offsets: Vec<WriteOffset>,
}

#[derive(Clone, Debug)]
enum InputValue {
    Scalar(String),
    Array(Vec<InputValue>),
    Object(IndexMap<String, InputValue>),
    EnumVariant {
        name: String,
        payload: Option<Box<InputValue>>,
    },
}

pub fn serialize_dap_value(
    input: &str,
    type_graph: &ComplexType,
    type_id: TypeId,
    runtime_value: Option<&Value>,
) -> Result<SerializedValue, SerializeError> {
    let input = parse_input_value(input)?;
    let mut offsets = Vec::new();
    let bytes =
        serialize_value_inner(&input, type_graph, type_id, runtime_value, "", &mut offsets)?;
    let size = bytes.len();
    Ok(SerializedValue {
        bytes,
        size,
        offsets,
    })
}

fn parse_input_value(input: &str) -> Result<InputValue, SerializeError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(SerializeError::UnsupportedInput("empty input".to_string()));
    }

    if let Ok(json) = serde_json::from_str::<JsonValue>(trimmed) {
        return Ok(input_from_json(&json));
    }

    if let Some(enum_value) = parse_enum_syntax(trimmed)? {
        return Ok(enum_value);
    }

    if let Some(tuple) = parse_tuple_syntax(trimmed)? {
        return Ok(tuple);
    }

    if let Some(obj) = parse_object_syntax(trimmed)? {
        return Ok(obj);
    }

    Ok(InputValue::Scalar(trimmed.to_string()))
}

fn parse_enum_syntax(input: &str) -> Result<Option<InputValue>, SerializeError> {
    let mut chars = input.chars();
    let first = chars.next();
    let Some(first) = first else {
        return Ok(None);
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return Ok(None);
    }

    let mut ident = String::new();
    ident.push(first);
    let mut rest_start = first.len_utf8();
    for ch in input[rest_start..].chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            ident.push(ch);
            rest_start += ch.len_utf8();
        } else {
            break;
        }
    }

    let rest = input[rest_start..].trim_start();
    if rest.is_empty() {
        return Ok(None);
    }

    let payload = if rest.starts_with('(') && rest.ends_with(')') {
        let inner = &rest[1..rest.len() - 1];
        Some(Box::new(parse_input_value(inner)?))
    } else if rest.starts_with('{') && rest.ends_with('}') {
        let inner = &rest[1..rest.len() - 1];
        Some(Box::new(parse_object_body(inner)?))
    } else {
        return Ok(None);
    };

    Ok(Some(InputValue::EnumVariant {
        name: ident,
        payload,
    }))
}

fn parse_tuple_syntax(input: &str) -> Result<Option<InputValue>, SerializeError> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        return Ok(None);
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let items = split_top_level(inner)
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| parse_input_value(part.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(InputValue::Array(items)))
}

fn parse_object_syntax(input: &str) -> Result<Option<InputValue>, SerializeError> {
    let trimmed = input.trim();
    if !(trimmed.starts_with('{') && trimmed.ends_with('}')) {
        return Ok(None);
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    Ok(Some(parse_object_body(inner)?))
}

fn parse_object_body(body: &str) -> Result<InputValue, SerializeError> {
    let mut out = IndexMap::new();
    for part in split_top_level(body) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (key, value) = split_key_value(part)?;
        let key = key.trim().trim_matches('"').trim_matches('\'');
        let value = parse_input_value(value.trim())?;
        out.insert(key.to_string(), value);
    }
    Ok(InputValue::Object(out))
}

fn split_key_value(input: &str) -> Result<(&str, &str), SerializeError> {
    let mut depth = 0usize;
    let mut in_str = None;
    for (idx, ch) in input.char_indices() {
        match ch {
            '"' | '\'' => {
                if in_str == Some(ch) {
                    in_str = None;
                } else if in_str.is_none() {
                    in_str = Some(ch);
                }
            }
            '{' | '[' | '(' if in_str.is_none() => depth += 1,
            '}' | ']' | ')' if in_str.is_none() => depth = depth.saturating_sub(1),
            ':' if in_str.is_none() && depth == 0 => {
                let (k, v) = input.split_at(idx);
                return Ok((k, &v[1..]));
            }
            _ => {}
        }
    }
    Err(SerializeError::UnsupportedInput(format!(
        "expected key:value in '{input}'"
    )))
}

fn split_top_level(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut start = 0usize;
    let mut prev_char = '\0';

    for (idx, ch) in input.char_indices() {
        if let Some(quote) = in_str {
            if ch == quote && prev_char != '\\' {
                in_str = None;
            }
            prev_char = ch;
            continue;
        }

        match ch {
            '\'' | '"' => in_str = Some(ch),
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
        prev_char = ch;
    }

    if start <= input.len() {
        parts.push(&input[start..]);
    }

    parts
}

fn input_from_json(value: &JsonValue) -> InputValue {
    match value {
        JsonValue::Null => InputValue::Scalar("null".to_string()),
        JsonValue::Bool(b) => InputValue::Scalar(b.to_string()),
        JsonValue::Number(num) => InputValue::Scalar(num.to_string()),
        JsonValue::String(s) => InputValue::Scalar(s.clone()),
        JsonValue::Array(items) => InputValue::Array(items.iter().map(input_from_json).collect()),
        JsonValue::Object(map) => InputValue::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), input_from_json(v)))
                .collect(),
        ),
    }
}

fn serialize_value_inner(
    input: &InputValue,
    type_graph: &ComplexType,
    type_id: TypeId,
    runtime_value: Option<&Value>,
    path: &str,
    offsets: &mut Vec<WriteOffset>,
) -> Result<Vec<u8>, SerializeError> {
    let decl = type_graph
        .types
        .get(&type_id)
        .ok_or(SerializeError::UnknownType)?;
    match decl {
        TypeDeclaration::Scalar(scalar) => serialize_scalar_value(input, scalar),
        TypeDeclaration::Pointer { .. } => serialize_pointer_value(input),
        TypeDeclaration::Array(array) => {
            serialize_array_value(input, type_graph, array, runtime_value, path, offsets)
        }
        TypeDeclaration::Structure {
            byte_size, members, ..
        } => serialize_struct_value(
            input,
            type_graph,
            type_id,
            *byte_size,
            members,
            runtime_value,
            path,
            offsets,
        ),
        TypeDeclaration::Union {
            byte_size, members, ..
        } => serialize_union_value(
            input,
            type_graph,
            type_id,
            *byte_size,
            members,
            runtime_value,
            path,
            offsets,
        ),
        TypeDeclaration::CStyleEnum {
            byte_size,
            discr_type,
            enumerators,
            ..
        } => serialize_c_enum_value(input, type_graph, *byte_size, *discr_type, enumerators),
        TypeDeclaration::RustEnum {
            byte_size,
            discr_type,
            enumerators,
            ..
        } => serialize_rust_enum_value(
            input,
            type_graph,
            type_id,
            *byte_size,
            discr_type.as_deref(),
            enumerators,
            runtime_value,
            path,
            offsets,
        ),
        TypeDeclaration::Subroutine { .. } => serialize_pointer_value(input),
        TypeDeclaration::ModifiedType { inner, .. } => {
            let inner = inner.ok_or(SerializeError::UnknownType)?;
            serialize_value_inner(input, type_graph, inner, runtime_value, path, offsets)
        }
    }
}

fn serialize_scalar_value(
    input: &InputValue,
    scalar: &crate::debugger::debugee::dwarf::r#type::ScalarType,
) -> Result<Vec<u8>, SerializeError> {
    let input = match input {
        InputValue::Scalar(s) => s.as_str(),
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "expected scalar".to_string(),
            ));
        }
    };
    let byte_size = scalar.byte_size.unwrap_or(0) as usize;
    let encoding = scalar.encoding;

    fn parse_i128(input: &str) -> Result<i128, SerializeError> {
        if let Some(hex) = input
            .strip_prefix("0x")
            .or_else(|| input.strip_prefix("0X"))
        {
            i128::from_str_radix(hex, 16).map_err(|e| SerializeError::ScalarParse(e.to_string()))
        } else {
            input
                .parse::<i128>()
                .map_err(|e| SerializeError::ScalarParse(e.to_string()))
        }
    }

    fn parse_u128(input: &str) -> Result<u128, SerializeError> {
        if let Some(hex) = input
            .strip_prefix("0x")
            .or_else(|| input.strip_prefix("0X"))
        {
            u128::from_str_radix(hex, 16).map_err(|e| SerializeError::ScalarParse(e.to_string()))
        } else {
            input
                .parse::<u128>()
                .map_err(|e| SerializeError::ScalarParse(e.to_string()))
        }
    }

    let bytes = match encoding {
        Some(gimli::DW_ATE_float) => match byte_size {
            4 => Ok(input
                .parse::<f32>()
                .map_err(|e| SerializeError::ScalarParse(e.to_string()))?
                .to_le_bytes()
                .to_vec()),
            8 => Ok(input
                .parse::<f64>()
                .map_err(|e| SerializeError::ScalarParse(e.to_string()))?
                .to_le_bytes()
                .to_vec()),
            _ => Err(SerializeError::ScalarParse(format!(
                "unsupported float size {byte_size}"
            ))),
        },
        Some(gimli::DW_ATE_boolean) => {
            let value = match input {
                "true" | "True" | "TRUE" | "1" => true,
                "false" | "False" | "FALSE" | "0" => false,
                _ => {
                    return Err(SerializeError::ScalarParse(format!(
                        "expected bool, got '{input}'"
                    )));
                }
            };
            Ok(vec![if value { 1 } else { 0 }])
        }
        Some(gimli::DW_ATE_UTF) | Some(gimli::DW_ATE_ASCII) => {
            let ch =
                if let Some(stripped) = input.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
                {
                    stripped.chars().next().ok_or_else(|| {
                        SerializeError::ScalarParse("empty char literal".to_string())
                    })?
                } else if input.chars().count() == 1 {
                    input.chars().next().unwrap()
                } else {
                    return Err(SerializeError::ScalarParse(
                        "expected char literal".to_string(),
                    ));
                };
            Ok((ch as u32).to_le_bytes().to_vec())
        }
        Some(gimli::DW_ATE_signed) | Some(gimli::DW_ATE_signed_char) => {
            let value = parse_i128(input)?;
            int_to_bytes(value as i128, byte_size)
        }
        Some(gimli::DW_ATE_unsigned)
        | Some(gimli::DW_ATE_unsigned_char)
        | Some(gimli::DW_ATE_address) => {
            let value = parse_u128(input)?;
            uint_to_bytes(value, byte_size)
        }
        _ => {
            let value = parse_i128(input)?;
            int_to_bytes(value, byte_size)
        }
    }?;

    Ok(bytes)
}

fn serialize_pointer_value(input: &InputValue) -> Result<Vec<u8>, SerializeError> {
    let input = match input {
        InputValue::Scalar(s) => s.as_str(),
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "expected pointer scalar".to_string(),
            ));
        }
    };
    let addr = if let Some(hex) = input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        usize::from_str_radix(hex, 16).map_err(|e| SerializeError::ScalarParse(e.to_string()))?
    } else {
        input
            .parse::<usize>()
            .map_err(|e| SerializeError::ScalarParse(e.to_string()))?
    };
    Ok(addr.to_le_bytes().to_vec())
}

fn serialize_struct_value(
    input: &InputValue,
    type_graph: &ComplexType,
    type_id: TypeId,
    byte_size: Option<u64>,
    members: &[StructureMember],
    runtime_value: Option<&Value>,
    path: &str,
    offsets: &mut Vec<WriteOffset>,
) -> Result<Vec<u8>, SerializeError> {
    let size = byte_size.ok_or(SerializeError::UnknownTypeSize(type_id))?;
    let mut buffer = vec![0u8; size as usize];

    let input_map = match input {
        InputValue::Object(map) => Some(map),
        _ => None,
    };
    let input_array = match input {
        InputValue::Array(items) => Some(items),
        _ => None,
    };

    for (index, member) in members.iter().enumerate() {
        let member_input = if let Some(name) = member.name.as_deref() {
            input_map
                .and_then(|map| map.get(name))
                .ok_or_else(|| SerializeError::MissingField(name.to_string()))?
        } else {
            input_array
                .and_then(|items| items.get(index))
                .ok_or_else(|| SerializeError::MissingField(format!("[{index}]")))?
        };

        let member_type = member.type_ref.ok_or(SerializeError::UnknownType)?;
        let member_runtime = runtime_member(runtime_value, member.name.as_deref(), index);
        let member_bytes = serialize_value_inner(
            member_input,
            type_graph,
            member_type,
            member_runtime,
            &format!("{path}{}", member_path_suffix(member, index)),
            offsets,
        )?;
        let offset = member_offset(member, runtime_value, index)
            .ok_or_else(|| SerializeError::MemberOffsetMissing(member_name(member, index)))?;
        let end = offset + member_bytes.len();
        if end > buffer.len() {
            return Err(SerializeError::SizeMismatch {
                expected: buffer.len(),
                actual: end,
            });
        }
        buffer[offset..end].copy_from_slice(&member_bytes);
        offsets.push(WriteOffset {
            path: format!("{path}{}", member_path_suffix(member, index)),
            offset,
            size: member_bytes.len(),
        });
    }

    Ok(buffer)
}

fn serialize_union_value(
    input: &InputValue,
    type_graph: &ComplexType,
    type_id: TypeId,
    byte_size: Option<u64>,
    members: &[StructureMember],
    runtime_value: Option<&Value>,
    path: &str,
    offsets: &mut Vec<WriteOffset>,
) -> Result<Vec<u8>, SerializeError> {
    let size = byte_size.ok_or(SerializeError::UnknownTypeSize(type_id))?;
    let mut buffer = vec![0u8; size as usize];
    let (member_idx, member_input) = match input {
        InputValue::Object(map) => {
            let (name, value) = map
                .iter()
                .next()
                .ok_or_else(|| SerializeError::UnsupportedInput("empty union".to_string()))?;
            let idx = members
                .iter()
                .position(|m| m.name.as_deref() == Some(name))
                .ok_or_else(|| SerializeError::MissingField(name.to_string()))?;
            (idx, value)
        }
        InputValue::Array(items) => {
            let value = items
                .first()
                .ok_or_else(|| SerializeError::UnsupportedInput("empty union".to_string()))?;
            (0, value)
        }
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "union expects object with single field".to_string(),
            ));
        }
    };
    let member = &members[member_idx];
    let member_type = member.type_ref.ok_or(SerializeError::UnknownType)?;
    let member_runtime = runtime_member(runtime_value, member.name.as_deref(), member_idx);
    let member_bytes = serialize_value_inner(
        member_input,
        type_graph,
        member_type,
        member_runtime,
        &format!("{path}{}", member_path_suffix(member, member_idx)),
        offsets,
    )?;
    let offset = member_offset(member, runtime_value, member_idx)
        .ok_or_else(|| SerializeError::MemberOffsetMissing(member_name(member, member_idx)))?;
    let end = offset + member_bytes.len();
    if end > buffer.len() {
        return Err(SerializeError::SizeMismatch {
            expected: buffer.len(),
            actual: end,
        });
    }
    buffer[offset..end].copy_from_slice(&member_bytes);
    offsets.push(WriteOffset {
        path: format!("{path}{}", member_path_suffix(member, member_idx)),
        offset,
        size: member_bytes.len(),
    });
    Ok(buffer)
}

fn serialize_array_value(
    input: &InputValue,
    type_graph: &ComplexType,
    array: &ArrayType,
    runtime_value: Option<&Value>,
    path: &str,
    offsets: &mut Vec<WriteOffset>,
) -> Result<Vec<u8>, SerializeError> {
    let items = match input {
        InputValue::Array(items) => items,
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "array expects list".to_string(),
            ));
        }
    };
    let element_type = array
        .element_type()
        .ok_or(SerializeError::ArrayElementTypeMissing)?;
    let mut buf = Vec::new();
    let mut element_size = None;
    for (idx, item) in items.iter().enumerate() {
        let runtime_item = runtime_array_item(runtime_value, idx);
        let bytes = serialize_value_inner(
            item,
            type_graph,
            element_type,
            runtime_item,
            &format!("{path}[{idx}]"),
            offsets,
        )?;
        let size = bytes.len();
        if let Some(existing) = element_size {
            if existing != size {
                return Err(SerializeError::SizeMismatch {
                    expected: existing,
                    actual: size,
                });
            }
        } else {
            element_size = Some(size);
        }
        buf.extend_from_slice(&bytes);
        offsets.push(WriteOffset {
            path: format!("{path}[{idx}]"),
            offset: idx * size,
            size,
        });
    }
    if let Some(expected_size) = array.byte_size_hint() {
        let expected_size = expected_size as usize;
        if expected_size != buf.len() {
            let element_size = element_size.unwrap_or(0);
            let expected_len = if element_size > 0 {
                expected_size / element_size
            } else {
                0
            };
            return Err(SerializeError::ArrayLengthMismatch {
                expected: expected_len,
                actual: items.len(),
            });
        }
    }
    Ok(buf)
}

fn serialize_c_enum_value(
    input: &InputValue,
    type_graph: &ComplexType,
    byte_size: Option<u64>,
    discr_type: Option<TypeId>,
    enumerators: &std::collections::HashMap<i64, String>,
) -> Result<Vec<u8>, SerializeError> {
    let name_or_value = match input {
        InputValue::Scalar(s) => s.clone(),
        InputValue::EnumVariant { name, .. } => name.clone(),
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "enum expects scalar".to_string(),
            ));
        }
    };

    let discr_value = if let Ok(num) = name_or_value.parse::<i64>() {
        num
    } else {
        let (value, _) = enumerators
            .iter()
            .find(|(_, name)| *name == &name_or_value)
            .ok_or_else(|| SerializeError::EnumVariantNotFound(name_or_value.clone()))?;
        *value
    };

    if let Some(type_id) = discr_type {
        let input = InputValue::Scalar(discr_value.to_string());
        let mut offsets = Vec::new();
        let bytes = serialize_value_inner(&input, type_graph, type_id, None, "", &mut offsets)?;
        if let Some(byte_size) = byte_size {
            let expected = byte_size as usize;
            if bytes.len() != expected {
                return Err(SerializeError::SizeMismatch {
                    expected,
                    actual: bytes.len(),
                });
            }
        }
        return Ok(bytes);
    }

    let size = byte_size.unwrap_or(0) as usize;
    if size == 0 {
        return Err(SerializeError::EnumDiscriminantMissing);
    }
    int_to_bytes(discr_value as i128, size)
}

fn serialize_rust_enum_value(
    input: &InputValue,
    type_graph: &ComplexType,
    type_id: TypeId,
    byte_size: Option<u64>,
    discr_member: Option<&StructureMember>,
    enumerators: &std::collections::HashMap<Option<i64>, StructureMember>,
    runtime_value: Option<&Value>,
    path: &str,
    offsets: &mut Vec<WriteOffset>,
) -> Result<Vec<u8>, SerializeError> {
    let size = byte_size.ok_or(SerializeError::UnknownTypeSize(type_id))?;
    let mut buffer = vec![0u8; size as usize];

    let (variant_name, payload) = match input {
        InputValue::EnumVariant { name, payload } => (name.clone(), payload.as_deref()),
        InputValue::Scalar(name) => (name.clone(), None),
        _ => {
            return Err(SerializeError::UnexpectedInput(
                "enum expects variant".to_string(),
            ));
        }
    };

    let (discr_value, variant_member) = enumerators
        .iter()
        .find(|(_, member)| member.name.as_deref() == Some(variant_name.as_str()))
        .map(|(discr, member)| (discr.unwrap_or_default(), member))
        .ok_or_else(|| SerializeError::EnumVariantNotFound(variant_name.clone()))?;

    if let Some(discr_member) = discr_member {
        let discr_type = discr_member
            .type_ref
            .ok_or(SerializeError::EnumDiscriminantMissing)?;
        let discr_bytes = serialize_value_inner(
            &InputValue::Scalar(discr_value.to_string()),
            type_graph,
            discr_type,
            None,
            path,
            offsets,
        )?;
        let offset = member_offset(discr_member, runtime_value, 0)
            .ok_or_else(|| SerializeError::MemberOffsetMissing("discriminant".to_string()))?;
        let end = offset + discr_bytes.len();
        if end > buffer.len() {
            return Err(SerializeError::SizeMismatch {
                expected: buffer.len(),
                actual: end,
            });
        }
        buffer[offset..end].copy_from_slice(&discr_bytes);
        offsets.push(WriteOffset {
            path: format!("{path}.discriminant"),
            offset,
            size: discr_bytes.len(),
        });
    }

    if let Some(variant_type) = variant_member.type_ref {
        let payload = payload.ok_or_else(|| SerializeError::MissingField(variant_name.clone()))?;
        let payload_bytes = serialize_value_inner(
            payload,
            type_graph,
            variant_type,
            runtime_member(runtime_value, variant_member.name.as_deref(), 0),
            &format!("{path}.{}", variant_name),
            offsets,
        )?;
        let offset = member_offset(variant_member, runtime_value, 0)
            .ok_or_else(|| SerializeError::MemberOffsetMissing(format!("enum::{variant_name}")))?;
        let end = offset + payload_bytes.len();
        if end > buffer.len() {
            return Err(SerializeError::SizeMismatch {
                expected: buffer.len(),
                actual: end,
            });
        }
        buffer[offset..end].copy_from_slice(&payload_bytes);
        offsets.push(WriteOffset {
            path: format!("{path}.{}", variant_name),
            offset,
            size: payload_bytes.len(),
        });
    }

    Ok(buffer)
}

fn member_path_suffix(member: &StructureMember, index: usize) -> String {
    member
        .name
        .as_deref()
        .map(|name| format!(".{name}"))
        .unwrap_or_else(|| format!("[{index}]"))
}

fn member_name(member: &StructureMember, index: usize) -> String {
    member
        .name
        .as_deref()
        .map(|name| name.to_string())
        .unwrap_or_else(|| format!("[{index}]"))
}

fn member_offset(
    member: &StructureMember,
    runtime_value: Option<&Value>,
    index: usize,
) -> Option<usize> {
    match member.in_struct_location.as_ref()? {
        MemberLocation::Offset(offset) => (*offset >= 0).then_some(*offset as usize),
        MemberLocation::Expr(_) => {
            let base = runtime_value?.in_memory_location()? as isize;
            let member_value = runtime_member(runtime_value, member.name.as_deref(), index)?;
            let member_addr = member_value.in_memory_location()? as isize;
            let offset = member_addr - base;
            (offset >= 0).then_some(offset as usize)
        }
    }
}

fn runtime_member<'a>(
    runtime_value: Option<&'a Value>,
    name: Option<&str>,
    index: usize,
) -> Option<&'a Value> {
    let Value::Struct(structure) = runtime_value? else {
        return None;
    };
    if let Some(name) = name {
        structure
            .members
            .iter()
            .find(|member| member.field_name.as_deref() == Some(name))
            .map(|member| &member.value)
    } else {
        structure.members.get(index).map(|member| &member.value)
    }
}

fn runtime_array_item<'a>(runtime_value: Option<&'a Value>, index: usize) -> Option<&'a Value> {
    let Value::Array(array) = runtime_value? else {
        return None;
    };
    array.items.as_ref()?.get(index).map(|item| &item.value)
}

fn int_to_bytes(value: i128, size: usize) -> Result<Vec<u8>, SerializeError> {
    let bytes = match size {
        1 => (value as i8).to_le_bytes().to_vec(),
        2 => (value as i16).to_le_bytes().to_vec(),
        4 => (value as i32).to_le_bytes().to_vec(),
        8 => (value as i64).to_le_bytes().to_vec(),
        16 => value.to_le_bytes().to_vec(),
        0 => Vec::new(),
        _ => {
            return Err(SerializeError::ScalarParse(format!(
                "unsupported int size {size}"
            )));
        }
    };
    Ok(bytes)
}

fn uint_to_bytes(value: u128, size: usize) -> Result<Vec<u8>, SerializeError> {
    let bytes = match size {
        1 => (value as u8).to_le_bytes().to_vec(),
        2 => (value as u16).to_le_bytes().to_vec(),
        4 => (value as u32).to_le_bytes().to_vec(),
        8 => (value as u64).to_le_bytes().to_vec(),
        16 => value.to_le_bytes().to_vec(),
        0 => Vec::new(),
        _ => {
            return Err(SerializeError::ScalarParse(format!(
                "unsupported uint size {size}"
            )));
        }
    };
    Ok(bytes)
}
