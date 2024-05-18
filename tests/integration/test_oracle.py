import unittest
from helper import Debugger


class OracleTestCase(unittest.TestCase):
    """Test oracles systems"""

    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/tokioticker', oracles=['tokio'])

    def test_oracle_unavailable_until_debugee_start(self):
        """Oracles unavailable until program not started"""
        self.debugger.cmd('oracle tokio', 'Oracle not found or not ready')

    def test_tokio_oracle(self):
        """Test tokio oracle"""
        self.debugger.cmd('b main.rs:20', 'New breakpoint')
        self.debugger.cmd('b main.rs:32', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        self.debugger.cmd_re('oracle tokio', r'[1-9]\d? tasks running')
        self.debugger.cmd('continue', 'Hit breakpoint 2')
        self.debugger.cmd('oracle tokio', '0 tasks running')
        self.debugger.cmd('q')
