use crate::{
    debugger::{
        Debugger, Error, TypeDeclaration,
        address::RelocatedAddress,
        call::{CallArgs, CallContext, CallError, CallHelper, RegType},
        debugee::dwarf::unit::DieAddr,
        variable::{execute::QueryResult, render::RenderValue, value::Value},
    },
    version::RustVersion,
    version_switch,
};
use indexmap::IndexMap;
use itertools::Itertools;
use log::debug;

#[derive(Debug, thiserror::Error)]
pub enum FmtCallError {
    #[error(
        "unable to build vtable for trait object <String as core::fmt::Write>: function `{0}` not found"
    )]
    VTable(String),
    #[error("attempt to call <{{type_name}} as core::fmt::Debug>::fmt but type is unsupported")]
    UnsupportedType,
    #[error("address of given variable not found")]
    AddrNotFound,
    #[error("unexpected result of Debug::fmt: {0}")]
    UnexpectedCallResult(&'static str),
    #[error("error while calling a Debug::fmt function: {0}")]
    Call(#[from] CallError),
    #[error("unsupported rustc version (1.81 is minimal for this future)")]
    UnsupportedRustC,
}

/// VTable for String as core::fmt::Write.
struct WriteStringVTable {
    drop_in_place: usize,
    size: usize,
    alignment: usize,
    write_str: usize,
    write_char: usize,
    write_fmt: usize,
}

impl WriteStringVTable {
    pub fn new(dbg: &Debugger, ver: RustVersion) -> Result<Self, FmtCallError> {
        const STRING_DROP_IN_PLACE: &str = "core::ptr::drop_in_place<alloc::string::String>";
        const STRING_DROP_IN_PLACE_NAME: &str = "drop_in_place<alloc::string::String>";
        const STRING_WRITE_FMT: &str = "core::fmt::Write::write_fmt";

        let string_write_fmt_name = version_switch!(
            ver,
                (1 . 81) .. (1 . 87) =>  "write_fmt<std::io::Write::write_fmt::Adapter<std::io::stdio::StdoutLock>>",
                (1 . 87) .. => "write_fmt<std::io::default_write_fmt::Adapter<std::io::stdio::StdoutLock>>",
            )
            .ok_or(FmtCallError::UnsupportedRustC)?;

        const STRING_WRITE_CHAR: &str = "<alloc::string::String as core::fmt::Write>::write_char";
        const STRING_WRITE_STR: &str = "<alloc::string::String as core::fmt::Write>::write_str";
        const STRING_SIZE: usize = std::mem::size_of::<String>();
        const STRING_ALIGN: usize = std::mem::align_of::<String>();

        let find_fn_for_vtable = |tpl, name_attr| -> Result<RelocatedAddress, FmtCallError> {
            let fn_info = dbg.gcx().with_call_cache(|cc| {
                cc.get_or_insert(dbg, tpl, name_attr)
                    .map_err(|_| FmtCallError::VTable(tpl.to_string()))
            })?;

            Ok(fn_info.fn_addr())
        };

        let drop_in_place_fn_addr =
            find_fn_for_vtable(STRING_DROP_IN_PLACE, Some(STRING_DROP_IN_PLACE_NAME))?;
        let write_fmt_addr = find_fn_for_vtable(STRING_WRITE_FMT, Some(string_write_fmt_name))?;
        let write_char_addr = find_fn_for_vtable(STRING_WRITE_CHAR, None)?;
        let write_str_addr = find_fn_for_vtable(STRING_WRITE_STR, None)?;

        Ok(Self {
            drop_in_place: drop_in_place_fn_addr.as_usize(),
            size: STRING_SIZE,
            alignment: STRING_ALIGN,
            write_str: write_str_addr.as_usize(),
            write_char: write_char_addr.as_usize(),
            write_fmt: write_fmt_addr.as_usize(),
        })
    }

    fn as_raw(&self) -> [usize; 6] {
        [
            self.drop_in_place,
            self.size,
            self.alignment,
            self.write_str,
            self.write_char,
            self.write_fmt,
        ]
    }
}

pub trait DebugFormattable {
    /// Return true whether type may be formatted using special command (vard/argd/etc).
    /// BS try to support as many type as possible, but for now not all types are supported.
    fn formattable(&self) -> bool;
}

impl DebugFormattable for Value {
    fn formattable(&self) -> bool {
        match self {
            Value::Struct(_)
            | Value::Specialized { .. }
            | Value::CEnum(_)
            | Value::RustEnum(_)
            | Value::Array(_) => true,
            // Debug for scalar types and pointers currently omitted
            Value::Scalar(_)
            | Value::Pointer(_)
            | Value::Subroutine(_)
            | Value::CModifiedVariable(_) => false,
        }
    }
}

struct FmtCallingPlan {
    fmt_fn_addr: RelocatedAddress,
    need_indirection: bool,
}

fn create_fmt_calling_plan(dbg: &Debugger, var: &QueryResult) -> Result<FmtCallingPlan, Error> {
    let mut type_name = var.value().r#type().to_string();

    // hack, cause linkage name for `String` type coming without namespace
    if type_name.as_str() == "String" {
        type_name = "alloc::string::String".to_string();
    }

    // fast path, try to find fmt function using "<{type_name} as core::fmt::Debug>::fmt" pattern
    let naive_linkage_name = format!("<{type_name} as core::fmt::Debug>::fmt");
    let fn_info_result = dbg
        .gcx()
        .with_call_cache(|cc| cc.get_or_insert(dbg, &naive_linkage_name, None));
    if let Ok(fn_info) = fn_info_result {
        return Ok(FmtCallingPlan {
            fmt_fn_addr: fn_info.fn_addr(),
            need_indirection: false,
        });
    }

    // slow path
    let mut addr_if_type_params =  |type_params: &IndexMap<String, Option<DieAddr>>| -> Result<Option<RelocatedAddress>, Error> {
        if type_params.is_empty() {
            return Ok(None);
        }
        let params_string: String = type_params.keys().join(",");
        let l_bracket = type_name.find("<").ok_or(FmtCallError::UnsupportedType)?;
        let r_bracket = type_name.find(">").ok_or(FmtCallError::UnsupportedType)?;
        type_name.replace_range(l_bracket + 1..r_bracket, &params_string);
        let linkage_name: String = format!("<{type_name} as core::fmt::Debug>::fmt");

        let concrete_types = type_params
            .values()
            .filter_map(|tref| {
                let tref = tref.as_ref()?;
                let r#type = &var.type_graph().identity(*tref);
                Some(r#type.to_string())
            })
            .join(", ");

        let fmt_fn_name = format!("fmt<{concrete_types}>");

        let fmt_fn_info_result = dbg.gcx()
        .with_call_cache(|cc| cc.get_or_insert(dbg, &linkage_name, Some(&fmt_fn_name)));

        if let Ok(fmt_fn) = fmt_fn_info_result {
            return Ok(Some(fmt_fn.fn_addr()));
        }
        Ok(None)
    };

    let r#type = &var.type_graph().types[&var.type_graph().root()];
    match r#type {
        TypeDeclaration::Array(array_type) => {
            const ARRAY_DEBUG_FMT_LINKAGE_NAME: &str =
                "core::array::<impl core::fmt::Debug for [T; N]>::fmt";

            let el_type_name = var.type_graph().identity(
                array_type
                    .element_type()
                    .ok_or(FmtCallError::UnsupportedType)?,
            );

            let size = var
                .value()
                .as_array()
                .expect("infallible")
                .items
                .as_ref()
                .map(|items| items.len())
                .unwrap_or_default();

            let fmt_fn_name = format!("fmt<{el_type_name}, {size}>");

            let fmt_fn_info_result = dbg.gcx().with_call_cache(|cc| {
                cc.get_or_insert(dbg, ARRAY_DEBUG_FMT_LINKAGE_NAME, Some(&fmt_fn_name))
            });

            if let Ok(fmt_fn) = fmt_fn_info_result {
                return Ok(FmtCallingPlan {
                    fmt_fn_addr: fmt_fn.fn_addr(),
                    need_indirection: false,
                });
            }
        }
        TypeDeclaration::Structure { type_params, .. } => {
            let mb_fn_addr = addr_if_type_params(type_params)?;
            if let Some(fn_addr) = mb_fn_addr {
                return Ok(FmtCallingPlan {
                    fmt_fn_addr: fn_addr,
                    need_indirection: false,
                });
            }
        }
        TypeDeclaration::RustEnum { enumerators, .. } => {
            // to find fmt function name
            // we need to go through the types in the enum variants
            // and find the longest sequence of type parameters in this types

            let variant_type_refs = enumerators
                .values()
                .filter_map(|member| member.type_ref)
                .collect::<Vec<_>>();

            let longest_type_params = variant_type_refs
                .into_iter()
                .filter_map(|vtr| {
                    let vt = &var.type_graph().types[&vtr];
                    let TypeDeclaration::Structure { type_params, .. } = vt else {
                        return None;
                    };
                    Some(type_params)
                })
                .max_by_key(|tp| tp.len());

            let Some(type_params) = longest_type_params else {
                return Err(CallError::FunctionNotFoundOrTooMany.into());
            };

            let mb_fn_addr = addr_if_type_params(type_params)?;
            if let Some(fn_addr) = mb_fn_addr {
                return Ok(FmtCallingPlan {
                    fmt_fn_addr: fn_addr,
                    need_indirection: false,
                });
            }
        }
        _ => {}
    }

    // name still not found, try to find using pattern:
    // linkage name: <&T as core::fmt::Debug>::fmt
    // fn name: fmt<{type_name}>
    // Cause &T is used one more indirection level is needed.
    // Currently this used in just one case - for &str type,
    // but perhaps the use will expand in the future
    let fmt_fn_name = format!("fmt<{type_name}>");
    let fmt_fn_info_result = dbg.gcx().with_call_cache(|cc| {
        cc.get_or_insert(dbg, "<&T as core::fmt::Debug>::fmt", Some(&fmt_fn_name))
    });

    if let Ok(fmt_fn) = fmt_fn_info_result {
        return Ok(FmtCallingPlan {
            fmt_fn_addr: fmt_fn.fn_addr(),
            need_indirection: true,
        });
    }

    Err(CallError::FunctionNotFoundOrTooMany.into())
}

fn formatter_to_bytes<const N: usize, F>(formatter: &F) -> [u8; N] {
    let src: *const u8 = formatter as *const F as *const u8;
    let mut formatter_bytes = [0u8; N];
    for (i, b) in formatter_bytes.iter_mut().enumerate() {
        // SAFETY: memory always reachable
        *b = unsafe { *src.add(i) };
    }
    formatter_bytes
}

fn make_formatter_bytes_rust_1_87_plus(string_header_ptr: usize, vtable_ptr: usize) -> Vec<u8> {
    // Layout of this structures should be equals to std::fmt::Formatter and std::fmt::FormattingOptions

    #[allow(unused)]
    #[derive(PartialEq)]
    struct FormattingOptions {
        flags: u32,
        width: u16,
        precision: u16,
    }
    #[allow(unused)]
    #[derive(PartialEq)]
    pub struct Formatter {
        buf_string_ptr: usize,
        buf_vtable_ptr: usize,
        options: FormattingOptions,
    }
    const FORMATTER_SZ: usize = std::mem::size_of::<Formatter>();

    const ALIGN_UNKNOWN: u32 = 3 << 29;
    const ALWAYS_SET: u32 = 1 << 31;

    let formatter = Formatter {
        options: FormattingOptions {
            flags: ' ' as u32 | ALIGN_UNKNOWN | ALWAYS_SET,
            width: 0,
            precision: 0,
        },
        buf_string_ptr: string_header_ptr,
        buf_vtable_ptr: vtable_ptr,
    };

    formatter_to_bytes::<FORMATTER_SZ, _>(&formatter).to_vec()
}

fn make_formatter_bytes_rust_1_85_1_87(string_header_ptr: usize, vtable_ptr: usize) -> Vec<u8> {
    // Layout of this structures should be equals to std::fmt::Formatter and std::fmt::FormattingOptions

    #[allow(unused)]
    #[derive(PartialEq)]
    struct FormattingOptions {
        flags: u32,
        fill: char,
        align: Option<std::fmt::Alignment>,
        width: Option<usize>,
        precision: Option<usize>,
    }
    #[allow(unused)]
    #[derive(PartialEq)]
    pub struct Formatter {
        options: FormattingOptions,
        buf_string_ptr: usize,
        buf_vtable_ptr: usize,
    }
    const FORMATTER_SZ: usize = std::mem::size_of::<Formatter>();

    let formatter = Formatter {
        options: FormattingOptions {
            flags: 0,
            fill: ' ',
            align: None,
            width: None,
            precision: None,
        },
        buf_string_ptr: string_header_ptr,
        buf_vtable_ptr: vtable_ptr,
    };

    formatter_to_bytes::<FORMATTER_SZ, _>(&formatter).to_vec()
}

fn make_formatter_bytes_1_81_1_85(string_header_ptr: usize, vtable_ptr: usize) -> Vec<u8> {
    #[allow(unused)]
    #[derive(Copy, Clone, PartialEq, Eq)]
    enum Alignment {
        Left,
        Right,
        Center,
        Unknown,
    }

    #[allow(unused)]
    pub struct Formatter {
        flags: u32,
        fill: char,
        align: Alignment,
        width: Option<usize>,
        precision: Option<usize>,

        buf_string_ptr: usize,
        buf_vtable_ptr: usize,
    }

    let formatter = Formatter {
        flags: 0,
        fill: ' ',
        align: Alignment::Unknown,
        width: None,
        precision: None,

        buf_string_ptr: string_header_ptr,
        buf_vtable_ptr: vtable_ptr,
    };
    const FORMATTER_SZ: usize = std::mem::size_of::<Formatter>();

    formatter_to_bytes::<FORMATTER_SZ, _>(&formatter).to_vec()
}

/// Call a core::fmt::Debug::fmt function for a variable and return formatted string.
pub fn call_debug_fmt(dbg: &Debugger, var: &QueryResult) -> Result<String, Error> {
    if !var.value().formattable() {
        return Err(FmtCallError::UnsupportedType.into());
    }
    assert!(std::mem::size_of::<Vec::<u8>>() == 24);

    let rust_version = var
        .unit()
        .rustc_version()
        .ok_or(FmtCallError::UnsupportedRustC)?;

    debug!("prepare calling plan for core::fmt::Debug::fmt");
    let calling_plan = create_fmt_calling_plan(dbg, var)?;

    debug!("prepare vtable for String as core::fmt::Write");
    let write_string_vtable = WriteStringVTable::new(dbg, rust_version)?;

    debug!("allocate memory for String header and std::fmt::Formatter");
    let ccx = CallContext::new(dbg)?;
    let (alloc_ptr, formatter_ptr, string_header_ptr, self_arg) = ccx.with_ccx(|ccx| {
        // use vector cause string have a same layout
        let vec = Vec::<u8>::new();

        // SAFETY: `vec`` is never allocate memory on the heap (since the capacity is 0),
        // so no memory leaks should occur here
        let vec_header_bytes: [u8; 24] = unsafe { std::mem::transmute::<_, [u8; 24]>(vec) };
        let alloc_ptr = CallHelper::mmap(ccx)?;

        // write string header
        let string_header_ptr = alloc_ptr as usize;
        for offset in (0usize..3).map(|el| el * size_of::<usize>()) {
            let bytes = &vec_header_bytes[offset..offset + size_of::<usize>()];
            let value = usize::from_ne_bytes(bytes.try_into().expect("infallible"));
            ccx.dbg.write_memory(string_header_ptr + offset, value)?;
        }

        // write vtable for <String as core::fmt::Write>
        let vtable_raw = write_string_vtable.as_raw();
        let vtable_ptr = string_header_ptr + vec_header_bytes.len();
        for (i, value) in vtable_raw.into_iter().enumerate() {
            ccx.dbg
                .write_memory(vtable_ptr + i * size_of::<usize>(), value)?;
        }

        let formatter_bytes = version_switch!(
            rust_version,
                (1 . 81) .. (1 . 85) =>  make_formatter_bytes_1_81_1_85(string_header_ptr, vtable_ptr),
                (1 . 85) .. (1 . 87) => make_formatter_bytes_rust_1_85_1_87(string_header_ptr, vtable_ptr),
                (1 . 87) .. => make_formatter_bytes_rust_1_87_plus(string_header_ptr, vtable_ptr),
            )
            .ok_or(FmtCallError::UnsupportedRustC)?;

        let formatter_size = formatter_bytes.len();

        let formatter_ptr = vtable_ptr + (vtable_raw.len() * 8);
        for offset in (0..(formatter_size / size_of::<usize>())).map(|el| el * size_of::<usize>()) {
            let bytes = &formatter_bytes[offset..offset + size_of::<usize>()];
            let value = usize::from_ne_bytes(bytes.try_into().expect("infallible"));
            ccx.dbg.write_memory(formatter_ptr + offset, value)?;
        }

        let var_addr = var
            .value()
            .in_memory_location()
            .ok_or(FmtCallError::AddrNotFound)?;

        if calling_plan.need_indirection {
            let self_ptr = formatter_ptr + formatter_size;
            ccx.dbg.write_memory(self_ptr, var_addr)?;
            return Ok((alloc_ptr, formatter_ptr, string_header_ptr, self_ptr));
        }

        Ok((alloc_ptr, formatter_ptr, string_header_ptr, var_addr))
    })?;

    let args = CallArgs(Box::new([
        (self_arg as u64, RegType::General),
        (formatter_ptr as u64, RegType::General),
    ]));

    let mut debug_fmt_call_result = dbg
        .call_fn_raw(calling_plan.fmt_fn_addr, args)
        .map(|_| String::new());

    let new_string_header = dbg.read_memory(string_header_ptr, 24)?;
    let string_cap = usize::from_ne_bytes((&new_string_header[..8]).try_into().unwrap());
    let string_data_ptr = usize::from_ne_bytes((&new_string_header[8..16]).try_into().unwrap());
    let string_len = usize::from_ne_bytes((&new_string_header[16..24]).try_into().unwrap());

    if string_cap < string_len {
        debug_fmt_call_result =
            Err(FmtCallError::UnexpectedCallResult("allocated cap are less than len").into());
    }

    if debug_fmt_call_result.is_ok() {
        let string_data_res = dbg.read_memory(string_data_ptr, string_len);
        match string_data_res {
            Ok(string_data) => {
                let s = String::from_utf8_lossy(&string_data);
                debug_fmt_call_result = Ok(s.to_string());
            }
            Err(e) => debug_fmt_call_result = Err(e),
        };
    };

    debug!(target: "debugger", "dealloc format string");

    dbg.call_fn_raw(
        RelocatedAddress::from(write_string_vtable.drop_in_place),
        CallArgs(Box::new([(string_header_ptr as u64, RegType::General)])),
    )
    .expect("failed to retrieve original program state after a call");

    debug!(target: "debugger", "dealloc temporary memory area");

    let ccx =
        CallContext::new(dbg).expect("failed to retrieve original program state after a call");
    ccx.with_ccx(|ccx| CallHelper::munmap(ccx, alloc_ptr))
        .expect("failed to retrieve original program state after a call");

    debug_fmt_call_result
}
