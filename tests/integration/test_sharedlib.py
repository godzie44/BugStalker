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
        self.debugger.expect(r'\?\?\?.*\.\/examples\/target\/debug\/calc_bin')
        # assert that shared lib showing
        self.debugger.expect(r'\?\?\?.*\.\/examples\/target\/debug\/libcalc_lib\.so')

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
        self.debugger.expect_exact('let sum_1_2 = unsafe { add(1, 2) }')

        self.debugger.sendline('step')
        self.debugger.expect(r'lib\.rs.*:3')
        self.debugger.expect_exact('3     a + b')

        self.debugger.sendline('step')
        self.debugger.expect_exact('4 }')

        self.debugger.sendline('step')
        self.debugger.expect(r'main\.rs.*:8')
        self.debugger.expect_exact('8     println!("1 + 2 = {sum_1_2}")')

    def test_lib_fn_breakpoint(self):
        """Set breakpoint at shared library function"""
        self.debugger.sendline('break add')
        self.debugger.expect('New breakpoint 1')

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
