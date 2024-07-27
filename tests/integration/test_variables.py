import unittest
import pexpect
from helper import Debugger


class VariablesTestCase(unittest.TestCase):
    def setUp(self):
        self.debugger = Debugger('./examples/target/debug/vars')

    def test_read_scalar_variables(self):
        """Reading rust scalar values"""
        self.debugger.cmd('break vars.rs:30', 'New breakpoint')
        self.debugger.cmd('run', '30     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',
            'int8 = i8(1)',
            'int16 = i16(-1)',
            'int32 = i32(2)',
            'int64 = i64(-2)',
            'int128 = i128(3)',
            'isize = isize(-3)',
            'uint8 = u8(1)',
            'uint16 = u16(2)',
            'uint32 = u32(3)',
            'uint64 = u64(4)',
            'uint128 = u128(5)',
            'usize = usize(6)',
            'f32 = f32(1.1)',
            'f64 = f64(1.2)',
            'boolean_true = bool(true)',
            'boolean_false = bool(false)',
            'char_ascii = char(a)',
            'char_non_ascii = char(😊)'.encode('utf-8'),
        )

    def test_read_scalar_variables_at_place(self):
        """Local variables reading only from the current lexical block"""
        self.debugger.cmd('break vars.rs:11', 'New breakpoint')
        self.debugger.cmd('run', '11     let int128 = 3_i128;')
        self.debugger.cmd(
            'var locals',
            'int8 = i8(1)',
            'int16 = i16(-1)',
            'int64 = i64(-2)',
        )
        with self.assertRaises(pexpect.exceptions.TIMEOUT):
            self.debugger.expect_in_output('int128 = i128(3)', timeout=1)

    def test_read_struct(self):
        """Reading rust structs"""
        self.debugger.cmd('break vars.rs:53', 'New breakpoint')
        self.debugger.cmd('run', '53     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',
            'tuple_0 = ()',

            'tuple_1 = (f64, f64) {',
            '0: f64(0)',
            '1: f64(1.1)',
            '}',

            'tuple_2 = (u64, i64, char, bool) {',
            '0: u64(1)',
            '1: i64(-1)',
            '2: char(a)',
            '3: bool(false)',
            '}',

            'foo = Foo {',
            'bar: i32(100)',
            'baz: char(9)',
            '}',

            'foo2 = Foo2 {',
            'foo: Foo {',
            'bar: i32(100)',
            'baz: char(9)',
            '}',
            'additional: bool(true)',
            '}',
        )

    def test_read_array(self):
        """Reading rust arrays"""
        self.debugger.cmd('break vars.rs:61', 'New breakpoint')
        self.debugger.cmd('run', '61     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',

            'arr_1 = [i32] {',
            '0: i32(1)',
            '1: i32(-1)',
            '2: i32(2)',
            '3: i32(-2)',
            '4: i32(3)',
            '}',

            'arr_2 = [[i32]] {',
            '0: [i32] {',
            '0: i32(1)',
            '1: i32(-1)',
            '2: i32(2)',
            '3: i32(-2)',
            '4: i32(3)',
            '}',
            '1: [i32] {',
            '0: i32(0)',
            '1: i32(1)',
            '2: i32(2)',
            '3: i32(3)',
            '4: i32(4)',
            '}',
            '2: [i32] {',
            '0: i32(0)',
            '1: i32(-1)',
            '2: i32(-2)',
            '3: i32(-3)',
            '4: i32(-4)',
            '}',
            '}',
        )

    def test_read_enum(self):
        """Reading rust enums"""
        self.debugger.cmd('break vars.rs:93', 'New breakpoint')
        self.debugger.cmd('run', '93     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',

            'enum_1 = EnumA::B',

            'enum_2 = EnumC::C {',
            '0: char(b)',
            '}',

            'enum_3 = EnumC::D {',
            '0: f64(1.1)',
            '1: f32(1.2)',
            '}',

            'enum_4 = EnumC::E {',
            '}',

            'enum_5 = EnumF::F {',
            '0: EnumC::C {',
            '0: char(f)',
            '}',
            '}',

            'enum_6 = EnumF::G {',
            '0: Foo {',
            'a: i32(1)',
            'b: char(1)',
            '}',
            '}',

            'enum_7 = EnumF::J {',
            '0: EnumA::A',
            '}',
        )

    def test_read_pointers(self):
        """Reading rust references and pointers"""
        self.debugger.cmd('break vars.rs:119', 'New breakpoint')
        self.debugger.cmd('run', '119     let nop: Option<u8> = None;')
        self.debugger.cmd_re(
            'var locals',
            r'a = i32\(2\)',
            r'ref_a = &i32 \[0x[0-9A-F]{14}\]',
            r'ptr_a = \*const i32 \[0x[0-9A-F]{14}\]',
            r'ptr_ptr_a = \*const \*const i32 \[0x[0-9A-F]{14}\]',
            r'b = i32\(2\)',
            r'mut_ref_b = &mut i32 \[0x[0-9A-F]{14}\]',
            r'c = i32\(2\)',
            r'mut_ptr_c = \*mut i32 \[0x[0-9A-F]{14}\]',
            r'box_d = alloc::boxed::Box<i32, alloc::alloc::Global> \[0x[0-9A-F]{14}\]',
            r'f = Foo {',
            r'bar: i32\(1\)',
            r'baz: \[i32\] {',
            r'0: i32\(1\)',
            r'1: i32\(2\)',
            r'}',
            r'foo: &i32 \[0x[0-9A-F]{14}\]',
            r'}',
            r'ref_f = &vars::references::Foo \[0x[0-9A-F]{14}\]'
        )

    def test_deref_pointers(self):
        """Reading deref rust references and pointers"""
        self.debugger.cmd('break vars.rs:119', 'New breakpoint')
        self.debugger.cmd('run', '119     let nop: Option<u8> = None;')

        self.debugger.cmd('var *ref_a', '*ref_a = i32(2)')
        self.debugger.cmd('var *ptr_a', '*ptr_a = i32(2)')
        self.debugger.cmd_re('var *ptr_ptr_a', r'\*ptr_ptr_a = \*const i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var **ptr_ptr_a', '**ptr_ptr_a = i32(2)')
        self.debugger.cmd('var *mut_ref_b', '*mut_ref_b = i32(2)')
        self.debugger.cmd('var *mut_ptr_c', '*mut_ptr_c = i32(2)')
        self.debugger.cmd('var *box_d', '*box_d = i32(2)')
        self.debugger.cmd_re(
            'var *ref_f',
            r'\*ref_f = Foo {',
            r'baz: \[i32\] {',
            r'0: i32\(1\)',
            r'1: i32\(2\)',
            r'}',
            r'foo: &i32 \[0x[0-9A-F]{14}\]',
            r'}',
        )

    def test_read_type_alias(self):
        """Reading rust variables with type aliases"""
        self.debugger.cmd('break vars.rs:126', 'New breakpoint')
        self.debugger.cmd('run', '126     let nop: Option<u8> = None;')
        self.debugger.cmd('var locals', 'a_alias = i32(1)')

    def test_read_vec_and_slice(self):
        """Reading rust vectors and slices"""
        self.debugger.cmd('break vars.rs:151', 'New breakpoint')
        self.debugger.cmd('run', '151     let nop: Option<u8> = None;')
        self.debugger.cmd_re(
            'var locals',

            r'vec1 = Vec<i32, alloc::alloc::Global> {',
            r'buf: \[i32\] {',
            r'0: i32\(1\)',
            r'1: i32\(2\)',
            r'2: i32\(3\)',
            r'}',
            r'cap: usize\(3\)',
            r'}',

            r'vec2 = Vec<vars::vec_and_slice_types::Foo, alloc::alloc::Global> {',
            r'buf: \[Foo\] {',
            r'0: Foo {',
            r'foo: i32\(1\)',
            r'}',
            r'1: Foo {',
            r'foo: i32\(2\)',
            r'}',
            r'}',
            r'cap: usize\(2\)',
            r'}',

            r'vec3 = Vec<alloc::vec::Vec<i32, alloc::alloc::Global>, alloc::alloc::Global> {',
            r'buf: \[Vec<i32, alloc::alloc::Global>\] {',
            r'0: Vec<i32, alloc::alloc::Global> {',
            r'buf: \[i32\] {',
            r'0: i32\(1\)',
            r'1: i32\(2\)',
            r'}',
            r'cap: usize\(3\)',
            r'}',
            r'1: Vec<i32, alloc::alloc::Global> {',
            r'buf: \[i32\] {',
            r'0: i32\(1\)',
            r'1: i32\(2\)',
            r'2: i32\(3\)',
            r'}',
            r'cap: usize\(3\)',
            r'}',
            r'}',
            r'cap: usize\(2\)',
            r'}',

            r'slice1 = &\[i32; 3\] \[0x[0-9A-F]{14}\]',
            r'slice2 = &\[&\[i32; 3\]; 2\] \[0x[0-9A-F]{14}\]',
        )

        self.debugger.cmd(
            'var *slice1',
            '*slice1 = [i32]',
            '0: i32(1)',
            '1: i32(2)',
            '2: i32(3)',
            '}',
        )

        self.debugger.cmd_re(
            'var *slice2',
            r'\*slice2 = \[&\[i32; 3\]\] {',
            r'0: &\[i32; 3\] \[0x[0-9A-F]{14}\]',
            r'1: &\[i32; 3\] \[0x[0-9A-F]{14}\]',
            '}',
        )

        self.debugger.cmd(
            'var *(*slice2)[0]',
            '*0 = [i32] {',
            '0: i32(1)',
            '1: i32(2)',
            '2: i32(3)',
            '}',
        )

        self.debugger.cmd(
            'var *(*slice2)[1]',
            '*1 = [i32] {',
            '0: i32(1)',
            '1: i32(2)',
            '2: i32(3)',
            '}',
        )

    def test_read_strings(self):
        """Reading rust strings and &str"""
        self.debugger.cmd('break vars.rs:159', 'New breakpoint')
        self.debugger.cmd('run', '159     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',
            's1 = String(hello world)',
            's2 = &str(hello world)',
            's3 = &str(hello world)',
        )

    def test_read_static_variables(self):
        """Reading rust static's"""
        self.debugger.cmd('break vars.rs:168', 'New breakpoint')
        self.debugger.cmd('run', '168     let nop: Option<u8> = None;')

        self.debugger.cmd('var GLOB_1', 'vars::GLOB_1 = &str(glob_1)')
        self.debugger.cmd('var GLOB_2', 'vars::GLOB_2 = i32(2)')

    def test_read_static_variables_different_modules(self):
        """Reading rust static's from another module"""
        self.debugger.cmd('break vars.rs:179', 'New breakpoint')
        self.debugger.cmd('run', '179     let nop: Option<u8> = None;')
        self.debugger.cmd_re(
            'var GLOB_3',
            r'vars::(ns_1::)?GLOB_3',
            r'vars::(ns_1::)?GLOB_3',
        )

    def test_read_tls_variables(self):
        """Reading rust tls variables"""
        self.debugger.cmd('break vars.rs:194', 'New breakpoint')
        self.debugger.cmd('run', '194         let nop: Option<u8> = None;')
        self.debugger.cmd('var THREAD_LOCAL_VAR_1', '::VAL = Cell<i32>(2)')
        self.debugger.cmd('var THREAD_LOCAL_VAR_2', '::VAL = Cell<&str>(2)')
        # assert uninit tls variables
        self.debugger.cmd('break vars.rs:199', 'New breakpoint')
        self.debugger.cmd('continue', '199         let nop: Option<u8> = None;')
        self.debugger.cmd('var THREAD_LOCAL_VAR_1')
        # assert tls variables changes in another thread
        self.debugger.cmd('break vars.rs:203', 'New breakpoint')
        self.debugger.cmd('continue', '203     let nop: Option<u8> = None;')
        self.debugger.cmd('var THREAD_LOCAL_VAR_1', '::VAL = Cell<i32>(1)')

    def test_custom_select(self):
        """Reading memory by select expressions"""
        self.debugger.cmd('break vars.rs:61', 'New breakpoint')
        self.debugger.cmd('run', '61     let nop: Option<u8> = None;')
        self.debugger.cmd('var arr_2[0][2]', '2 = i32(2)')
        self.debugger.cmd(
            'var arr_1[2..4]',
            'arr_1 = [i32] {',
            '2: i32(2)',
            '3: i32(-2)',
            '}',
        )
        self.debugger.cmd(
            'var arr_1[..]',
            'arr_1 = [i32] {',
            '0: i32(1)',
            '1: i32(-1)',
            '2: i32(2)',
            '3: i32(-2)',
            '4: i32(3)',
            '}',
        )
        self.debugger.cmd(
            'var arr_1[..2]',
            'arr_1 = [i32] {',
            '0: i32(1)',
            '1: i32(-1)',
            '}',
        )
        self.debugger.cmd(
            'var arr_1[3..]',
            'arr_1 = [i32] {',
            '3: i32(-2)',
            '4: i32(3)',
            '}',
        )
        self.debugger.cmd(
            'var arr_1[4..6]',
            'arr_1 = [i32] {',
            '4: i32(3)',
            '}',
        )
        self.debugger.cmd(
            'var arr_1[2..4][1..]',
            'arr_1 = [i32] {',
            '3: i32(-2)',
            '}',
        )

        self.debugger.cmd('break vars.rs:93', 'New breakpoint')
        self.debugger.cmd('continue', '93     let nop: Option<u8> = None;')
        self.debugger.cmd('var enum_6.0.a', 'a = i32(1)')

        self.debugger.cmd('break vars.rs:119', 'New breakpoint')
        self.debugger.cmd('continue', '119     let nop: Option<u8> = None;')
        self.debugger.cmd('var *((*ref_f).foo)', '*foo = i32(2)')

        self.debugger.cmd('break vars.rs:290', 'New breakpoint')
        self.debugger.cmd('continue', '290     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var hm2.abc',
            'abc = Vec<i32, alloc::alloc::Global> {',
            'buf: [i32] {',
            '1: i32(2)',
            '2: i32(3)',
            '}',
            'cap: usize(3)',
            '}',
        )
        self.debugger.cmd('var hm1[false]', 'value = i64(5)')
        self.debugger.cmd('var hm2["abc"]', 'value = Vec<i32, alloc::alloc::Global> {')
        self.debugger.cmd('var hm3[55]', 'value = i32(55)')
        self.debugger.cmd('var hm4["1"][1]', 'value = i32(1)')

        self.debugger.cmd('var a')
        addr = self.debugger.search_in_output(r'a = &i32 \[(.*)\]')
        self.debugger.cmd(f'var hm5[{addr}]', 'value = &str(a)')

        self.debugger.cmd('break vars.rs:307', 'New breakpoint')
        self.debugger.cmd('continue', '307     let nop: Option<u8> = None;')
        self.debugger.cmd('var hs1[1]', 'contains = bool(true)')
        self.debugger.cmd('var hs2[22]', 'contains = bool(true)')
        self.debugger.cmd('var hs2[222]', 'contains = bool(false)')

        self.debugger.cmd('var b')
        addr = self.debugger.search_in_output(r'b = &i32 \[(.*)\]')
        self.debugger.cmd(f'var hs4[{addr}]', 'contains = bool(true)')
        self.debugger.cmd('var hs4[0x000]', 'contains = bool(false)')

        self.debugger.cmd('break vars.rs:460', 'New breakpoint')
        self.debugger.cmd('continue', '460     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var ptr[..4]',
            '[*ptr] = [i32] {',
            '0: i32(1)',
            '1: i32(2)',
            '2: i32(3)',
            '3: i32(4)',
            '}',
        )

    def test_zst_types(self):
        """Read variables of zero sized types"""
        self.debugger.cmd('break vars.rs:496', 'New breakpoint')
        self.debugger.cmd('run', '496     let nop: Option<u8> = None;')
        self.debugger.cmd(
            'var locals',

            'ptr_zst = &()',

            'array_zst = [()] {',
            '0: ()(())',
            '1: ()(())',
            '}',

            'vec_zst = Vec<(), alloc::alloc::Global> {',
            'buf: [()] {',
            '0: ()(())',
            '1: ()(())',
            '2: ()(())',
            '}',
            'cap: usize(0)',
            '}',

            'slice_zst = &[(); 4]',

            'struct_zst = StructZst {',
            '0: ()(())',
            '}',

            'enum_zst = Option<()>::Some {',
            '0: ()(())',
            '}',

            'vecdeque_zst = VecDeque<(), alloc::alloc::Global> {',
            'buf: [()] {',
            '0: ()(())',
            '1: ()(())',
            '2: ()(())',
            '3: ()(())',
            '4: ()(())',
            '}',
            'cap: usize(0)',
            '}',

            'hash_map_zst_key = HashMap',
            '()(()): i32(1)',
            '}',

            'hash_map_zst_val = HashMap',
            'i32(1): ()(())',
            '}',

            'hash_map_zst = HashMap',
            '()(()): ()(())',
            '}',

            'hash_set_zst = HashSet',
            '()(())',
            '}',

            'btree_map_zst_key = BTreeMap<(), i32, alloc::alloc::Global> {',
            '()(()): i32(1)',
            '}',

            'btree_map_zst_val = BTreeMap<i32, (), alloc::alloc::Global> {',
            'i32(1): ()(())',
            'i32(2): ()(())',
            '}',

            'btree_map_zst = BTreeMap<(), (), alloc::alloc::Global> {',
            '()(()): ()(())',
            '}',

            'btree_set_zst = BTreeSet<(), alloc::alloc::Global> {',
            '()(())',
            '}',
        )

    def test_read_arguments(self):
        """Reading rust tls variables"""
        self.debugger.cmd('break vars.rs:232', 'New breakpoint')
        self.debugger.cmd('run', 'let nop: Option<u8> = None;')
        self.debugger.cmd('arg by_val', 'by_val = i32(1)')
        self.debugger.cmd_re('arg by_ref', r'by_ref = &i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('arg vec', 'vec = Vec<u8, alloc::alloc::Global> {')
        self.debugger.cmd('arg box_arr', 'box_arr = alloc::boxed::Box<[u8], alloc::alloc::Global> {')
        self.debugger.cmd(
            'arg all',
            'by_val = i32(1)',
            'by_ref = &i32',
            'vec = Vec<u8, alloc::alloc::Global> {',
            'box_arr = alloc::boxed::Box<[u8], alloc::alloc::Global> {',
        )

    def test_ptr_cast(self):
        """Cast const address to a typed pointer"""
        self.debugger.cmd('break vars.rs:119', 'New breakpoint')
        self.debugger.cmd('run', 'let nop: Option<u8> = None;')
        self.debugger.cmd('var ref_a')
        addr = self.debugger.search_in_output(r'ref_a = &i32 \[0x(.*)\]')
        addr = "0x" + addr[:14]
        self.debugger.cmd(f'var *(*const i32){addr}', '{unknown} = i32(2)')

    def test_address(self):
        """Test address operator"""
        self.debugger.cmd('break vars.rs:30', 'New breakpoint')
        self.debugger.cmd('run', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &int8', r'&i8 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&int8', 'i8(1)')

        self.debugger.cmd('break vars.rs:53', 'New breakpoint')
        self.debugger.cmd('continue', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &tuple_1', r'&\(f64, f64\) \[0x[0-9A-F]{14}\]')
        self.debugger.cmd(
            'var *&tuple_1',
            '(f64, f64) {',
            '0: f64(0)',
            '1: f64(1.1)',
            '}',
        )
        self.debugger.cmd_re('var &tuple_1.0', r'&f64 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&tuple_1.0', 'f64(0)')
        self.debugger.cmd_re('var &foo2.foo.bar', r'&i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&foo2.foo.bar', 'i32(100)')

        self.debugger.cmd('break vars.rs:61', 'New breakpoint')
        self.debugger.cmd('continue', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &arr_1', r'&\[i32\] \[0x[0-9A-F]{14}\]')
        self.debugger.cmd(
            'var *&arr_1',
            '[i32] {',
            '0: i32(1)',
            '1: i32(-1)',
        )
        self.debugger.cmd_re('var &arr_1[3]', r'&i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&arr_1[3]', 'i32(-2)')

        self.debugger.cmd('break vars.rs:119', 'New breakpoint')
        self.debugger.cmd('continue', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &ref_a', r'&&i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd_re('var *&ref_a', r'&i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var **&ref_a', 'i32(2)')

        self.debugger.cmd('break vars.rs:168', 'New breakpoint')
        self.debugger.cmd('continue', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &GLOB_1', r'&&str \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&GLOB_1', '&str(glob_1)')

        self.debugger.cmd('break vars.rs:290', 'New breakpoint')
        self.debugger.cmd('continue', 'let nop: Option<u8> = None;')
        self.debugger.cmd_re('var &hm6[{field_1: 1, field_2: *, field_3: *}]', r'&i32 \[0x[0-9A-F]{14}\]')
        self.debugger.cmd('var *&hm6[{field_1: 1, field_2: *, field_3: *}]', 'i32(1)')
