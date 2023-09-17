import unittest
import pexpect


class SharedLibTestCase(unittest.TestCase):
    """Test a debugger on project with shared libraries dependencies"""
    def setUp(self):
        debugger = pexpect.spawn(
                './target/debug/bugstalker ./examples/target/debug/calc_bin')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_lib_info(self):
        """View information about loaded shared libraries"""
        self.debugger.sendline('sharedlib info')
        # assert that main executable showing
        self.debugger.expect_exact('??? ./examples/target/debug/calc_bin')
        # assert that shared lib showing
        self.debugger.expect_exact('??? ./examples/target/debug/libcalc_lib.so')

        self.debugger.sendline('break main.rs:7')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')

        self.debugger.sendline('sharedlib info')
        # assert that main executable showing with mapping address
        self.debugger.expect(r'0x.*\.\/examples\/target\/debug\/calc_bin')
        # assert that shared lib showing with mapping address
        self.debugger.expect(r'0x.*\.\/examples\/target\/debug\/libcalc_lib\.so')

    def test_lib_step(self):
        """Do steps in shared library code"""
        self.debugger.sendline('break main.rs:7')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.expect_exact('let sum_1_2 = unsafe { calc_add(1, 2) }')

        self.debugger.sendline('step')
        self.debugger.expect_exact('lib.rs:3')
        self.debugger.expect_exact('3     a + b')

        self.debugger.sendline('step')
        self.debugger.expect_exact('4 }')

        self.debugger.sendline('step')
        self.debugger.expect_exact(r'main.rs:8')
        self.debugger.expect_exact('8     let sub_2_1 = unsafe { calc_sub(2, 1) };')

    def test_lib_fn_breakpoint(self):
        """Set breakpoint at shared library function"""
        self.debugger.sendline('break calc_add')
        self.debugger.expect_exact('New breakpoint 1')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.expect_exact('3     a + b')

    def test_lib_line_breakpoint(self):
        """Set breakpoint at line in shared library source code"""
        self.debugger.sendline('b lib.rs:8')
        self.debugger.expect('New breakpoint 1')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.expect_exact('8     a - b')

    def test_dynamic_load_lib_info(self):
        """View information about shared libraries loaded dynamically"""
        self.debugger.sendline('b main.rs:8')
        self.debugger.expect('New breakpoint 1')
        self.debugger.sendline('b main.rs:14')
        self.debugger.expect('New breakpoint 2')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('sharedlib info')

        try:
            self.debugger.expect_exact('./examples/target/debug/libprinter_lib.so')
        except pexpect.ExceptionPexpect:
            self.debugger.sendline('continue')
        else:
            raise pexpect.ExceptionPexpect("lib is not loading at this point")

        self.debugger.expect_exact('Hit breakpoint 2')
        self.debugger.sendline('sharedlib info')
        self.debugger.expect_exact('./examples/target/debug/libprinter_lib.so')

    def test_dynamic_load_lib_step(self):
        """Do steps into dynamically loaded shared library code"""
        self.debugger.sendline('break main.rs:19')
        self.debugger.expect('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.expect_exact('19         print_sum_fn(sum_1_2);')

        self.debugger.sendline('step')
        self.debugger.sendline('step')
        self.debugger.sendline('step')
        self.debugger.sendline('step')
        self.debugger.sendline('step')
        self.debugger.expect_exact('printer_lib/src/lib.rs:3')
        self.debugger.expect_exact('3     println!("sum is {num}")')

    def test_deferred_breakpoint(self):
        """Set breakpoint into dynamically loaded shared lib"""
        self.debugger.sendline('break print_sum')
        self.debugger.expect('Add deferred breakpoint for future shared library load')
        self.debugger.sendline('y')

        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint')
        self.debugger.expect_exact('3     println!("sum is {num}")')
