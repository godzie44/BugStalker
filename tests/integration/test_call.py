import unittest
import re
import time
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
        
    def test_pointer_arg(self):
        """Call function with pointer arguments"""
        self.debugger.cmd('break calls.rs:45', 'New breakpoint 1')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        
        self.debugger.cmd('var &arg1')
        addr1 = self.debugger.search_in_output(r'arg1.*\[(.*)\]')
        self.debugger.cmd('var &arg2')
        addr2 = self.debugger.search_in_output(r'arg2.*\[(.*)\]')
        self.debugger.cmd('var &arg3')
        addr3 = self.debugger.search_in_output(r'arg3.*\[(.*)\]')

        self.debugger.cmd('call print_deref ' + addr1 + ' ' + addr2 + ' ' + addr3, 'deref is 100 101 Foo { bar: 102, baz: "103" }')
                
    def test_fmt_vars(self):
        """Test vard command"""
        self.debugger = Debugger(path='./examples/target/debug/vars')

        self.debugger.cmd('break vars.rs:641')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        
        self.debugger.cmd(
            'vard locals', 
            '[]', 
            '[]', 
            '[1, 23, 3]', 
            'Struct0 { a: 1 }', 
            'Struct1 { field1: 1, field2: 3 }', 
            'Struct1 { field1: 1, field2: "44" }',
            'Struct2 { field1: "66", field2: 55 }',
            'Struct3 { field1: 11, field2: 12 }',
            '["abc", "ef", "g"]',
            'A',
            'S1(Struct1 { field1: 100, field2: "100" })',
            'S2(Struct2 { field1: 1, field2: 2 })',
            'Some(1)',
            '"some str"',
            '"some string"',
        )
        
    def test_fmt_args(self):
        """Test argd command"""
        self.debugger = Debugger(path='./examples/target/debug/vars')

        self.debugger.cmd('break vars.rs:645')
        self.debugger.cmd('run', 'Hit breakpoint 1')
        
        self.debugger.cmd(
            'argd all', 
            '"one"', 
            '["two", "three"]', 
        )