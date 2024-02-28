import unittest
import pexpect
import re


class VariablesTestCase(unittest.TestCase):
    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bugstalker ./examples/target/debug/vars')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_read_scalar_variables(self):
        """Reading rust scalar values"""
        self.debugger.sendline('break vars.rs:30')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('30     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('int8 = i8(1)')
        self.debugger.expect_exact('int16 = i16(-1)')
        self.debugger.expect_exact('int32 = i32(2)')
        self.debugger.expect_exact('int64 = i64(-2)')
        self.debugger.expect_exact('int128 = i128(3)')
        self.debugger.expect_exact('isize = isize(-3)')
        self.debugger.expect_exact('uint8 = u8(1)')
        self.debugger.expect_exact('uint16 = u16(2)')
        self.debugger.expect_exact('uint32 = u32(3)')
        self.debugger.expect_exact('uint64 = u64(4)')
        self.debugger.expect_exact('uint128 = u128(5)')
        self.debugger.expect_exact('usize = usize(6)')
        self.debugger.expect_exact('f32 = f32(1.1)')
        self.debugger.expect_exact('f64 = f64(1.2)')
        self.debugger.expect_exact('boolean_true = bool(true)')
        self.debugger.expect_exact('boolean_false = bool(false)')
        self.debugger.expect_exact('char_ascii = char(a)')
        self.debugger.expect_exact('char_non_ascii = char(😊)'.encode('utf-8'))

    def test_read_scalar_variables_at_place(self):
        """Local variables reading only from the current lexical block"""
        self.debugger.sendline('break vars.rs:11')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('11     let int128 = 3_i128;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('int8 = i8(1)')
        self.debugger.expect_exact('int16 = i16(-1)')
        self.debugger.expect_exact('int32 = i32(2)')
        self.debugger.expect_exact('int64 = i64(-2)')
        with self.assertRaises(pexpect.exceptions.TIMEOUT):
            self.debugger.expect_exact('int128 = i128(3)', timeout=1)

    def test_read_struct(self):
        """Reading rust structs"""
        self.debugger.sendline('break vars.rs:53')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('53     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('tuple_0 = ()')

        self.debugger.expect_exact('tuple_1 = (f64, f64) {')
        self.debugger.expect_exact('0: f64(0)')
        self.debugger.expect_exact('1: f64(1.1)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('tuple_2 = (u64, i64, char, bool) {')
        self.debugger.expect_exact('0: u64(1)')
        self.debugger.expect_exact('1: i64(-1)')
        self.debugger.expect_exact('2: char(a)')
        self.debugger.expect_exact('3: bool(false)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('foo = Foo {')
        self.debugger.expect_exact('bar: i32(100)')
        self.debugger.expect_exact('baz: char(9)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('foo2 = Foo2 {')
        self.debugger.expect_exact('foo: Foo {')
        self.debugger.expect_exact('bar: i32(100)')
        self.debugger.expect_exact('baz: char(9)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('additional: bool(true)')
        self.debugger.expect_exact('}')

    def test_read_array(self):
        """Reading rust arrays"""
        self.debugger.sendline('break vars.rs:61')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('61     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(-1)')
        self.debugger.expect_exact('2: i32(2)')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('4: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('arr_2 = [[i32]] {')
        self.debugger.expect_exact('0: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(-1)')
        self.debugger.expect_exact('2: i32(2)')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('4: i32(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('1: [i32] {')
        self.debugger.expect_exact('0: i32(0)')
        self.debugger.expect_exact('1: i32(1)')
        self.debugger.expect_exact('2: i32(2)')
        self.debugger.expect_exact('3: i32(3)')
        self.debugger.expect_exact('4: i32(4)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('2: [i32] {')
        self.debugger.expect_exact('0: i32(0)')
        self.debugger.expect_exact('1: i32(-1)')
        self.debugger.expect_exact('2: i32(-2)')
        self.debugger.expect_exact('3: i32(-3)')
        self.debugger.expect_exact('4: i32(-4)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('}')

    def test_read_enum(self):
        """Reading rust enums"""
        self.debugger.sendline('break vars.rs:93')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('93     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('enum_1 = EnumA::B')

        self.debugger.expect_exact('enum_2 = EnumC::C {')
        self.debugger.expect_exact('0: char(b)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_3 = EnumC::D {')
        self.debugger.expect_exact('0: f64(1.1)')
        self.debugger.expect_exact('1: f32(1.2)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_4 = EnumC::E {')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_5 = EnumF::F {')
        self.debugger.expect_exact('0: EnumC::C {')
        self.debugger.expect_exact('0: char(f)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_6 = EnumF::G {')
        self.debugger.expect_exact('0: Foo {')
        self.debugger.expect_exact('a: i32(1)')
        self.debugger.expect_exact('b: char(1)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_7 = EnumF::J {')
        self.debugger.expect_exact('0: EnumA::A')
        self.debugger.expect_exact('}')

    def test_read_pointers(self):
        """Reading rust references and pointers"""
        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('119     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('a = i32(2)')
        self.debugger.expect(r'ref_a = &i32 \[0x[0-9A-F]{14}\]')
        self.debugger.expect(r'ptr_a = \*const i32 \[0x[0-9A-F]{14}\]')
        self.debugger.expect(r'ptr_ptr_a = \*const \*const i32 \[0x[0-9A-F]{'
                             r'14}\]')

        self.debugger.expect_exact('b = i32(2)')
        self.debugger.expect(r'mut_ref_b = &mut i32 \[0x[0-9A-F]{14}\]')

        self.debugger.expect_exact('c = i32(2)')
        self.debugger.expect(r'mut_ptr_c = \*mut i32 \[0x[0-9A-F]{14}\]')

        self.debugger.expect(r'box_d = alloc::boxed::Box<i32, '
                             r'alloc::alloc::Global> \[0x[0-9A-F]{14}\]')

        self.debugger.expect_exact('f = Foo {')
        self.debugger.expect_exact('bar: i32(1)')
        self.debugger.expect_exact('baz: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect(r'foo: &i32 \[0x[0-9A-F]{14}\]')
        self.debugger.expect_exact('}')

        self.debugger.expect(r'ref_f = &vars::references::Foo \[0x[0-9A-F]{'
                             r'14}\]')

    def test_deref_pointers(self):
        """Reading deref rust references and pointers"""
        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('119     let nop: Option<u8> = None;')

        self.debugger.sendline('var *ref_a')
        self.debugger.expect_exact('*ref_a = i32(2)')
        self.debugger.sendline('var *ptr_a')
        self.debugger.expect_exact('*ptr_a = i32(2)')
        self.debugger.sendline('var *ptr_ptr_a')
        self.debugger.expect(r'\*ptr_ptr_a = \*const i32 \[0x[0-9A-F]{14}\]')
        self.debugger.sendline('var **ptr_ptr_a')
        self.debugger.expect_exact('**ptr_ptr_a = i32(2)')
        self.debugger.sendline('var *mut_ref_b')
        self.debugger.expect_exact('*mut_ref_b = i32(2)')
        self.debugger.sendline('var *mut_ptr_c')
        self.debugger.expect_exact('*mut_ptr_c = i32(2)')
        self.debugger.sendline('var *box_d')
        self.debugger.expect_exact('*box_d = i32(2)')
        self.debugger.sendline('var *ref_f')
        self.debugger.expect_exact('*ref_f = Foo {')
        self.debugger.expect_exact('bar: i32(1)')
        self.debugger.expect_exact('baz: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect(r'foo: &i32 \[0x[0-9A-F]{14}\]')
        self.debugger.expect_exact('}')

    def test_read_type_alias(self):
        """Reading rust variables with type aliases"""
        self.debugger.sendline('break vars.rs:126')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('126     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('a_alias = i32(1)')

    def test_read_vec_and_slice(self):
        """Reading rust vectors and slices"""
        self.debugger.sendline('break vars.rs:151')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('151     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('vec1 = Vec<i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(3)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('vec2 = Vec<vars::vec_and_slice_types'
                                   '::Foo, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [Foo] {')
        self.debugger.expect_exact('0: Foo {')
        self.debugger.expect_exact('foo: i32(1)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('1: Foo {')
        self.debugger.expect_exact('foo: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(2)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('vec3 = Vec<alloc::vec::Vec<i32, '
                                   'alloc::alloc::Global>, '
                                   'alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [Vec<i32, alloc::alloc::Global>] {')
        self.debugger.expect_exact('0: Vec<i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('1: Vec<i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(2)')
        self.debugger.expect_exact('}')

        self.debugger.expect(r'slice1 = &\[i32; 3\] \[0x[0-9A-F]{14}\]')
        self.debugger.expect(r'slice2 = &\[&\[i32; 3\]; 2\] \[0x[0-9A-F]{14}\]')

        self.debugger.sendline('var *slice1')
        self.debugger.expect_exact('*slice1 = [i32]')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var *slice2')
        self.debugger.expect_exact('*slice2 = [&[i32; 3]] {')
        self.debugger.expect(r'0: &\[i32; 3\] \[0x[0-9A-F]{14}\]')
        self.debugger.expect(r'1: &\[i32; 3\] \[0x[0-9A-F]{14}\]')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var *(*slice2)[0]')
        self.debugger.expect_exact('*0 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var *(*slice2)[1]')
        self.debugger.expect_exact('*1 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

    def test_read_strings(self):
        """Reading rust strings and &str"""
        self.debugger.sendline('break vars.rs:159')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('159     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')
        self.debugger.expect_exact('s1 = String(hello world)')
        self.debugger.expect_exact('s2 = &str(hello world)')
        self.debugger.expect_exact('s3 = &str(hello world)')

    def test_read_static_variables(self):
        """Reading rust static's"""
        self.debugger.sendline('break vars.rs:168')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('168     let nop: Option<u8> = None;')

        self.debugger.sendline('var GLOB_1')
        self.debugger.expect_exact('vars::GLOB_1 = &str(glob_1)')
        self.debugger.sendline('var GLOB_2')
        self.debugger.expect_exact('vars::GLOB_2 = i32(2)')

    def test_read_static_variables_different_modules(self):
        """Reading rust static's from another module"""
        self.debugger.sendline('break vars.rs:179')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('179     let nop: Option<u8> = None;')

        self.debugger.sendline('var GLOB_3')
        self.debugger.expect(r'vars::(ns_1::)?GLOB_3')
        self.debugger.expect(r'vars::(ns_1::)?GLOB_3')

    def test_read_tls_variables(self):
        """Reading rust tls variables"""
        self.debugger.sendline('break vars.rs:194')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('194         let nop: Option<u8> = None;')

        self.debugger.sendline('var THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact(
            'vars::THREAD_LOCAL_VAR_1::__getit::__KEY = Cell<i32>(2)')
        self.debugger.sendline('var THREAD_LOCAL_VAR_2')
        self.debugger.expect_exact(
            'vars::THREAD_LOCAL_VAR_2::__getit::__KEY = Cell<&str>(2)')

        # assert uninit tls variables
        self.debugger.sendline('break vars.rs:199')
        self.debugger.expect('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('199         let nop: Option<u8> = None;')

        self.debugger.sendline('var THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact(
            'vars::THREAD_LOCAL_VAR_1::__getit::__KEY = Cell<i32>(uninit)')

        # assert tls variables changes in another thread
        self.debugger.sendline('break vars.rs:203')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('203     let nop: Option<u8> = None;')

        self.debugger.sendline('var THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact(
            'vars::THREAD_LOCAL_VAR_1::__getit::__KEY = Cell<i32>(1)')

    def test_custom_select(self):
        """Reading memory by select expressions"""
        self.debugger.sendline('break vars.rs:61')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('61     let nop: Option<u8> = None;')

        self.debugger.sendline('var arr_2[0][2]')
        self.debugger.expect_exact('2 = i32(2)')

        self.debugger.sendline('var arr_1[2..4]')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('2: i32(2)')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var arr_1[..]')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(-1)')
        self.debugger.expect_exact('2: i32(2)')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('4: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var arr_1[..2]')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(-1)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var arr_1[3..]')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('4: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('var arr_1[2..4][1..]')
        self.debugger.expect_exact('arr_1 = [i32] {')
        self.debugger.expect_exact('3: i32(-2)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('break vars.rs:93')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('93     let nop: Option<u8> = None;')

        self.debugger.sendline('var enum_6.0.a')
        self.debugger.expect_exact('a = i32(1)')

        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('119     let nop: Option<u8> = None;')

        self.debugger.sendline('var *((*ref_f).foo)')
        self.debugger.expect_exact('*foo = i32(2)')

        self.debugger.sendline('break vars.rs:261')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('261     let nop: Option<u8> = None;')

        self.debugger.sendline('var hm2.abc')
        self.debugger.expect_exact('abc = Vec<i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [i32] {')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('break vars.rs:394')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('394     let nop: Option<u8> = None;')

        self.debugger.sendline('var ptr[..4]')
        self.debugger.expect_exact('[*ptr] = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('3: i32(4)')
        self.debugger.expect_exact('}')

    def test_zst_types(self):
        """Read variables of zero sized types"""
        self.debugger.sendline('break vars.rs:430')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('430     let nop: Option<u8> = None;')

        self.debugger.sendline('var locals')

        self.debugger.expect_exact('ptr_zst = &()')

        self.debugger.expect_exact('array_zst = [()] {')
        self.debugger.expect_exact('0: ()(())')
        self.debugger.expect_exact('1: ()(())')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('vec_zst = Vec<(), alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [()] {')
        self.debugger.expect_exact('0: ()(())')
        self.debugger.expect_exact('1: ()(())')
        self.debugger.expect_exact('2: ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(0)')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('slice_zst = &[(); 4]')

        self.debugger.expect_exact('struct_zst = StructZst {')
        self.debugger.expect_exact('0: ()(())')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('enum_zst = Option<()>::Some {')
        self.debugger.expect_exact('0: ()(())')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('vecdeque_zst = VecDeque<(), alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [()] {')
        self.debugger.expect_exact('0: ()(())')
        self.debugger.expect_exact('1: ()(())')
        self.debugger.expect_exact('2: ()(())')
        self.debugger.expect_exact('3: ()(())')
        self.debugger.expect_exact('4: ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(0)')
        self.debugger.expect_exact('}')

        self.debugger.expect(re.compile(b"hash_map_zst_key = HashMap<\(\), i32, .*::RandomState> {"))
        self.debugger.expect_exact('()(()): i32(1)')
        self.debugger.expect_exact('}')
        self.debugger.expect(re.compile(b"hash_map_zst_val = HashMap<i32, \(\), .*::RandomState> {"))
        self.debugger.expect_exact('i32(1): ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect(re.compile(b"hash_map_zst = HashMap<\(\), \(\), .*::RandomState> {"))
        self.debugger.expect_exact('()(()): ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect(re.compile(b"hash_set_zst = HashSet<\(\), .*::RandomState> {"))
        self.debugger.expect_exact('()(())')
        self.debugger.expect_exact('}')

        self.debugger.expect_exact('btree_map_zst_key = BTreeMap<(), i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('()(()): i32(1)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('btree_map_zst_val = BTreeMap<i32, (), alloc::alloc::Global> {')
        self.debugger.expect_exact('i32(1): ()(())')
        self.debugger.expect_exact('i32(2): ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('btree_map_zst = BTreeMap<(), (), alloc::alloc::Global> {')
        self.debugger.expect_exact('()(()): ()(())')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('btree_set_zst = BTreeSet<(), alloc::alloc::Global> {')
        self.debugger.expect_exact('()(())')
        self.debugger.expect_exact('}')

    def test_read_arguments(self):
        """Reading rust tls variables"""
        self.debugger.sendline('break vars.rs:232')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('let nop: Option<u8> = None;')

        self.debugger.sendline('arg by_val')
        self.debugger.expect_exact('by_val = i32(1)')
        self.debugger.sendline('arg by_ref')
        self.debugger.expect(r'by_ref = &i32 \[0x[0-9A-F]{14}\]')
        self.debugger.sendline('arg vec')
        self.debugger.expect_exact('vec = Vec<u8, alloc::alloc::Global> {')
        self.debugger.sendline('arg box_arr')
        self.debugger.expect_exact('box_arr = alloc::boxed::Box<[u8], alloc::alloc::Global> {')

        self.debugger.sendline('arg all')
        self.debugger.expect_exact('by_val = i32(1)')
        self.debugger.expect_exact('by_ref = &i32')
        self.debugger.expect_exact('vec = Vec<u8, alloc::alloc::Global> {')
        self.debugger.expect_exact('box_arr = alloc::boxed::Box<[u8], alloc::alloc::Global> {')

    def test_ptr_cast(self):
        """Cast const address to a typed pointer"""
        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('let nop: Option<u8> = None;')

        self.debugger.sendline('var ref_a')

        addr = ""
        for x in range(10):
            line = self.debugger.readline().decode("utf-8")
            result = re.search(r'ref_a = &i32 \[0x(.*)\]', line)
            if result:
                addr = result.group(1)
                addr = "0x"+addr[:14]
                break

        self.debugger.sendline('var *(*const i32)'+addr)
        self.debugger.expect_exact('{unknown} = i32(2)')

