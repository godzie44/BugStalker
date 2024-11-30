use crate::debugger;
use crate::debugger::debugee::dwarf::r#type::{
    ComplexType, EvaluationContext, StructureMember, TypeIdentity,
};
use crate::debugger::variable::select::ObjectBinaryRepr;
use crate::debugger::variable::AssumeError::NoType;
use crate::debugger::variable::ParsingError::ReadDebugeeMemory;
use crate::debugger::variable::{AssumeError, ParsingError};
use crate::debugger::TypeDeclaration;
use fallible_iterator::FallibleIterator;
use std::mem;
use std::ptr::NonNull;

const B: usize = 6;

/// Helper function, returns true if structure member name exists and starts with `starts_with` string.
fn assert_member_name(member: &StructureMember, starts_with: &str) -> bool {
    member
        .name
        .as_ref()
        .map(|name| name.starts_with(starts_with))
        .unwrap_or_default()
}

/// LeafNodeMarkup represent meta information for `LeafNode<K, V>` type.
struct LeafNodeMarkup {
    parent: StructureMember,
    parent_idx: StructureMember,
    len: StructureMember,
    keys: StructureMember,
    vals: StructureMember,
    size: usize,
}

impl LeafNodeMarkup {
    /// Returns meta information about `LeafNode<K, V>` where `key_type_id` is id of K type and
    /// `value_type_id` as an id of V type. Result node are closest to type with id `map_id`.
    fn from_type(
        r#type: &ComplexType,
        map_id: TypeIdentity,
        key_type_id: TypeIdentity,
        value_type_id: TypeIdentity,
    ) -> Option<LeafNodeMarkup> {
        let mut iterator = r#type.bfs_iterator(map_id);

        let (members, byte_size) = iterator.find_map(|type_decl| {
            if let TypeDeclaration::Structure {
                name,
                members,
                byte_size,
                type_params,
                ..
            } = type_decl
            {
                if name.as_ref()?.starts_with("LeafNode") {
                    let v_found = type_params
                        .iter()
                        .any(|(_, &type_id)| type_id == Some(value_type_id));
                    let k_found = type_params
                        .iter()
                        .any(|(_, &type_id)| type_id == Some(key_type_id));

                    if v_found & k_found {
                        return Some((members, byte_size));
                    }
                }
            }
            None
        })?;
        let size = (*byte_size)? as usize;

        let parent_member = members.iter().find(|&m| assert_member_name(m, "parent"))?;
        let parent_idx_member = members
            .iter()
            .find(|&m| assert_member_name(m, "parent_idx"))?;
        let len_member = members.iter().find(|&m| assert_member_name(m, "len"))?;
        let keys_member = members.iter().find(|&m| assert_member_name(m, "keys"))?;
        let vals_member = members.iter().find(|&m| assert_member_name(m, "vals"))?;

        Some(LeafNodeMarkup {
            parent: parent_member.clone(),
            parent_idx: parent_idx_member.clone(),
            len: len_member.clone(),
            keys: keys_member.clone(),
            vals: vals_member.clone(),
            size,
        })
    }
}

/// InternalNodeMarkup represent meta information for `InternalNode<K, V>` type.
struct InternalNodeMarkup {
    data: StructureMember,
    edges: StructureMember,
    size: usize,
}

impl InternalNodeMarkup {
    /// Returns meta information about `InternalNode<K, V>` where `key_type_id` is id of K type and
    /// `value_type_id` as an id of V type. Result node are closest to type with id `map_id`.
    fn from_type(
        r#type: &ComplexType,
        map_id: TypeIdentity,
        key_type_id: TypeIdentity,
        value_type_id: TypeIdentity,
    ) -> Option<InternalNodeMarkup> {
        let mut iterator = r#type.bfs_iterator(map_id);

        let (members, byte_size) = iterator.find_map(|type_decl| {
            if let TypeDeclaration::Structure {
                name,
                members,
                byte_size,
                type_params,
                ..
            } = type_decl
            {
                if name.as_ref()?.starts_with("InternalNode") {
                    let v_found = type_params
                        .iter()
                        .any(|(_, &type_id)| type_id == Some(value_type_id));
                    let k_found = type_params
                        .iter()
                        .any(|(_, &type_id)| type_id == Some(key_type_id));

                    if v_found & k_found {
                        return Some((members, byte_size));
                    }
                }
            }
            None
        })?;

        let size = (*byte_size)? as usize;
        let data_member = members.iter().find(|&m| assert_member_name(m, "data"))?;
        let edges_member = members.iter().find(|&m| assert_member_name(m, "edges"))?;

        Some(InternalNodeMarkup {
            data: data_member.clone(),
            edges: edges_member.clone(),
            size,
        })
    }
}

/// Represent btree leaf node.
struct Leaf {
    parent: Option<NonNull<()>>,
    parent_idx: u16,
    len: u16,
    keys_debugee_location: Option<usize>,
    keys_raw: Vec<u8>,
    vals_debugee_location: Option<usize>,
    vals_raw: Vec<u8>,
}

impl Leaf {
    fn from_markup(
        eval_ctx: &EvaluationContext,
        r#type: &ComplexType,
        ptr: *const (),
        markup: &LeafNodeMarkup,
    ) -> Result<Leaf, ParsingError> {
        let leaf_bytes = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            ptr as usize,
            markup.size,
        )
        .map_err(ReadDebugeeMemory)?;
        let data = ObjectBinaryRepr {
            raw_data: bytes::Bytes::from(leaf_bytes),
            address: Some(ptr as usize),
            size: markup.size,
        };
        Ok(Self::from_bytes(eval_ctx, r#type, data, markup)?)
    }

    fn from_bytes(
        eval_ctx: &EvaluationContext,
        r#type: &ComplexType,
        data: ObjectBinaryRepr,
        markup: &LeafNodeMarkup,
    ) -> Result<Leaf, AssumeError> {
        let parent = unsafe {
            const EXPECTED_SIZE: usize = mem::size_of::<Option<NonNull<()>>>();
            mem::transmute::<[u8; EXPECTED_SIZE], Option<NonNull<()>>>(
                markup
                    .parent
                    .value(eval_ctx, r#type, &data)
                    .ok_or(AssumeError::NoData("leaf node (parent)"))?
                    .raw_data
                    .to_vec()
                    .try_into()
                    .map_err(|data: Vec<_>| {
                        AssumeError::UnexpectedBinaryRepr(
                            "leaf node (parent)",
                            EXPECTED_SIZE,
                            data.len(),
                        )
                    })?,
            )
        };

        let len_bytes = markup
            .len
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("leaf node (len)"))?
            .raw_data
            .to_vec();
        let len = u16::from_ne_bytes(len_bytes.try_into().map_err(|data: Vec<_>| {
            AssumeError::UnexpectedBinaryRepr("leaf node len", 2, data.len())
        })?);
        let parent_idx_bytes = markup
            .parent_idx
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("leaf node (parent index)"))?
            .raw_data
            .to_vec();
        let parent_idx =
            u16::from_ne_bytes(parent_idx_bytes.try_into().map_err(|data: Vec<_>| {
                AssumeError::UnexpectedBinaryRepr("leaf node parent index", 2, data.len())
            })?);

        let keys_data = markup
            .keys
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("leaf node (keys)"))?;
        let vals_data = markup
            .vals
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("leaf node (vals)"))?;

        Ok(Leaf {
            parent,
            parent_idx,
            len,
            keys_debugee_location: keys_data.address,
            keys_raw: keys_data.raw_data.to_vec(),
            vals_debugee_location: vals_data.address,
            vals_raw: vals_data.raw_data.to_vec(),
        })
    }
}

/// Represent btree internal node.
struct Internal {
    leaf: Leaf,
    edges: [*const (); 2 * B],
}

impl Internal {
    fn from_markup(
        eval_ctx: &EvaluationContext,
        r#type: &ComplexType,
        ptr: *const (),
        l_markup: &LeafNodeMarkup,
        i_markup: &InternalNodeMarkup,
    ) -> Result<Self, ParsingError> {
        let bytes = debugger::read_memory_by_pid(
            eval_ctx.expl_ctx.pid_on_focus(),
            ptr as usize,
            i_markup.size,
        )
        .map_err(ReadDebugeeMemory)?;
        let data = ObjectBinaryRepr {
            raw_data: bytes::Bytes::from(bytes),
            address: Some(ptr as usize),
            size: i_markup.size,
        };
        let edges_v = i_markup
            .edges
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("internal node (edges_v)"))?
            .raw_data
            .to_vec()
            .chunks_exact(mem::size_of::<usize>())
            .map(|chunk| {
                usize::from_ne_bytes(
                    chunk
                        .try_into()
                        .expect("unreachable: 8 bytes chunk must be convertible for usize"),
                ) as *const ()
            })
            .collect::<Vec<_>>();
        let edges: [*const (); B * 2] = edges_v
            .try_into()
            .map_err(|_edges: Vec<_>| AssumeError::NoData("internal node (edges_v)"))?;

        let leaf_bytes = i_markup
            .data
            .value(eval_ctx, r#type, &data)
            .ok_or(AssumeError::NoData("internal node (leaf_bytes)"))?;

        Ok(Internal {
            leaf: Leaf::from_bytes(eval_ctx, r#type, leaf_bytes, l_markup)?,
            edges,
        })
    }
}

enum LeafOrInternal {
    Leaf(Leaf),
    Internal(Internal),
}

impl LeafOrInternal {
    fn len(&self) -> u16 {
        match self {
            LeafOrInternal::Leaf(leaf) => leaf.len,
            LeafOrInternal::Internal(internal) => internal.leaf.len,
        }
    }

    fn leaf(&self) -> &Leaf {
        match self {
            LeafOrInternal::Leaf(leaf) => leaf,
            LeafOrInternal::Internal(internal) => &internal.leaf,
        }
    }

    fn internal(&self) -> &Internal {
        match self {
            LeafOrInternal::Leaf(_) => panic!("not an internal"),
            LeafOrInternal::Internal(internal) => internal,
        }
    }
}

/// BTree node representation.
pub(super) struct Node {
    data: LeafOrInternal,
    height: usize,
}

/// BTree node and item in it.
struct Handle {
    node: Node,
    idx: usize,
}

impl Handle {
    fn node_is_leaf(&self) -> bool {
        self.node.height == 0
    }

    /// Returns false if node is not valid.
    /// Caller must ascend node if it possible.
    fn is_right_kv(&self) -> bool {
        let len = self.node.data.len() as usize;
        self.idx < len
    }

    /// Returns underline key and value.
    fn data(
        &self,
        k_size: usize,
        v_size: usize,
    ) -> nix::Result<(ObjectBinaryRepr, ObjectBinaryRepr)> {
        let leaf = self.node.data.leaf();
        let key_offset = k_size * self.idx;
        let key_raw = leaf.keys_raw[key_offset..k_size * (self.idx + 1)].to_vec();
        let key_data = ObjectBinaryRepr {
            raw_data: bytes::Bytes::from(key_raw),
            address: leaf.keys_debugee_location.map(|addr| addr + key_offset),
            size: k_size,
        };
        let val_offset = v_size * self.idx;
        let val_raw = leaf.vals_raw[val_offset..v_size * (self.idx + 1)].to_vec();
        let val_data = ObjectBinaryRepr {
            raw_data: bytes::Bytes::from(val_raw),
            address: leaf.vals_debugee_location.map(|addr| addr + val_offset),
            size: v_size,
        };

        Ok((key_data, val_data))
    }

    fn next_leaf_edge(
        self,
        eval_ctx: &EvaluationContext,
        reflection: &BTreeReflection,
    ) -> Result<Self, ParsingError> {
        if self.node_is_leaf() {
            Ok(Handle {
                node: self.node,
                idx: self.idx + 1,
            })
        } else {
            let mut idx = self.idx + 1;

            let internal = self.node.data.internal();
            let mut node =
                reflection.make_node(eval_ctx, internal.edges[idx], self.node.height - 1)?;

            while node.height != 0 {
                idx = 0;
                let internal = node.data.internal();
                node = reflection.make_node(eval_ctx, internal.edges[idx], node.height - 1)?;
            }

            if node.height == 0 {
                idx = 0;
            }

            Ok(Handle { node, idx })
        }
    }

    /// Returns first leaf of tree with root in handle.
    fn first_leaf_edge(
        self,
        eval_ctx: &EvaluationContext,
        reflection: &BTreeReflection,
    ) -> Result<Handle, ParsingError> {
        let mut handle = self;

        while !handle.node_is_leaf() {
            let internal = handle.node.data.internal();
            handle = Handle {
                node: reflection.make_node(eval_ctx, internal.edges[0], handle.node.height - 1)?,
                idx: 0,
            }
        }

        Ok(handle)
    }

    /// Ascend node. Return None if current node is root.
    pub(crate) fn try_ascend(
        &self,
        eval_ctx: &EvaluationContext,
        reflection: &BTreeReflection,
    ) -> Result<Option<Handle>, ParsingError> {
        let leaf = self.node.data.leaf();
        let parent = match leaf.parent {
            None => return Ok(None),
            Some(p) => p,
        };

        Ok(Some(Handle {
            node: reflection.make_node(eval_ctx, parent.as_ptr(), self.node.height + 1)?,
            idx: leaf.parent_idx as usize,
        }))
    }
}

/// Reflection of BTreeMap data structure.
pub struct BTreeReflection<'a> {
    root: *const (),
    root_h: usize,
    internal_markup: InternalNodeMarkup,
    leaf_markup: LeafNodeMarkup,
    r#type: &'a ComplexType,
    k_type_id: TypeIdentity,
    v_type_id: TypeIdentity,
}

impl<'a> BTreeReflection<'a> {
    /// Creates new BTreeReflection.
    pub fn new(
        r#type: &'a ComplexType,
        root_ptr: *const (),
        root_height: usize,
        map_id: TypeIdentity,
        k_type_id: TypeIdentity,
        v_type_id: TypeIdentity,
    ) -> Result<Self, AssumeError> {
        Ok(Self {
            root: root_ptr,
            root_h: root_height,
            internal_markup: InternalNodeMarkup::from_type(r#type, map_id, k_type_id, v_type_id)
                .ok_or(NoType("internal node"))?,
            leaf_markup: LeafNodeMarkup::from_type(r#type, map_id, k_type_id, v_type_id)
                .ok_or(NoType("leaf node"))?,
            r#type,
            k_type_id,
            v_type_id,
        })
    }

    fn make_node(
        &self,
        eval_ctx: &EvaluationContext,
        node_ptr: *const (),
        height: usize,
    ) -> Result<Node, ParsingError> {
        let data = if height == 0 {
            LeafOrInternal::Leaf(Leaf::from_markup(
                eval_ctx,
                self.r#type,
                node_ptr,
                &self.leaf_markup,
            )?)
        } else {
            LeafOrInternal::Internal(Internal::from_markup(
                eval_ctx,
                self.r#type,
                node_ptr,
                &self.leaf_markup,
                &self.internal_markup,
            )?)
        };

        Ok(Node { data, height })
    }

    /// Creates new BTreeMap key-value iterator.
    pub fn iter(self, eval_ctx: &'a EvaluationContext) -> Result<KVIterator<'a>, AssumeError> {
        let k_size = self
            .r#type
            .type_size_in_bytes(eval_ctx, self.k_type_id)
            .ok_or(AssumeError::UnknownSize("btree key type".into()))?;
        let v_size = self
            .r#type
            .type_size_in_bytes(eval_ctx, self.v_type_id)
            .ok_or(AssumeError::UnknownSize("btree value type".into()))?;

        Ok(KVIterator {
            reflection: self,
            handle: None,
            eval_ctx,
            k_size: k_size as usize,
            v_size: v_size as usize,
        })
    }
}

pub struct KVIterator<'a> {
    reflection: BTreeReflection<'a>,
    eval_ctx: &'a EvaluationContext<'a>,
    handle: Option<Handle>,
    k_size: usize,
    v_size: usize,
}

impl FallibleIterator for KVIterator<'_> {
    type Item = (ObjectBinaryRepr, ObjectBinaryRepr);
    type Error = ParsingError;

    fn next(&mut self) -> Result<Option<Self::Item>, Self::Error> {
        let mut handle = match self.handle.take() {
            None => Handle {
                node: self.reflection.make_node(
                    self.eval_ctx,
                    self.reflection.root,
                    self.reflection.root_h,
                )?,
                idx: 0,
            }
            .first_leaf_edge(self.eval_ctx, &self.reflection)?,
            Some(handle) => handle,
        };

        loop {
            let is_kv = handle.is_right_kv();
            if !is_kv {
                handle = match handle.try_ascend(self.eval_ctx, &self.reflection)? {
                    None => return Ok(None),
                    Some(h) => h,
                };
                continue;
            }

            let data = handle.data(self.k_size, self.v_size)?;

            self.handle = Some(handle.next_leaf_edge(self.eval_ctx, &self.reflection)?);

            return Ok(Some(data));
        }
    }
}
