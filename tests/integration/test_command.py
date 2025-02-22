import unittest
from helper import Debugger


class CommandTestCase(unittest.TestCase):
    def setUp(self):
        self.debugger = Debugger(path='./examples/target/debug/hello_world')

    def test_debugee_execute(self):
        """Debugee executing"""
        self.debugger.cmd('run', 'Hello, world!', 'bye!')

    def test_function_breakpoint(self):
        """Stop debugee at function by its name"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('run', 'myprint("Hello, world!");')
        self.debugger.cmd('break myprint', 'New breakpoint')
        self.debugger.cmd('continue', 'Hit breakpoint 2')
        self.debugger.cmd('continue', 'Hello, world!', 'Hit breakpoint 2')
        self.debugger.cmd('continue', 'bye')

    def test_line_breakpoint(self):
        """Stop debugee at line by its number"""
        self.debugger.cmd('break hello_world.rs:15', 'New breakpoint')
        self.debugger.cmd('run', '15     println!("{}", s)')
        self.debugger.cmd('continue', 'Hello, world!', '15     println!("{}", s)')
        self.debugger.cmd('continue', 'bye!')

    def test_multiple_breakpoints_set(self):
        """Sets multiple breakpoints at line"""
        self.debugger.cmd('break hello_world.rs:5', 'New breakpoint')
        self.debugger.cmd('break hello_world.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at', 'myprint("Hello, world!")')
        self.debugger.cmd('continue', 'Hello, world!', 'Hit breakpoint 2 at', 'myprint("bye!")')
        self.debugger.cmd('continue', 'bye!')

    # maps 555555554000-55555555a000
    def test_address_breakpoint_set(self):
        """Sets breakpoints at address"""
        # determine address first
        self.debugger.cmd('break hello_world.rs:5', 'New breakpoint')
        self.debugger.cmd('run')
        addr = self.debugger.search_in_output(r'Hit breakpoint 1 at .*0x(.*):')
        addr = "0x" + addr[:14]
        self.debugger.cmd('q')
        # respawn debugger and test address breakpoint
        self.debugger = Debugger(path='./examples/target/debug/hello_world')
        self.debugger.cmd(f'break {addr}', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at')
        self.debugger.cmd('continue', 'Hello, world!', 'bye!')

    def test_write_register(self):
        """Register writes (by moving pc counter into program start)"""
        # determine program start and main ret addresses first
        self.debugger.cmd('break hello_world.rs:4', 'New breakpoint')
        self.debugger.cmd('break hello_world.rs:10', 'New breakpoint')
        self.debugger.cmd('run')

        start_addr = self.debugger.search_in_output(r'Hit breakpoint 1 at .*0x(.*):')
        start_addr = "0x" + start_addr[:14]
        self.assertNotEqual(start_addr, "")
        self.debugger.cmd('continue')
        addr = self.debugger.search_in_output(r'Hit breakpoint 2 at .*0x(.*):')
        addr = "0x" + addr[:14]
        self.assertNotEqual(addr, "")
        self.debugger.cmd('q')

        # assume that address of ret instruction at 1 byte offset
        addr_as_integer = int(addr, 16) + 1
        ret_addr = hex(addr_as_integer)

        # respawn debugger and move pc counter
        self.debugger = Debugger(path='./examples/target/debug/hello_world')
        self.debugger.cmd(f'break {ret_addr}', 'New breakpoint')
        self.debugger.cmd('run', 'Hello, world!', 'bye!')
        self.debugger.cmd(f'register write rip {start_addr}')
        self.debugger.cmd('continue', 'Hello, world!', 'bye!')

    @staticmethod
    def test_step_in():
        """Debugger step in command (move to next line)"""
        debugger = Debugger(path='./examples/target/debug/calc -- 1 2 3 --description result')
        debugger.cmd('break main.rs:10', 'New breakpoint')
        debugger.cmd('run', '10     let s: i64')
        debugger.cmd('step', 'calc::sum3', '25     let ab = sum2')
        debugger.cmd('step', 'calc::sum2', '21     a + b')
        debugger.cmd('step', '22 }')
        debugger.cmd('step', 'calc::sum3', '26     sum2(ab, c)')
        debugger.cmd('step', 'calc::sum2', '21     a + b')
        debugger.cmd('step', '22 }')
        debugger.cmd('step', 'calc::sum3', '27 }')
        debugger.cmd('step', 'calc::main', '15     print(s, &args[5]);')

    def test_step_out(self):
        """Debugger step out command (move out from current function)"""
        self.debugger.cmd('break hello_world.rs:15', 'New breakpoint')
        self.debugger.cmd('run', '15     println!("{}", s)')
        self.debugger.cmd('stepout', '7     sleep(Duration::from_secs(1));')

    def test_step_over(self):
        """Debugger step over command (move to next line without
        entering functions)"""
        self.debugger.cmd('break hello_world.rs:5', 'New breakpoint')
        self.debugger.cmd('run', 'myprint("Hello, world!");')
        self.debugger.cmd('next', '7     sleep(Duration::from_secs(1));')
        self.debugger.cmd('next', '9     myprint("bye!")')
        self.debugger.cmd('next', '10 }')

    def test_step_over_on_fn_decl(self):
        """Stop debugee at function declaration line"""
        self.debugger.cmd('break hello_world.rs:14', 'New breakpoint')
        self.debugger.cmd('run', 'Hit breakpoint 1 at')
        self.debugger.cmd('next', '15     println!("{}", s)')

    def test_get_symbol(self):
        """Get debugee symbol"""
        self.debugger.cmd_re('symbol main', '__libc_start_main', r'main - Text 0x[0-9A-F]{,16}')

    def test_backtrace(self):
        """Backtrace"""
        self.debugger.cmd('break hello_world.rs:15', 'New breakpoint')
        self.debugger.cmd('run', '15     println!("{}", s)')
        self.debugger.cmd('bt', 'myprint', 'hello_world::main')

    @staticmethod
    def test_args_for_executable():
        """Run debugee with arguments"""
        debugger = Debugger(path='./examples/target/debug/calc -- 1 1 1 --description three')
        debugger.cmd('run', 'three: 3')

    @staticmethod
    def test_read_value_u64():
        """Get program variable"""
        debugger = Debugger(path='./examples/target/debug/calc -- 1 2 3 --description result')
        debugger.cmd('break main.rs:15', 'New breakpoint')
        debugger.cmd('run', '15     print(s, &args[5]);')
        debugger.cmd('var locals', 's = i64(6)')

    def test_function_breakpoint_remove(self):
        """Remove breakpoint at function by its name"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('break remove main', 'Removed breakpoint')
        self.debugger.cmd('run', 'bye!')

    def test_line_breakpoint_remove(self):
        """Remove breakpoint at line by its number"""
        self.debugger.cmd('break hello_world.rs:15', 'New breakpoint')
        self.debugger.cmd('run', '15     println!("{}", s)')
        self.debugger.cmd('break remove hello_world.rs:15', 'Removed breakpoint')
        self.debugger.cmd('continue', 'bye!')

    def test_breakpoint_remove_by_number(self):
        """Remove breakpoint by its number"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('break remove 1', 'Removed breakpoint')
        self.debugger.cmd('run', 'bye!')

    def test_breakpoint_info(self):
        """View breakpoints list"""
        self.debugger.cmd('break hello_world.rs:9', 'New breakpoint')
        self.debugger.cmd('break myprint', 'New breakpoint')
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('break hello_world.rs:7', 'New breakpoint')

        self.debugger.cmd_re(
            'break info',
            r'- Breakpoint 1 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:9',
            r'- Breakpoint 2 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:15',
            r'- Breakpoint 3 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:5',
            r'- Breakpoint 4 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:7',
        )

        self.debugger.cmd('run')

        self.debugger.cmd_re(
            'break info',
            r'- Breakpoint 1 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:9 ',
            r'- Breakpoint 2 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:15',
            r'- Breakpoint 3 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:5',
            r'- Breakpoint 4 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:7',
        )

        self.debugger.cmd('break remove main', 'Removed breakpoint')

        self.debugger.cmd_re(
            'break info',
            r'- Breakpoint 1 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:9 ',
            r'- Breakpoint 2 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:15',
            r'- Breakpoint 4 at .*0x[0-9A-F]{14,16}.*: .*\/hello_world\.rs.*:7'
        )

    def test_debugee_restart(self):
        """Debugee process restart"""
        self.debugger.cmd('run', 'Hello, world!', 'bye!')
        self.debugger.cmd('run', 'Restart a program?')
        self.debugger.cmd('y', 'Hello, world!', 'bye!')

    def test_debugee_restart_at_bp(self):
        """Debugee process restarting at breakpoint"""
        self.debugger.cmd('break hello_world.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hello, world!')
        self.debugger.cmd('run', 'Restart a program?')
        self.debugger.cmd('y', 'Hello, world!')
        self.debugger.cmd('continue', 'bye!')

    def test_debugee_restart_at_end(self):
        """Debugee process restarting after debugee completing"""
        self.debugger.cmd('break hello_world.rs:9', 'New breakpoint')
        self.debugger.cmd('run', 'Hello, world!', 'Hit breakpoint 1')
        self.debugger.cmd('continue', 'bye!')
        self.debugger.cmd('run', 'Restart a program?')
        self.debugger.cmd('y', 'Hello, world!', 'Hit breakpoint 2')
        self.debugger.cmd('quit')

    @staticmethod
    def test_frame_switch():
        """Switch stack frame and assert argument values"""
        debugger = Debugger(path='./examples/target/debug/calc -- 1 2 3 --description result')
        debugger.cmd('break main.rs:21', 'New breakpoint 1')
        debugger.cmd('run', 'Hit breakpoint 1')
        debugger.cmd('arg all', 'a = i64(1)', 'b = i64(2)')
        debugger.cmd('frame switch 1')
        debugger.cmd('arg all', 'a = i64(1)', 'b = i64(2)', 'c = i64(3)')

    def test_disasm(self):
        """View function disassembled code"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('run')
        self.debugger.cmd('source asm', 'Assembler code for function hello_world::main', 'mov')
        self.debugger.cmd('break myprint', 'New breakpoint')
        self.debugger.cmd('continue')
        self.debugger.cmd('source asm', 'Assembler code for function hello_world::myprint', 'mov')

    def test_source_fn(self):
        """View function source code"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('run')
        self.debugger.cmd(
            'source fn',
            'hello_world::main at',
            '4 fn main() {',
            '7     sleep(Duration::from_secs(1));',
            '10 }'
        )

    @staticmethod
    def test_source_fn_with_frame_switch():
        """Switch stack frame and assert argument values"""
        debugger = Debugger(path='./examples/target/debug/calc -- 1 2 3 --description result')
        debugger.cmd('break main.rs:21', 'New breakpoint 1')
        debugger.cmd('run', 'Hit breakpoint 1')
        debugger.cmd(
            'source fn',
            'fn sum2(a: i64, b: i64) -> i64 {',
            'a + b',
            '}',
        )

        debugger.cmd('frame switch 1')

        debugger.cmd(
            'source fn',
            'fn sum3(a: i64, b: i64, c: i64) -> i64 {',
            'let ab = sum2(a, b);',
            'sum2(ab, c)',
            '}',
        )

        debugger.cmd('frame switch 2')

        debugger.cmd(
            'source fn',
            'fn main() {',
            'let args: Vec<String> = env::args().collect();',
            'let v1 = &args[1];',
            'let v2 = &args[2];',
            '}',
        )

    def test_source_bounds(self):
        """View source code"""
        self.debugger.cmd('break main', 'New breakpoint')
        self.debugger.cmd('run')
        self.debugger.cmd(
            'source 4',
            '1 use std::thread::sleep;',
            '4 fn main() {',
            '9     myprint("bye!")',
        )

    @staticmethod
    def test_breakpoint_at_rust_panic():
        """Set breakpoint to rust panic handler and catch panics"""
        debugger = Debugger(path='./examples/target/debug/panic -- user')
        debugger.cmd('break rust_panic', 'New breakpoint')
        debugger.cmd('run', 'then panic!')
        debugger.cmd('bt', 'rust_panic', 'panic::user_panic')
        debugger.cmd('continue')

        debugger = Debugger(path='./examples/target/debug/panic -- system')
        debugger.cmd('break rust_panic', 'New breakpoint')
        debugger.cmd('run', 'attempt to divide by zero')
        debugger.cmd('bt', 'rust_panic', 'panic::divided_by_zero')
