use crate::debugger;
use fallible_iterator::FallibleIterator;
use nix::unistd::Pid;

/// A bit mask which contains the result of a Match operation on a Group and allows iterating through them.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct BitMask(u16);

impl BitMask {
    const BITMASK_STRIDE: usize = 1;

    fn invert(self) -> Self {
        BitMask(self.0 ^ 0xffff_u16)
    }

    pub fn remove_lowest_bit(self) -> Self {
        BitMask(self.0 & (self.0 - 1))
    }

    pub fn lowest_set_bit(self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some(unsafe { self.lowest_set_bit_nonzero() })
        }
    }

    pub unsafe fn lowest_set_bit_nonzero(self) -> usize {
        self.trailing_zeros()
    }

    pub fn trailing_zeros(self) -> usize {
        self.0.trailing_zeros() as usize / Self::BITMASK_STRIDE
    }
}

struct GroupReflection([u8; 16]);

impl GroupReflection {
    fn width() -> usize {
        16
    }

    /// Load group of control bytes from debugee process.
    fn load(pid: Pid, ptr: *const u8) -> Result<Self, nix::Error> {
        let mut data: [u8; 16] = Default::default();
        data.copy_from_slice(&debugger::read_memory_by_pid(
            pid,
            ptr as usize,
            Self::width(),
        )?);
        Ok(Self(data))
    }

    /// Returns a BitMask indicating all bytes in the group which are EMPTY or DELETED.
    #[inline]
    pub fn match_empty_or_deleted(self) -> BitMask {
        let mut result = 0_u16;

        for (i, &b) in self.0.iter().enumerate() {
            let b = b as u16;
            let b = (b >> 7) << i;
            result |= b;
        }

        BitMask(result)
    }
}

/// Hashmap bucket.
pub(super) struct BucketReflection {
    size: usize,
    ptr: *const u8,
}

impl BucketReflection {
    fn new(ptr: *const u8, size: usize) -> Self {
        Self { size, ptr }
    }

    /// Move to next bucket by offset.
    fn next_n(&self, offset: usize) -> Self {
        unsafe {
            let p = self.ptr.sub(offset * self.size);
            Self {
                size: self.size,
                ptr: p,
            }
        }
    }

    /// Read `T` as raw bytes from a debugee process.
    pub(super) fn read(&self, pid: Pid) -> nix::Result<Vec<u8>> {
        debugger::read_memory_by_pid(pid, self.location(), self.size)
    }

    pub(super) fn location(&self) -> usize {
        unsafe { self.ptr.sub(self.size) as usize }
    }

    pub(super) fn size(&self) -> usize {
        self.size
    }
}

pub(super) struct HashmapReflection {
    bucket_mask: usize,
    kv_size: usize,
    crtl: *const u8,
}

impl HashmapReflection {
    pub(super) fn new(crtl_ptr: *const u8, bucket_mask: usize, kv_size: usize) -> Self {
        Self {
            bucket_mask,
            kv_size,
            crtl: crtl_ptr,
        }
    }

    fn data_end(&self) -> *const u8 {
        self.crtl
    }

    fn buckets(&self) -> usize {
        self.bucket_mask + 1
    }

    pub(super) fn iter(&self, pid: Pid) -> Result<BucketIterator, nix::Error> {
        unsafe {
            let ctrl = self.crtl;

            Ok(BucketIterator {
                data: BucketReflection::new(self.data_end(), self.kv_size),
                current_group: GroupReflection::load(pid, ctrl)?
                    .match_empty_or_deleted()
                    .invert(),
                end: ctrl.add(self.buckets()),
                next_ctrl: ctrl.add(GroupReflection::width()),
                pid,
            })
        }
    }
}

/// Iterator over hashbrown hashmap buckets.
pub(super) struct BucketIterator {
    data: BucketReflection,
    current_group: BitMask,
    next_ctrl: *const u8,
    end: *const u8,
    pid: Pid,
}

impl FallibleIterator for BucketIterator {
    type Item = BucketReflection;
    type Error = nix::Error;

    fn next(&mut self) -> Result<Option<Self::Item>, Self::Error> {
        unsafe {
            loop {
                if let Some(index) = self.current_group.lowest_set_bit() {
                    self.current_group = self.current_group.remove_lowest_bit();
                    return Ok(Some(self.data.next_n(index)));
                }

                if self.next_ctrl >= self.end {
                    return Ok(None);
                }

                self.current_group = GroupReflection::load(self.pid, self.next_ctrl)?
                    .match_empty_or_deleted()
                    .invert();
                self.data = self.data.next_n(GroupReflection::width());
                self.next_ctrl = self.next_ctrl.add(GroupReflection::width());
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_group_match_empty_or_deleted() {
        for _ in 0..100 {
            let mut data = [0u8; 16];
            for i in &mut data {
                *i = rand::random::<u8>();
            }

            let group = GroupReflection(data);
            let mask = group.match_empty_or_deleted();

            let expected = unsafe {
                let vec = core::arch::x86_64::_mm_loadu_si128(data.as_ptr() as *const _);
                BitMask(core::arch::x86_64::_mm_movemask_epi8(vec) as u16)
            };

            assert_eq!(mask, expected);
        }
    }
}
