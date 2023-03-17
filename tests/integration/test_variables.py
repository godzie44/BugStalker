import unittest
import pexpect
import re


class VariablesTestCase(unittest.TestCase):
    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/vars')
        debugger.expect('No previous history.')
        self.debugger = debugger

    def test_read_scalar_variables(self):
        """Reading rust scalar values"""
        self.debugger.sendline('break vars.rs:30')
        self.debugger.expect('break vars.rs:30')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
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
        """Local variables reading only from current lexical block"""
        self.debugger.sendline('break vars.rs:11')
        self.debugger.expect('break vars.rs:11')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let int128 = 3_i128;')

        self.debugger.sendline('vars')
        self.debugger.expect_exact('int8 = i8(1)')
        self.debugger.expect_exact('int16 = i16(-1)')
        self.debugger.expect_exact('int32 = i32(2)')
        self.debugger.expect_exact('int64 = i64(-2)')
        with self.assertRaises(pexpect.exceptions.TIMEOUT):
            self.debugger.expect_exact('int128 = i128(3)', timeout=1)

    def test_read_struct(self):
        """Reading rust structs"""
        self.debugger.sendline('break vars.rs:53')
        self.debugger.expect('break vars.rs:53')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
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
        self.debugger.expect('break vars.rs:61')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
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
        self.debugger.expect('break vars.rs:93')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
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
        self.debugger.expect('break vars.rs:119')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
        self.debugger.expect_exact('a = i32(2)')
        self.debugger.expect(r'ref_a = &i32 \[0x[0-9a-f]{12}\]')
        self.debugger.expect(r'ptr_a = \*const i32 \[0x[0-9a-f]{12}\]')
        self.debugger.expect(r'ptr_ptr_a = \*const \*const i32 \[0x[0-9a-f]{'
                             r'12}\]')

        self.debugger.expect_exact('b = i32(2)')
        self.debugger.expect(r'mut_ref_b = &mut i32 \[0x[0-9a-f]{12}\]')

        self.debugger.expect_exact('c = i32(2)')
        self.debugger.expect(r'mut_ptr_c = \*mut i32 \[0x[0-9a-f]{12}\]')

        self.debugger.expect(r'box_d = alloc::boxed::Box<i32, '
                             r'alloc::alloc::Global> \[0x[0-9a-f]{12}\]')

        self.debugger.expect_exact('f = Foo {')
        self.debugger.expect_exact('bar: i32(1)')
        self.debugger.expect_exact('baz: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect(r'foo: &i32 \[0x[0-9a-f]{12}\]')
        self.debugger.expect_exact('}')

        self.debugger.expect(r'ref_f = &vars::references::Foo \[0x[0-9a-f]{'
                             r'12}\]')

    def test_deref_pointers(self):
        """Reading deref rust references and pointers"""
        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect('break vars.rs:119')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars *ref_a')
        self.debugger.expect_exact('*ref_a = i32(2)')
        self.debugger.sendline('vars *ptr_a')
        self.debugger.expect_exact('*ptr_a = i32(2)')
        self.debugger.sendline('vars *ptr_ptr_a')
        self.debugger.expect(r'\*ptr_ptr_a = \*const i32 \[0x[0-9a-f]{12}\]')
        self.debugger.sendline('vars **ptr_ptr_a')
        self.debugger.expect_exact('**ptr_ptr_a = i32(2)')
        self.debugger.sendline('vars *mut_ref_b')
        self.debugger.expect_exact('*mut_ref_b = i32(2)')
        self.debugger.sendline('vars *mut_ptr_c')
        self.debugger.expect_exact('*mut_ptr_c = i32(2)')
        self.debugger.sendline('vars *box_d')
        self.debugger.expect_exact('*box_d = i32(2)')
        self.debugger.sendline('vars *ref_f')
        self.debugger.expect_exact('*ref_f = Foo {')
        self.debugger.expect_exact('bar: i32(1)')
        self.debugger.expect_exact('baz: [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('}')
        self.debugger.expect(r'foo: &i32 \[0x[0-9a-f]{12}\]')
        self.debugger.expect_exact('}')

    def test_read_type_alias(self):
        """Reading rust variables with type aliases"""
        self.debugger.sendline('break vars.rs:126')
        self.debugger.expect('break vars.rs:126')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
        self.debugger.expect_exact('a_alias = i32(1)')

    def test_read_vec_and_slice(self):
        """Reading rust vectors and slices"""
        self.debugger.sendline('break vars.rs:151')
        self.debugger.expect('break vars.rs:151')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
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

        self.debugger.expect(r'slice1 = &\[i32; 3\] \[0x[0-9a-f]{12}\]')
        self.debugger.expect(r'slice2 = &\[&\[i32; 3\]; 2\] \[0x[0-9a-f]{12}\]')

        self.debugger.sendline('vars *slice1')
        self.debugger.expect_exact('*slice1 = [i32]')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('vars *slice2')
        self.debugger.expect_exact('*slice2 = [&[i32; 3]] {')
        self.debugger.expect(r'0: &\[i32; 3\] \[0x[0-9a-f]{12}\]')
        self.debugger.expect(r'1: &\[i32; 3\] \[0x[0-9a-f]{12}\]')
        self.debugger.expect_exact('}')

        self.debugger.sendline('vars *(*slice2)[0]')
        self.debugger.expect_exact('*0 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('vars *(*slice2)[1]')
        self.debugger.expect_exact('*1 = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')

    def test_read_strings(self):
        """Reading rust strings and &str"""
        self.debugger.sendline('break vars.rs:159')
        self.debugger.expect('break vars.rs:159')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars')
        self.debugger.expect_exact('s1 = String(hello world)')
        self.debugger.expect_exact('s2 = &str(hello world)')
        self.debugger.expect_exact('s3 = &str(hello world)')

    def test_read_static_variables(self):
        """Reading rust static's"""
        self.debugger.sendline('break vars.rs:168')
        self.debugger.expect('break vars.rs:168')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars GLOB_1')
        self.debugger.expect_exact('GLOB_1 = &str(glob_1)')
        self.debugger.sendline('vars GLOB_2')
        self.debugger.expect_exact('GLOB_2 = i32(2)')

    def test_read_static_variables_different_modules(self):
        """Reading rust static's from another module"""
        self.debugger.sendline('break vars.rs:179')
        self.debugger.expect('break vars.rs:179')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars GLOB_3')
        self.debugger.expect_exact('GLOB_3')
        self.debugger.expect_exact('GLOB_3')

    def test_read_tls_variables(self):
        """Reading rust tls variables"""
        self.debugger.sendline('break vars.rs:194')
        self.debugger.expect('break vars.rs:194')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>        let nop: Option<u8> = None;')

        self.debugger.sendline('vars THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact('THREAD_LOCAL_VAR_1 = Cell<i32>(2)')
        self.debugger.sendline('vars THREAD_LOCAL_VAR_2')
        self.debugger.expect_exact('THREAD_LOCAL_VAR_2 = Cell<&str>(2)')

        # assert uninit tls variables
        self.debugger.sendline('break vars.rs:199')
        self.debugger.expect('break vars.rs:199')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>        let nop: Option<u8> = None;')

        self.debugger.sendline('vars THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact('THREAD_LOCAL_VAR_1 = Cell<i32>(uninit)')

        # assert tls variables changes in another thread
        self.debugger.sendline('break vars.rs:203')
        self.debugger.expect('break vars.rs:203')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars THREAD_LOCAL_VAR_1')
        self.debugger.expect_exact('THREAD_LOCAL_VAR_1 = Cell<i32>(1)')

    def test_custom_select(self):
        """Reading memory by select expressions"""
        self.debugger.sendline('break vars.rs:61')
        self.debugger.expect_exact('break vars.rs:61')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars arr_2[0][2]')
        self.debugger.expect_exact('2 = i32(2)')

        self.debugger.sendline('break vars.rs:93')
        self.debugger.expect_exact('break vars.rs:93')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars enum_6.0.a')
        self.debugger.expect_exact('a = i32(1)')

        self.debugger.sendline('break vars.rs:119')
        self.debugger.expect_exact('break vars.rs:119')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars *((*ref_f).foo)')
        self.debugger.expect_exact('*foo = i32(2)')

        self.debugger.sendline('break vars.rs:256')
        self.debugger.expect_exact('break vars.rs:256')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars hm2.abc')
        # todo fix '1 = ... '
        self.debugger.expect_exact('1 = Vec<i32, alloc::alloc::Global> {')
        self.debugger.expect_exact('buf: [i32] {')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('}')
        self.debugger.expect_exact('cap: usize(3)')
        self.debugger.expect_exact('}')

        self.debugger.sendline('break vars.rs:389')
        self.debugger.expect_exact('break vars.rs:389')
        self.debugger.sendline('continue')
        self.debugger.expect_exact('>    let nop: Option<u8> = None;')

        self.debugger.sendline('vars ptr[..4]')
        self.debugger.expect_exact('[*ptr] = [i32] {')
        self.debugger.expect_exact('0: i32(1)')
        self.debugger.expect_exact('1: i32(2)')
        self.debugger.expect_exact('2: i32(3)')
        self.debugger.expect_exact('3: i32(4)')
        self.debugger.expect_exact('}')
