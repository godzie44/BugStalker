import unittest
from helper import Debugger


class WatchpointTestCase(unittest.TestCase):
    """Test watchpoint"""

    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/calculations')

    def test_watchpoint(self):
        """Add a new watchpoint and check it works"""
        self.debugger.cmd('break calculations.rs:20', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch c', 'New watchpoint')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: c) (w)',
            'old value: u64(3)',
            'new value: u64(1)',
        )

        self.debugger.cmd('continue', 'Watchpoint 1 (expr: c) end of scope', 'old value: u64(1)')

    def test_watchpoint_at_field(self):
        """Add a new watchpoint for structure field or value in vector and check it works"""
        self.debugger.cmd('break calculations.rs:80', 'New breakpoint')
        self.debugger.cmd('break calculations.rs:88', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch vector[2]', 'New watchpoint')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: vector[2]) (w)',
            'old value: i32(3)',
            'new value: i32(4)',
        )
        self.debugger.cmd('continue', 'Hit breakpoint 2')
        self.debugger.cmd('watch s.b', 'New watchpoint')
        self.debugger.cmd('continue', 'old value: f64(1)', 'new value: f64(2)')
        self.debugger.cmd(
            'continue',
            'Watchpoint 1 (expr: vector[2]) end of scope',
            'old value: i32(4)',
            'Watchpoint 2 (expr: s.b) end of scope',
            'old value: f64(2)',
        )

    def test_watchpoint_at_address(self):
        """Add a new watchpoint for a raw memory region and check it works"""
        self.debugger.cmd('break calculations.rs:18', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')

        self.debugger.cmd('var &a')
        addr = self.debugger.search_in_output(r'&u64 \[0x(.*)\]')
        addr = "0x" + addr[:14]
        self.debugger.cmd(f'watch {addr}:8', 'New watchpoint')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint',
            'old value: u64(1)',
            'new value: u64(6)',
        )
        self.debugger.cmd('q')

    def test_watchpoint_with_stepping(self):
        """Add a new watchpoint, do steps, test it works"""
        self.debugger.cmd('break calculations.rs:22', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch a', 'New watchpoint')
        self.debugger.cmd('next', 'Hit watchpoint')
        self.debugger.cmd('next', '24     c -= 2;')
        self.debugger.cmd('next')
        self.debugger.cmd('next')
        self.debugger.cmd('next', 'Watchpoint 1 (expr: a) end of scope')

    def test_watchpoint_at_undefined_value(self):
        """Trying to set watchpoint for undefined value"""
        self.debugger.cmd('break calculations.rs:20', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch e', 'variable or argument to watch not found')

    def test_watchpoint_address_already_in_use(self):
        """Check that set two watchpoints on a single memory location is forbidden"""
        self.debugger.cmd('break calculations.rs:18', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch a', 'New watchpoint')
        self.debugger.cmd('watch a', 'memory location observed by another watchpoint')

    def test_watchpoint_remove(self):
        """Remove watchpoint"""
        self.debugger.cmd('break calculations.rs:22', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch a', 'New watchpoint')
        self.debugger.cmd('watch remove 1', 'Removed watchpoint')
        self.debugger.cmd('watch b', 'New watchpoint')
        self.debugger.cmd('watch remove b', 'Removed watchpoint')

    def test_watchpoint_hw_limit(self):
        """Check that watchpoints has a limited count"""
        self.debugger.cmd('break calculations.rs:22', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch a', 'New watchpoint')
        self.debugger.cmd('watch b', 'New watchpoint')
        self.debugger.cmd('watch c', 'New watchpoint')
        self.debugger.cmd('watch d', 'New watchpoint')
        self.debugger.cmd('watch GLOBAL_1', 'watchpoint limit is reached')
        self.debugger.cmd('watch remove a', 'Removed watchpoint')
        self.debugger.cmd('watch GLOBAL_1', 'New watchpoint')

    def test_watchpoint_after_restart(self):
        """Set watchpoint to local and global variables, restart debugee, check that local is
        removed but global not"""
        self.debugger.cmd('break calculations.rs:22', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch a', 'New watchpoint')
        self.debugger.cmd('watch GLOBAL_1', 'New watchpoint')
        self.debugger.cmd('watch c', 'New watchpoint')
        self.debugger.cmd('watch info', '3/4 active watchpoints')
        self.debugger.cmd('run', 'Restart a program?')
        self.debugger.cmd('y', 'Hit breakpoint')
        self.debugger.cmd('watch info', '1/4 active watchpoints')
        self.debugger.cmd('continue', 'Hit watchpoint 2')
        self.debugger.cmd('q')

    def test_watchpoint_rw(self):
        """Add a new watchpoint with read-write condition and check it works"""
        self.debugger.cmd('break calculations.rs:20', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch +rw a', 'New watchpoint')
        self.debugger.cmd('continue', 'Hit watchpoint 1 (expr: a) (rw)', 'value: u64(1)')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: a) (rw)',
            'old value: u64(1)',
            'new value: u64(6)',
        )
        self.debugger.cmd('continue', 'Hit watchpoint 1 (expr: a) (rw)', 'value: u64(6)')
        self.debugger.cmd('continue', 'Watchpoint 1 (expr: a) end of scope', 'old value: u64(6)')

    def test_watchpoint_at_addr_rw(self):
        """Add a new watchpoint with read-write condition at address and check it works"""
        self.debugger.cmd('break calculations.rs:20', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('var &a')
        addr = self.debugger.search_in_output(r'&u64 \[0x(.*)\]')
        addr = "0x" + addr[:14]
        self.debugger.cmd(f'watch +rw {addr}:8', 'New watchpoint')
        self.debugger.cmd('continue', 'Hit watchpoint 1 (rw)', 'value: u64(1)')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (rw)',
            'old value: u64(1)',
            'new value: u64(6)',
        )
        self.debugger.cmd('continue', 'Hit watchpoint 1 (rw)', 'value: u64(6)')

    def test_watchpoint_at_complex_data_types(self):
        """Add watchpoints for vector attribute"""
        self.debugger.cmd('break calculations.rs:91', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch (~vector2).len', 'New watchpoint')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: (~vector2).len) (w)',
            'old value: usize(2)',
            'new value: usize(3)',
        )
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: (~vector2).len) (w)',
            'old value: usize(3)',
            'new value: usize(4)',
        )
        self.debugger.cmd(
            'continue',
            'Watchpoint 1 (expr: (~vector2).len) end of scope',
            'old value: usize(4)',
        )

    def test_watchpoint_at_complex_data_types2(self):
        """Add watchpoints for string attribute"""
        self.debugger.cmd('break calculations.rs:95', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd('watch (~(~string).vec).len', 'New watchpoint')
        self.debugger.cmd(
            'continue',
            'Hit watchpoint 1 (expr: (~(~string).vec).len) (w)',
            'old value: usize(3)',
            'new value: usize(7)',
        )
        self.debugger.cmd(
            'continue',
            'Watchpoint 1 (expr: (~(~string).vec).len) end of scope',
            'old value: usize(7)',
        )
