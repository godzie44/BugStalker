import unittest
from helper import Debugger


class FunctionCallTestCase(unittest.TestCase):
    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/calls')

    def test_simple_call_execute(self):
        """Call simple sum function"""
        self.debugger.cmd('break main', 'New breakpoint 1')
        self.debugger.cmd('run')
        self.debugger.cmd('call sum2 2 5', 'my sum is 7')
        self.debugger.cmd('call sum2 1 9', 'my sum is 10')
        self.debugger.cmd('continue', 'Program exit with code: 0')
        
    def test_breakpoint_not_hit(self):
        """Set a breakpoint, verify that it won't be hit when using the call command"""
        self.debugger.cmd('break main', 'New breakpoint 1')
        self.debugger.cmd('break sum2', 'New breakpoint 2')
        self.debugger.cmd('run')
        self.debugger.cmd('call sum2 2 5', 'my sum is 7')
        self.debugger.cmd('call sum2 1 9', 'my sum is 10')
        self.debugger.cmd('continue', 'Hit breakpoint 2')
        self.debugger.cmd('continue', 'Program exit with code: 0')
        
    def test_six_args(self):
        """Call sum function with 6 args of unsigned/signed integers"""
        self.debugger.cmd('break main', 'New breakpoint 1')
        self.debugger.cmd('run')
        self.debugger.cmd('call sum6i -1 -2 -3 -4 -5 -6', 'my sum is -21')
        self.debugger.cmd('call sum6i 1 256 65537 4294967297 4294967297 -1', 'my sum is 8590000387')
        self.debugger.cmd('call sum6u 1 2 3 4 5 6', 'my sum is 21')
        self.debugger.cmd('call sum6u 1 256 65537 4294967297 4294967297 1', 'my sum is 8590000389')
        
    def test_bool_arg(self):
        """Call function with bool arg"""
        self.debugger.cmd('break main', 'New breakpoint 1')
        self.debugger.cmd('run')
        self.debugger.cmd('call print_bool false', 'bool is false')
        self.debugger.cmd('call print_bool true', 'bool is true')
