import unittest
import pexpect


class CommandTestCase(unittest.TestCase):
    def setUp(self):
        debugger = pexpect.spawn('./target/debug/bugstalker ./tests/hello_world')
        debugger.expect('No previous history.')
        self.debugger = debugger

    def test_debugee_execute(self):
        """Debugee executing"""
        self.debugger.sendline('run')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('bye!')

    def test_address_breakpoint_set(self):
        """Sets breakpoints at address"""
        self.debugger.sendline('break 0x55555555BD63')
        self.debugger.expect('break 0x55555555BD63')
        self.debugger.sendline('run')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('Hit breakpoint at address 0x0055555555BD63')
        self.debugger.expect_exact('myprint("bye!")')
        self.debugger.sendline('continue')
        self.debugger.expect('bye!')

    def test_multiple_address_breakpoint_set(self):
        """Sets multiple breakpoints at address"""
        self.debugger.sendline('break 0x55555555BD30')
        self.debugger.expect('break 0x55555555BD30')
        self.debugger.sendline('break 0x55555555BD63')
        self.debugger.expect('break 0x55555555BD63')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint at address 0x0055555555BD30')
        self.debugger.expect_exact('myprint("Hello, world!")')

        self.debugger.sendline('continue')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('Hit breakpoint at address 0x0055555555BD63')
        self.debugger.expect_exact('myprint("bye!")')

        self.debugger.sendline('continue')
        self.debugger.expect('bye!')

    def test_write_register(self):
        """Register writes (by moving pc counter into program start)"""
        self.debugger.sendline('break 0x55555555BD6C')
        self.debugger.expect('break 0x55555555BD6C')

        self.debugger.sendline('run')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('bye!')

        self.debugger.sendline('register write rip 55555555BD20')
        self.debugger.expect('register write rip 55555555BD20')

        self.debugger.sendline('continue')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('bye!')

    def test_step_in(self):
        """Debugger step in command (move to next line)"""
        self.debugger.sendline('break 0x55555555BD20')
        self.debugger.expect('break 0x55555555BD20')

        self.debugger.sendline('run')
        self.debugger.expect('>fn main()')
        self.debugger.sendline('step')
        self.debugger.expect_exact('>    myprint("Hello, world!");')
        self.debugger.sendline('step')
        self.debugger.expect_exact('>fn myprint(s: &str)')
        self.debugger.sendline('step')
        self.debugger.expect_exact('>    println!("{}", s)')

    def test_step_out(self):
        """Debugger step out command (move out from current function)"""
        self.debugger.sendline('break 0x55555555BD30')
        self.debugger.expect('break 0x55555555BD30')

        self.debugger.sendline('run')
        self.debugger.expect_exact('myprint("Hello, world!");')
        self.debugger.sendline('step')
        self.debugger.expect_exact('>fn myprint(s: &str)')
        self.debugger.sendline('step')
        self.debugger.expect_exact('>    println!("{}", s)')
        self.debugger.sendline('stepout')
        self.debugger.expect_exact('>    sleep(Duration::from_secs(1));')

    def test_step_over(self):
        """Debugger step over command (move to next line without
        entering functions)"""
        self.debugger.sendline('break 0x55555555BD30')
        self.debugger.expect('break 0x55555555BD30')

        self.debugger.sendline('run')
        self.debugger.expect_exact('myprint("Hello, world!");')
        self.debugger.sendline('next')
        self.debugger.expect_exact('>    sleep(Duration::from_secs(1));')
        self.debugger.sendline('next')
        self.debugger.expect_exact('>    myprint("bye!")')
        self.debugger.sendline('next')
        self.debugger.expect_exact('>}')

    def test_step_over_on_fn_decl(self):
        """Stop debugee at function declaration line"""
        self.debugger.sendline('break hello_world.rs:14')
        self.debugger.expect('break hello_world.rs:14')

        self.debugger.sendline('run')
        self.debugger.expect('Hit breakpoint at address')
        self.debugger.sendline('next')
        self.debugger.expect_exact('>    println!("{}", s)')

    def test_function_breakpoint(self):
        """Stop debugee at function by its name"""
        self.debugger.sendline('break myprint')
        self.debugger.expect('break myprint')

        self.debugger.sendline('run')
        self.debugger.expect('fn myprint')
        self.debugger.sendline('continue')
        self.debugger.expect('Hello, world!')
        self.debugger.expect('fn myprint')
        self.debugger.sendline('continue')
        self.debugger.expect('bye!')

    def test_line_breakpoint(self):
        """Stop debugee at line by its number"""
        self.debugger.sendline('break hello_world.rs:15')
        self.debugger.expect('break hello_world.rs:15')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    println!("{}", s)')
        self.debugger.sendline('continue')
        self.debugger.expect('Hello, world!')
        self.debugger.expect_exact('>    println!("{}", s)')
        self.debugger.sendline('continue')
        self.debugger.expect('bye!')

    def test_get_symbol(self):
        """Get debugee symbol"""
        self.debugger.sendline('symbol main')
        self.debugger.expect('Text 0x00000000007DE0')

    def test_backtrace(self):
        """Backtrace"""
        self.debugger.sendline('break hello_world.rs:15')
        self.debugger.expect('break hello_world.rs:15')

        self.debugger.sendline('run')
        self.debugger.expect_exact('>    println!("{}", s)')

        self.debugger.sendline('bt')
        self.debugger.expect_exact('myprint (0x0055555555BD70)')
        self.debugger.expect_exact('hello_world::main (0x0055555555BD20)')

    @staticmethod
    def test_read_value_u64():
        """Get program variable"""
        debugger = pexpect.spawn('./target/debug/bugstalker ./tests/calc')
        debugger.expect('No previous history.')
        debugger.sendline('break calc.rs:3')
        debugger.expect('break calc.rs:3')

        debugger.sendline('run')
        debugger.expect_exact('>    print(s);')

        debugger.sendline('vars')
        debugger.expect_exact('s = i64(3)')
