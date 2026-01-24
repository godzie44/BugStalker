use crate::{
    debugger::{
        Error,
        debugee::dwarf::{EndianArcSlice, unit::DieAddr},
    },
    weak_error,
};
use gimli::{
    Attribute, AttributeValue, DW_AT_byte_size, DW_AT_const_value, DW_AT_count,
    DW_AT_data_member_location, DW_AT_discr, DW_AT_discr_value, DW_AT_encoding, DW_AT_frame_base,
    DW_AT_location, DW_AT_lower_bound, DW_AT_name, DW_AT_type, DW_AT_upper_bound,
    DebuggingInformationEntry, DwAt, DwTag, Dwarf, Range, Reader, Unit, UnitOffset,
};
use std::collections::VecDeque;

/// Deref context (or dcx). Context suitable for dereference DIE references
#[derive(Clone)]
pub struct DerefContext<'unit, 'dwarf: 'unit> {
    dwarf: &'dwarf Dwarf<EndianArcSlice>,
    unit: &'unit Unit<EndianArcSlice>,
}

impl<'unit, 'dwarf: 'unit> DerefContext<'unit, 'dwarf> {
    pub fn new(dwarf: &'dwarf Dwarf<EndianArcSlice>, unit: &'unit Unit<EndianArcSlice>) -> Self {
        Self { dwarf, unit }
    }
}

/// Debug information entry representation
pub enum Die<'a> {
    /// generated DIE, currently contains only DW_AT_type
    Virtual { type_ref: Option<DieAddr> },
    /// DIE located in debug information sections
    Dwarf {
        dcx: DerefContext<'a, 'a>,
        die: DebuggingInformationEntry<EndianArcSlice>,
    },
}

impl<'a> Die<'a> {
    /// Take DIE from debug information
    pub fn new(dcx: DerefContext<'a, 'a>, offset: UnitOffset) -> Result<Die<'a>, Error> {
        let die = dcx
            .unit
            .entry(offset)
            .map_err(|_| Error::DieNotFound(DieAddr::Unit(offset)))?;
        Ok(Die::Dwarf { dcx, die })
    }
}

macro_rules! impl_no_virt {
    ($name: ident, $rty: ty, $fn: expr) => {
        pub fn $name(&self) -> $rty {
            match self {
                Die::Virtual { .. } => unimplemented!(),
                Die::Dwarf { dcx, die } => $fn(dcx, die),
            }
        }
    };
}

type GimliDie<'a> = &'a DebuggingInformationEntry<EndianArcSlice>;

impl<'a> Die<'a> {
    impl_no_virt!(offset, UnitOffset, |_, die: GimliDie| { die.offset() });

    impl_no_virt!(tag, DwTag, |_, die: GimliDie| { die.tag() });

    impl_no_virt!(name, Option<String>, |dcx: &DerefContext, die: GimliDie| {
        Self::attr_to_string(dcx.dwarf, dcx.unit, die, DW_AT_name).ok()?
    });

    impl_no_virt!(ranges, Box<[Range]>, |dcx: &DerefContext, die: GimliDie| {
        dcx.dwarf
            .die_ranges(dcx.unit, die)
            .unwrap_or_default()
            .collect::<Result<Vec<Range>, _>>()
            .unwrap_or_default()
            .into()
    });

    pub fn type_ref(&self) -> Option<DieAddr> {
        match self {
            Die::Virtual { type_ref } => *type_ref,
            Die::Dwarf { die, .. } => die.attr(DW_AT_type).and_then(DieAddr::from_attr),
        }
    }

    impl_no_virt!(discr_ref, Option<DieAddr>, |_, die: GimliDie| {
        die.attr(DW_AT_discr).and_then(DieAddr::from_attr)
    });

    impl_no_virt!(byte_size, Option<u64>, |_, die: GimliDie| {
        die.attr(DW_AT_byte_size).and_then(|val| val.udata_value())
    });

    impl_no_virt!(discr_value, Option<i64>, |_, die: GimliDie| {
        die.attr(DW_AT_discr_value)
            .and_then(|val| val.sdata_value())
    });

    impl_no_virt!(const_value, Option<i64>, |_, die: GimliDie| {
        die.attr(DW_AT_const_value)
            .and_then(|val| val.sdata_value())
    });

    impl_no_virt!(
        location,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_location).cloned() }
    );

    impl_no_virt!(
        data_member_location,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_data_member_location).cloned() }
    );

    impl_no_virt!(encoding, Option<gimli::DwAte>, |_, die: GimliDie| {
        die.attr(DW_AT_encoding).and_then(|attr| {
            if let AttributeValue::Encoding(enc) = attr.value() {
                Some(enc)
            } else {
                None
            }
        })
    });

    impl_no_virt!(
        lower_bound,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_lower_bound).cloned() }
    );

    impl_no_virt!(
        upper_bound,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_upper_bound).cloned() }
    );

    impl_no_virt!(
        count,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_count).cloned() }
    );

    impl_no_virt!(
        frame_base,
        Option<Attribute<EndianArcSlice>>,
        |_, die: GimliDie| { die.attr(DW_AT_frame_base).cloned() }
    );

    pub fn for_each_children_t<T>(&self, mut f: impl FnMut(Die<'a>) -> Option<T>) -> Option<T> {
        match self {
            Die::Virtual { .. } => unimplemented!(),
            Die::Dwarf { dcx, die } => {
                let mut tree = weak_error!(dcx.unit.entries_tree(Some(die.offset())))?;

                let root = weak_error!(tree.root())?;
                let mut children = root.children();
                while let Some(c) = weak_error!(children.next())? {
                    let die = Die::new(dcx.clone(), c.entry().offset()).expect("DIE should exist");

                    if let Some(r) = f(die) {
                        return Some(r);
                    }
                }

                None
            }
        }
    }

    pub fn for_each_children(&self, mut f: impl FnMut(Die)) {
        self.for_each_children_t::<()>(|die| {
            f(die);
            None
        });
    }

    pub fn for_each_children_filter_collect<T>(
        &self,
        mut f: impl FnMut(Die) -> Option<T>,
    ) -> Vec<T> {
        let mut result = vec![];
        self.for_each_children(|die| {
            if let Some(r) = f(die) {
                result.push(r);
            }
        });

        result
    }

    pub fn for_each_children_recursive_t<T>(
        &self,
        mut f: impl FnMut(Die<'a>) -> Option<T>,
    ) -> Option<T> {
        match self {
            Die::Virtual { .. } => unimplemented!(),
            Die::Dwarf { dcx, die } => {
                let mut queue = VecDeque::from([die.offset()]);

                while let Some(offset) = queue.pop_front() {
                    let mut tree = weak_error!(dcx.unit.entries_tree(Some(offset)))?;
                    let root = weak_error!(tree.root())?;
                    let mut children = root.children();
                    while let Some(child) = weak_error!(children.next())? {
                        let offset = child.entry().offset();
                        let die = Die::new(dcx.clone(), offset).expect("DIE should exist");

                        if let Some(r) = f(die) {
                            return Some(r);
                        }

                        queue.push_back(offset);
                    }
                }
                None
            }
        }
    }

    pub fn for_each_children_recursive(&self, mut f: impl FnMut(Die)) {
        self.for_each_children_recursive_t::<()>(|die| {
            f(die);
            None
        });
    }

    fn attr_to_string(
        dwarf: &Dwarf<EndianArcSlice>,
        unit: &gimli::Unit<EndianArcSlice, usize>,
        die: &DebuggingInformationEntry<EndianArcSlice, usize>,
        attr: DwAt,
    ) -> gimli::Result<Option<String>> {
        die.attr(attr)
            .and_then(|attr| dwarf.attr_string(unit, attr.value()).ok())
            .map(|l| l.to_string_lossy().map(|s| s.to_string()))
            .transpose()
    }
}
