import unittest
import pexpect
import re


class WatchpointTestCase(unittest.TestCase):
    """Test watchpoint"""

    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bs -t none ./examples/target/debug/calculations')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_watchpoint(self):
        """Add a new watchpoint and check it works"""
        self.debugger.sendline('break calculations.rs:20')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch c')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint')
        self.debugger.expect_exact('old value: c = u64(3)')
        self.debugger.expect_exact('new value: c = u64(1)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.expect_exact('old value: c = u64(1)')

    def test_watchpoint_at_field(self):
        """Add a new watchpoint for structure field or value in vector and check it works"""
        self.debugger.sendline('break calculations.rs:80')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('break calculations.rs:88')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')

        self.debugger.expect_exact('Hit breakpoint 1')
        self.debugger.sendline('watch vector[2]')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint')
        self.debugger.expect_exact('old value: vector[2] = i32(3)')
        self.debugger.expect_exact('new value: vector[2] = i32(4)')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit breakpoint 2')
        self.debugger.sendline('watch s.b')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('c')
        self.debugger.expect_exact('old value: s.b = f64(1)')
        self.debugger.expect_exact('new value: s.b = f64(2)')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.expect_exact('old value: vector[2] = i32(4)')
        self.debugger.expect_exact('Watchpoint 2 end of scope')
        self.debugger.expect_exact('old value: s.b = f64(2)')

    def test_watchpoint_at_address(self):
        """Add a new watchpoint for a raw memory region and check it works"""
        self.debugger.sendline('break calculations.rs:18')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('var &a')
        addr = ""
        for x in range(10):
            line = self.debugger.readline().decode("utf-8")
            result = re.search(r'= &u64 \[0x(.*)\]', line)
            if result:
                addr = result.group(1)
                addr = "0x" + addr[:14]
                break

        self.debugger.sendline(f'watch {addr}:8')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint')
        self.debugger.expect_exact('old value: data = u64(1)')
        self.debugger.expect_exact('new value: data = u64(6)')

        self.debugger.sendline('q')

    def test_watchpoint_with_stepping(self):
        """Add a new watchpoint, do steps, test it works"""
        self.debugger.sendline('break calculations.rs:12')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch int8')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('next')
        self.debugger.expect_exact('Hit watchpoint')
        self.debugger.sendline('next')
        self.debugger.expect_exact('13     println!("{int8}");')
        self.debugger.sendline('next')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.sendline('next')
        self.debugger.expect_exact('102     calculation_four_value();')

    def test_watchpoint_at_undefined_value(self):
        """Trying to set watchpoint for undefined value"""
        self.debugger.sendline('break calculations.rs:20')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch e')
        self.debugger.expect_exact('variable or argument to watch not found')

    def test_watchpoint_address_already_in_use(self):
        """Check that set two watchpoints on a single memory location is forbidden"""
        self.debugger.sendline('break calculations.rs:18')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch a')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch a')
        self.debugger.expect_exact('memory location observed by another watchpoint')

    def test_watchpoint_remove(self):
        """Remove watchpoint"""
        self.debugger.sendline('break calculations.rs:22')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch a')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch remove 1')
        self.debugger.expect_exact('Removed watchpoint')

        self.debugger.sendline('watch b')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch remove b')
        self.debugger.expect_exact('Removed watchpoint')

    def test_watchpoint_hw_limit(self):
        """Check that watchpoints has a limited count"""
        self.debugger.sendline('break calculations.rs:22')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch a')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch b')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch c')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch d')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch GLOBAL_1')
        self.debugger.expect_exact('watchpoint limit is reached')

        self.debugger.sendline('watch remove a')
        self.debugger.expect_exact('Removed watchpoint')

        self.debugger.sendline('watch GLOBAL_1')
        self.debugger.expect_exact('New watchpoint')

    def test_watchpoint_after_restart(self):
        """Set watchpoint to local and global variables, restart debugee, check that local is
        removed but global not"""
        self.debugger.sendline('break calculations.rs:22')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch a')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch GLOBAL_1')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch c')
        self.debugger.expect_exact('New watchpoint')
        self.debugger.sendline('watch info')
        self.debugger.expect_exact('3/4 active watchpoints')

        self.debugger.sendline('run')
        self.debugger.expect('Restart a program?')
        self.debugger.sendline('y')
        self.debugger.expect_exact('Hit breakpoint')

        self.debugger.sendline('watch info')
        self.debugger.expect_exact('1/4 active watchpoints')

        self.debugger.sendline('continue')
        self.debugger.expect('Hit watchpoint 2?')

        self.debugger.sendline('q')

    def test_watchpoint_rw(self):
        """Add a new watchpoint with read-write condition and check it works"""
        self.debugger.sendline('break calculations.rs:20')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch +rw a')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('value: a = u64(1)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('old value: a = u64(1)')
        self.debugger.expect_exact('new value: a = u64(6)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('value: a = u64(6)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.expect_exact('old value: a = u64(6)')

    def test_watchpoint_at_addr_rw(self):
        """Add a new watchpoint with read-write condition at address and check it works"""
        self.debugger.sendline('break calculations.rs:20')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('var &a')
        addr = ""
        for x in range(10):
            line = self.debugger.readline().decode("utf-8")
            result = re.search(r'= &u64 \[0x(.*)\]', line)
            if result:
                addr = result.group(1)
                addr = "0x" + addr[:14]
                break
        self.debugger.sendline(f'watch +rw {addr}:8')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('value: data = u64(1)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('old value: data = u64(1)')
        self.debugger.expect_exact('new value: data = u64(6)')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (rw)')
        self.debugger.expect_exact('value: data = u64(6)')

    def test_watchpoint_at_complex_data_types(self):
        """Add watchpoints for vector attribute"""
        self.debugger.sendline('break calculations.rs:91')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch (~vector2).len')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (w)')
        self.debugger.expect_exact('old value: (~vector2).len = usize(2)')
        self.debugger.expect_exact('new value: (~vector2).len = usize(3)')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (w)')
        self.debugger.expect_exact('old value: (~vector2).len = usize(3)')
        self.debugger.expect_exact('new value: (~vector2).len = usize(4)')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.expect_exact('old value: (~vector2).len = usize(4)')

    def test_watchpoint_at_complex_data_types2(self):
        """Add watchpoints for string attribute"""
        self.debugger.sendline('break calculations.rs:95')
        self.debugger.expect_exact('New breakpoint')
        self.debugger.sendline('run')
        self.debugger.expect_exact('Hit breakpoint 1')

        self.debugger.sendline('watch (~(~string).vec).len')
        self.debugger.expect_exact('New watchpoint')

        self.debugger.sendline('c')
        self.debugger.expect_exact('Hit watchpoint 1 (w)')
        self.debugger.expect_exact('old value: (~(~string).vec).len = usize(3)')
        self.debugger.expect_exact('new value: (~(~string).vec).len = usize(7)')
        self.debugger.sendline('c')
        self.debugger.expect_exact('Watchpoint 1 end of scope')
        self.debugger.expect_exact('old value: (~(~string).vec).len = usize(7)')
