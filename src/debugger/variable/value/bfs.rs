use crate::debugger::variable::value::Value;
use std::collections::VecDeque;

#[derive(PartialEq, Debug)]
pub(super) enum FieldOrIndex<'a> {
    Field(Option<&'a str>),
    Index(i64),
    Root,
}

/// Iterator for visits underline values in BFS order.
pub(super) struct BfsIterator<'a> {
    pub(super) queue: VecDeque<(FieldOrIndex<'a>, &'a Value)>,
}

impl<'a> Iterator for BfsIterator<'a> {
    type Item = (FieldOrIndex<'a>, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        let (field_or_idx, next_value) = self.queue.pop_front()?;

        match next_value {
            Value::Struct(r#struct) => {
                r#struct.members.iter().for_each(|member| {
                    let item = (
                        FieldOrIndex::Field(member.field_name.as_deref()),
                        &member.value,
                    );

                    self.queue.push_back(item)
                });
            }
            Value::Array(array) => {
                if let Some(items) = array.items.as_ref() {
                    items.iter().for_each(|item| {
                        let item = (FieldOrIndex::Index(item.index), &item.value);
                        self.queue.push_back(item)
                    })
                }
            }
            Value::RustEnum(r#enum) => {
                if let Some(enumerator) = r#enum.value.as_ref() {
                    let item = (
                        FieldOrIndex::Field(enumerator.field_name.as_deref()),
                        &enumerator.value,
                    );
                    self.queue.push_back(item)
                }
            }
            Value::Pointer(_) => {}
            Value::Specialized {
                original: origin, ..
            } => origin.members.iter().for_each(|member| {
                let item = (
                    FieldOrIndex::Field(member.field_name.as_deref()),
                    &member.value,
                );
                self.queue.push_back(item)
            }),
            _ => {}
        }

        Some((field_or_idx, next_value))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::debugger::debugee::dwarf::r#type::TypeIdentity;
    use crate::debugger::variable::value::{
        ArrayItem, ArrayValue, Member, PointerValue, RustEnumValue, ScalarValue, StructValue,
    };

    #[test]
    fn test_bfs_iterator() {
        struct TestCase {
            variable: Value,
            expected_order: Vec<FieldOrIndex<'static>>,
        }

        let test_cases = vec![
            TestCase {
                variable: Value::Struct(StructValue {
                    type_ident: TypeIdentity::unknown(),
                    type_id: None,
                    members: vec![
                        Member {
                            field_name: Some("array_1".to_owned()),
                            value: Value::Array(ArrayValue {
                                type_id: None,
                                type_ident: TypeIdentity::unknown(),
                                items: Some(vec![
                                    ArrayItem {
                                        index: 1,
                                        value: Value::Scalar(ScalarValue {
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                            type_id: None,
                                        }),
                                    },
                                    ArrayItem {
                                        index: 2,
                                        value: Value::Scalar(ScalarValue {
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                            type_id: None,
                                        }),
                                    },
                                ]),
                                raw_address: None,
                            }),
                        },
                        Member {
                            field_name: Some("array_2".to_owned()),
                            value: Value::Array(ArrayValue {
                                type_ident: TypeIdentity::unknown(),
                                type_id: None,
                                items: Some(vec![
                                    ArrayItem {
                                        index: 3,
                                        value: Value::Scalar(ScalarValue {
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                            type_id: None,
                                        }),
                                    },
                                    ArrayItem {
                                        index: 4,
                                        value: Value::Scalar(ScalarValue {
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                            type_id: None,
                                        }),
                                    },
                                ]),
                                raw_address: None,
                            }),
                        },
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                expected_order: vec![
                    FieldOrIndex::Root,
                    FieldOrIndex::Field(Some("array_1")),
                    FieldOrIndex::Field(Some("array_2")),
                    FieldOrIndex::Index(1),
                    FieldOrIndex::Index(2),
                    FieldOrIndex::Index(3),
                    FieldOrIndex::Index(4),
                ],
            },
            TestCase {
                variable: Value::Struct(StructValue {
                    type_id: None,
                    type_ident: TypeIdentity::unknown(),
                    members: vec![
                        Member {
                            field_name: Some("struct_2".to_owned()),
                            value: Value::Struct(StructValue {
                                type_id: None,
                                type_ident: TypeIdentity::unknown(),
                                members: vec![
                                    Member {
                                        field_name: Some("scalar_1".to_owned()),
                                        value: Value::Scalar(ScalarValue {
                                            type_id: None,
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                        }),
                                    },
                                    Member {
                                        field_name: Some("enum_1".to_owned()),
                                        value: Value::RustEnum(RustEnumValue {
                                            type_id: None,
                                            type_ident: TypeIdentity::unknown(),
                                            value: Some(Box::new(Member {
                                                field_name: Some("scalar_2".to_owned()),
                                                value: Value::Scalar(ScalarValue {
                                                    type_id: None,
                                                    type_ident: TypeIdentity::unknown(),
                                                    value: None,
                                                    raw_address: None,
                                                }),
                                            })),
                                            raw_address: None,
                                        }),
                                    },
                                    Member {
                                        field_name: Some("scalar_3".to_owned()),
                                        value: Value::Scalar(ScalarValue {
                                            type_id: None,
                                            type_ident: TypeIdentity::unknown(),
                                            value: None,
                                            raw_address: None,
                                        }),
                                    },
                                ],
                                type_params: Default::default(),
                                raw_address: None,
                            }),
                        },
                        Member {
                            field_name: Some("pointer_1".to_owned()),
                            value: Value::Pointer(PointerValue {
                                type_id: None,
                                type_ident: TypeIdentity::unknown(),
                                value: None,
                                target_type: None,
                                target_type_size: None,
                                raw_address: None,
                            }),
                        },
                    ],
                    type_params: Default::default(),
                    raw_address: None,
                }),
                expected_order: vec![
                    FieldOrIndex::Root,
                    FieldOrIndex::Field(Some("struct_2")),
                    FieldOrIndex::Field(Some("pointer_1")),
                    FieldOrIndex::Field(Some("scalar_1")),
                    FieldOrIndex::Field(Some("enum_1")),
                    FieldOrIndex::Field(Some("scalar_3")),
                    FieldOrIndex::Field(Some("scalar_2")),
                ],
            },
        ];

        for tc in test_cases {
            let iter = tc.variable.bfs_iterator();
            let names: Vec<_> = iter.map(|(field_or_idx, _)| field_or_idx).collect();
            assert_eq!(tc.expected_order, names);
        }
    }
}
