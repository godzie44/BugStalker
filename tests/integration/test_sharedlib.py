import unittest
import pexpect
from helper import Debugger


class SharedLibTestCase(unittest.TestCase):
    """Test a debugger on project with shared libraries dependencies"""

    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/calc_bin')

    def test_lib_info(self):
        """View information about loaded shared libraries"""
        self.debugger.cmd(
            'sharedlib info',
            # assert that main executable showing
            '???     ./examples/target/debug/calc_bin',
            # assert that shared lib showing
            '???     ./examples/target/debug/libcalc_lib.so',
        )

        self.debugger.cmd('break main.rs:7', 'New breakpoint')
        self.debugger.cmd('run')

        self.debugger.cmd_re(
            'sharedlib info',
            # assert that main executable showing with mapping address
            r'0x.*\.\/examples\/target\/debug\/calc_bin',
            # assert that shared lib showing with mapping address
            r'0x.*\.\/examples\/target\/debug\/libcalc_lib\.so',
        )

    def test_lib_step(self):
        """Do steps in shared library code"""
        self.debugger.cmd('break main.rs:7', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1', 'let sum_1_2 = unsafe { calc_add(1, 2) }')
        self.debugger.cmd('step', 'lib.rs:3', '3     a + b')
        self.debugger.cmd('step', '4 }')
        self.debugger.cmd('step', 'main.rs:8', '8     let sub_2_1 = unsafe { calc_sub(2, 1) };')

    def test_lib_fn_breakpoint(self):
        """Set breakpoint at shared library function"""
        self.debugger.cmd('break calc_add', 'New breakpoint 1')
        self.debugger.cmd('run', 'Hit breakpoint 1', '3     a + b')

    def test_lib_line_breakpoint(self):
        """Set breakpoint at line in shared library source code"""
        self.debugger.cmd('b lib.rs:8', 'New breakpoint 1')
        self.debugger.cmd('run', 'Hit breakpoint 1', '8     a - b')

    def test_dynamic_load_lib_info(self):
        """View information about shared libraries loaded dynamically"""
        self.debugger.cmd('b main.rs:8', 'New breakpoint 1')
        self.debugger.cmd('b main.rs:14', 'New breakpoint 2')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('sharedlib info')

        try:
            self.debugger.expect_in_output('./examples/target/debug/libprinter_lib.so')
        except pexpect.ExceptionPexpect:
            self.debugger.cmd('continue')
        else:
            raise pexpect.ExceptionPexpect("lib is not loading at this point")

        self.debugger.expect_in_output('Hit breakpoint 2')
        self.debugger.cmd('sharedlib info', './examples/target/debug/libprinter_lib.so')

    def test_dynamic_load_lib_step(self):
        """Do steps into dynamically loaded shared library code"""
        self.debugger.cmd('break main.rs:19', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1', '19         print_sum_fn(sum_1_2);')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step')
        self.debugger.cmd('step', '3     println!("sum is {num}")')

    def test_deferred_breakpoint(self):
        """Set breakpoint into dynamically loaded shared lib"""
        self.debugger.cmd('break print_sum', 'Add deferred breakpoint for future shared library load')
        self.debugger.cmd('y')
        self.debugger.cmd('run', 'Hit breakpoint')
