import time
import unittest
import pexpect
import re


class HintsTestCase(unittest.TestCase):
    def setUp(self):
        debugger = pexpect.spawn(
            './target/debug/bugstalker ./target/debug/vars')
        debugger.expect('BugStalker greets')
        self.debugger = debugger

    def test_command_hints(self):
        """Test command autocompletion"""
        self.debugger.send("br\t")
        self.debugger.expect_exact('break')
        self.debugger.send("\n")

        self.debugger.send("f\t")
        self.debugger.expect_exact('frame')
        self.debugger.send("\n")

        self.debugger.send("ste\t")
        self.debugger.expect_exact('step')
        self.debugger.send("\n")

        self.debugger.send("b\t\t")
        self.debugger.expect_exact('\x1b[4mb\x1b[0mreak')
        self.debugger.expect_exact(' backtrace|\x1b[1m\x1b[4mbt\x1b[0m')

    def test_break_command_hints(self):
        """Test files autocompletion for `break` command"""
        self.debugger.send("b var\t")
        self.debugger.expect_exact('b vars.rs:')
        self.debugger.send("\n")

    def test_var_command_hints(self):
        """Test variable autocompletion for `var` command"""
        self.debugger.sendline('break vars.rs:9')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('let int32 = 2_i32;')

        self.debugger.send("var \t\t")
        self.debugger.expect_exact('int8')
        self.debugger.expect_exact('int16')
        self.debugger.expect_exact('\x1b[4mlocals\x1b[0m')

        self.debugger.send(" int1\t")
        self.debugger.expect_exact('int16')

    def test_arg_command_hints(self):
        """Test argument autocompletion for `args` command"""
        self.debugger.sendline('break vars.rs:222')
        self.debugger.expect_exact('New breakpoint')

        self.debugger.sendline('run')
        self.debugger.expect_exact('println!("{by_val}");')

        self.debugger.send("arg \t\t")
        self.debugger.expect_exact('by_val')
        self.debugger.expect_exact('by_ref')
        self.debugger.expect_exact('vec')
        self.debugger.expect_exact('box_arr')


