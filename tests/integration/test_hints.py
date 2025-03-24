import unittest
from helper import Debugger


class HintsTestCase(unittest.TestCase):
    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/vars')

    def test_command_hints(self):
        """Test command autocompletion"""
        self.debugger.print('br\t', 'break')
        self.debugger.print('\n')
        self.debugger.print('f\t', 'frame')
        self.debugger.print('\n')
        self.debugger.print('ste\t', 'step')
        self.debugger.print('\n')
        self.debugger.print('b\t\t', '\x1b[4mb\x1b[0mreak', ' backtrace|\x1b[1m\x1b[4mbt\x1b[0m')

    def test_break_command_hints(self):
        """Test files autocompletion for `break` command"""
        self.debugger.print('b vars.r\t', 'b vars.rs:')
        self.debugger.print("\n")

    def test_var_command_hints(self):
        """Test variable autocompletion for `var` command"""
        self.debugger.cmd('break vars.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'let int32 = 2_i32;')

        self.debugger.print('var \t\t', 'int8', 'int16', '\x1b[4mlocals\x1b[0m')
        self.debugger.print(' int1\t', 'int16')
        
        self.debugger.print('\n')

        self.debugger.print('vard \t\t', 'int8', 'int16', '\x1b[4mlocals\x1b[0m')
        self.debugger.print(' int1\t', 'int16')

    def test_arg_command_hints(self):
        """Test argument autocompletion for `args` command"""
        self.debugger.cmd('break vars.rs:227', 'New breakpoint')
        self.debugger.cmd('run', 'println!("{by_val}");')

        self.debugger.print('arg \t\t', 'by_val', 'by_ref', 'vec', 'box_arr')
        self.debugger.print('\n')
        self.debugger.print('argd \t\t', 'by_val', 'by_ref', 'vec', 'box_arr')

    def test_sub_command_hints(self):
        """Test subcommands autocompletion"""
        self.debugger.print('thread cu\t', 'thread current')
        self.debugger.print('\n')
        self.debugger.print('thread \t\t', 'info', 'switch', 'current')
        self.debugger.print('\n')
        self.debugger.print('frame i\t', 'frame info')
        self.debugger.print('\n')
        self.debugger.print('frame \t\t', 'info', 'switch')
        self.debugger.print('\n')
