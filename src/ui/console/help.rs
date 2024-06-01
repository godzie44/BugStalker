use crate::debugger::Debugger;
use crate::ui::command;
use command::parser;

pub const HELP: &str = r#"
Available debugger commands:

var <name or expression>|locals             -- show local and global variables
arg <name or expression>|all                -- show arguments of current stack frame
bt, backtrace <>|all                        -- print backtrace of all stack frames in current thread or from all threads
f, frame info|switch <number>               -- print current stack frame information or change frame
c, continue                                 -- continue program being debugged, after signal or breakpoint
r, run                                      -- start or restart debugged programm 
stepi                                       -- step one instruction
step, stepinto                              -- step program until it reaches a different source line
finish, stepout                             -- execute program until selected stack frame returns
next, stepover                              -- step program, stepping over subroutine calls
b, break <addr>|<file:line>|<function>      -- manage breakpoints
w, watch +w|+rw| <expression>|<addr:size>   -- manage write or read-write watchpoints
symbol <name>                               -- print symbol kind and address
mem, memory read|write <addr>               -- read or write into debugged program memory
reg, register read|write|info <addr>        -- read, write, or view debugged program registers
thread info|current|switch <number>         -- show list of threads or current (in focus) thread or set thread in focus
sharedlib info                              -- show list of shared libraries
source asm|fn|<bounds>                      -- show source code or assembly instructions for current (in focus) function
oracle <oracle> <>|<subcommand>             -- execute a specific oracle
h, help <>|<command>                        -- show help
tui                                         -- change ui mode to tui
q, quit                                     -- exit the BugStalker 
"#;

pub const DQE_DESCRIPTION: &str = "
\x1b[;1mData query expression\x1b[0m
To analyze data in a program, you often need a tool for introspection of variables, which allows, 
for example, dereference a pointer or taking an array element by index. BugStalker provides a data 
query expressions as such a tool. 

Available operators:
`*` - dereference, available for references, pointers and smart pointers (Rc and Arc)
`&` - address operator, available for any variable, structure field or index value
`~` - canonic representation, types like vectors or hashmaps are prettyfied by default, use canonic operator to see a raw structure as is
`[{left}..{right}]` - slice operator, available for pointers
`.` - get field, available for structs, enums and hashmaps (with string keys)
`(` and `)` - parentheses to prioritize operations
`({ptr/ref type})` - cast constant address to typed pointer or reference
`[{literal}]` - index operator, available for arrays, enums, vectors, veqdequeues, hashmaps, hashsets, btreemaps and btreesets. 
Literal is a json-like object (with wildcards feature), that can be used for matching with real data. 
See `help dqe literal` for more information

Examples:
`**var1` - print the value pointed to by the pointer `*var1`
`**var1.field1` - print the value pointed to by the pointer `*var1.field1`
`&var1.field1` - print address of field `field1`
`(~vec1).len` - print lenght field of vector header structure
`&vec1[1]` - print address of second element in vector `vec1`
`(**var1).field1` - print field `field1` in struct pointed to by the pointer `*var1`
`*(*const i32)0x1234AA332` - cast memory address to `*const i32` pointer, then dereference it 
`hashmap[0x1337]` - get value by pointer key 0x1337 from hashmap 
`hashmap[{\"a\", \"b\"}]` - get value by array key {\"a\", \"b\"} from hashmap
`*(*(var1.field1)).field2[1][2]` - get `field1` from struct `var1`, dereference it, 
then get `field2` from dereference result, then get element by index 1, and get element 2 from it,
finally print dereference of this value
";

pub const DQE_LITERAL_DESCRIPTION: &str = "
\x1b[;1mLiterals in data query expression\x1b[0m
For advanced search of elements in containers (hashmaps, hashsets, etc.), debugger provides an index 
operator that accepts a literal object. This object is very similar to a regular json object, 
but, among other things, it supports wildcards (*) in arrays and associated arrays. 

Scalar literal objects:
- numbers (`123`) - matches with any rust integer values
- strings (`\"abc\") - matches with rust strings or &str's
- float numbers (`123.5`) - matches with rust floats equals with little epsilon
- booleans (`true`) - matches with bool's
- addresses (`0x1337`) - matches with pointers (smart pointers too) and referenses

Complex literal objects:
- arrays (`{1, 2, 3, *, 5}`) - matches with arrays, vectors, tuples, sets
- assoc arrays (`{field_1: 1, field_2: *}`) - matches with structures
- enum variants (`None`) - matches with enums identifiers
- enum variants with values (`Some({true, 12})`) - matches with enums identifiers with items

Example of usage of literal objects:
`map[1]` - get value by int key `1` from hashmap 
`map[\"key_1\"]` - get value by string key `key_1` from hashmap
`set[{*, *, *}]` - checks that there is a vector of any three elements in set
`map[{field_1: 1, field_2: Some({true}), field_3: *}]` - get value by key that matches 3-field structure with
field `field_1` equals to 1, field `field_2` equals to `Option::Some(true)` and field `field_3` equals to any value
";

pub const HELP_VAR: &str = "\
\x1b[32;1mvar\x1b[0m
Show local and global variables, supports data queries expressions over variables (see `help dqe`).

Available subcomands:
var locals - print current stack frame local variables
var <name or expression> - print local and global variables with selected name

Examples of usage:
var locals - print current stack frame local variables
var some_variable - print all variables with given name, variables can be in local or global scope 
var *some_variable - dereference and print value if `some_variable` is a pointer or RC/ARC
var some_array[0] - print first element if `some_array` is a vector, array, vecdeque or enum
var some_array[2..5] - print 3 elements, starts from index 2
var *some_array[0] - print dereferenced value of some_array[0]
var (*some_array)[0] - print first element of *some_array
";

pub const HELP_ARG: &str = "\
\x1b[32;1marg\x1b[0m
Show current stack frame arguments, supports data queries expressions over arguments (see `help dqe`).

Available subcomands:
arg all - print all arguments
arg <name or expression> - print argument with selected name

Examples of usage:
arg all - print current stack frame local variables
arg some_arg - print argument with name equals to `some_arg`
arg *some_arg - dereference and print value if `some_arg` is a pointer or RC/ARC
";

pub const HELP_BACKTRACE: &str = "\
\x1b[32;1mbt, backtrace\x1b[0m
Show backtrace of all stack frames in current thread or from all threads.

Available subcomands:
backtrace all - show backtrace for all running threads
backtrace - show backtrace of current thread

Output format:
thread {id} - {current ip value}
{current ip value} - {function name} ({function address} + {offset})
{the address of the instruction in the overlay stack frame} - {function name} ({function address} + {offset})
...
";

pub const HELP_FRAME: &str = "\
\x1b[32;1mf, frame\x1b[0m
Show current stack frame info or set frame to focus.

Available subcomands:
frame info - show current stack frame information (see output explanation)
frame switch <number> - set frame <number> to focus

Output format:
cfa: {address} -- canonical frame address
return address: {address} - return address for current stack frame
";

pub const HELP_CONTINUE: &str = "\
\x1b[32;1mc, continue\x1b[0m
Continue program being debugged, after signal or breakpoint.
";

pub const HELP_RUN: &str = "\
\x1b[32;1mr, run\x1b[0m
Start or restart debugged programm.
";

pub const HELP_STEPI: &str = "\
\x1b[32;1mstepi\x1b[0m
step one instruction.
";

pub const HELP_STEPINTO: &str = "\
\x1b[32;1mstep, stepinto\x1b[0m
Step program until it reaches a different source line.
";

pub const HELP_STEPOUT: &str = "\
\x1b[32;1mfinish, stepout\x1b[0m
Execute program until selected stack frame returns.
";

pub const HELP_STEPOVER: &str = "\
\x1b[32;1mnext, stepover\x1b[0m
Step program, stepping over subroutine calls.
";

pub const HELP_BREAK: &str = "\
\x1b[32;1mb, break\x1b[0m
Manage breakpoints.

Available subcomands:
break <location> - set breakpoint to location
break remove <location>|<number> - deactivate and delete selected breakpoint
break info - show all breakpoints

Posible location format:
- at instruction. Example: break 0x55555555BD30
- at function start. A function can be defined by its full name (with namespace) 
or by function name (in case of possible collisions, breakpoints will be set in 
all matching functions). Examples: 
    * break fn1 
    * break module1::fn1
- at code line. Example: break hello_world.rs:15
- a breakpoint number (only for `remove` subcommand)
";

pub const HELP_WATCH: &str = "\
\x1b[32;1mw, watch\x1b[0m
Manage watchpoints. Note that watchpoints for local variables and watchpoints for global varibales
or raw memory region have a different lifetimes. Watchpoints for global variables or memory locations
are lives until BugStalker session is alive. On the contrary, watchpoints for local variables
are lives until debugee is not restarted, and will be removed automatically.

Available subcomands:
watch +rw|+w| <addr:size> - set write or read-write watchpoint (write by default) to memory location [addr; addr+size], size must be one of [1,2,4,8] bytes
watch +rw|+w| <expression> - set write or read-write watchpoint (write by default) to DQE result (see `help dqe`), expression result must one of [1,2,4,8] bytes
watch remove <addr:size>|<expression>|<number> - deactivate and delete selected watchpoint
watch info - show all watchpoints

Examples:
* watch 0x00000004:4 - set watchpoint to memory region [0x0..04:0x0..07]
* watch var1 - set watchpoint (write condition) to variable `var1`
* watch +rw var1 - set watchpoint (read-write condition) to variable `var1`
* watch struct1.field1 - set watchpoint to `field1` of variable `struct1`
* watch arr[2] - set watchpoint to 2nd element of variable `arr`
";

pub const HELP_SYMBOL: &str = "\
\x1b[32;1msymbol\x1b[0m
Print symbols matched by regular expression.

Available subcomands:
symbol <name_regex>
";

pub const HELP_MEMORY: &str = "\
\x1b[32;1mmem, memory\x1b[0m
Read or write into debugged program memory.

Available subcomands:
memory read <address> - print 8-byte block at address in debugee memory
memory write <address> <value> - writes 8-byte value to address in debugee memory
";

pub const HELP_REGISTER: &str = "\
\x1b[32;1mreg, register\x1b[0m
Read, write, or view debugged program registers (x86_64 registers support).

Available subcomands:
register read <reg_name> - print value of register by name (x86_64 register name in lowercase)
register write <reg_name> <value> - set new value to register by name
register info - print list of registers with it values
";

pub const HELP_THREAD: &str = "\
\x1b[32;1mthread\x1b[0m
Show threads information or set thread to focus.

Available subcomands:
thread info - print list of thread information
thread current - prints thread that has focus
thread switch <number> - set thread <number> to focus
";

pub const HELP_SHARED_LIB: &str = "\
\x1b[32;1msharedlib\x1b[0m
Show shared libraries information.

Available subcomands:
sharedlib info - print list of loaded shared libraries and their mapping addresses
";

pub const HELP_SOURCE: &str = "\
\x1b[32;1msource\x1b[0m
Show source code or assembly instructions for current (in focus) function.

Available subcomands:
source fn - show code of function in focus 
source asm - show assembly of function in focus 
source <bounds> - show line in focus with <bounds> lines up and down of this line
";

pub const HELP_TUI: &str = "\
\x1b[32;1mtui\x1b[0m
Change ui mode to terminal ui.
";

pub const HELP_ORACLE: &str = "\
\x1b[32;1moracle\x1b[0m
Execute a specific oracle.

List of available oracles:
";

pub const HELP_QUIT: &str = "\
\x1b[32;1mq, quit\x1b[0m
Exit the BugStalker, kill debugee before it.
";

#[derive(Default)]
pub struct Helper {
    oracle_help: Option<String>,
}

impl Helper {
    pub fn help_for_command(&mut self, debugger: &Debugger, command: Option<&str>) -> &str {
        match command {
            None => HELP,
            Some("dqe") => DQE_DESCRIPTION,
            Some("dqe literal") => DQE_LITERAL_DESCRIPTION,
            Some(parser::VAR_COMMAND) => HELP_VAR,
            Some(parser::ARG_COMMAND) => HELP_ARG,
            Some(parser::BACKTRACE_COMMAND) | Some(parser::BACKTRACE_COMMAND_SHORT) => {
                HELP_BACKTRACE
            }
            Some(parser::FRAME_COMMAND) | Some(parser::FRAME_COMMAND_SHORT) => HELP_FRAME,
            Some(parser::CONTINUE_COMMAND) | Some(parser::CONTINUE_COMMAND_SHORT) => HELP_CONTINUE,
            Some(parser::RUN_COMMAND) | Some(parser::RUN_COMMAND_SHORT) => HELP_RUN,
            Some(parser::STEP_INSTRUCTION_COMMAND) => HELP_STEPI,
            Some(parser::STEP_INTO_COMMAND) | Some(parser::STEP_INTO_COMMAND_SHORT) => {
                HELP_STEPINTO
            }
            Some(parser::STEP_OUT_COMMAND) | Some(parser::STEP_OUT_COMMAND_SHORT) => HELP_STEPOUT,
            Some(parser::STEP_OVER_COMMAND) | Some(parser::STEP_OVER_COMMAND_SHORT) => {
                HELP_STEPOVER
            }
            Some(parser::BREAK_COMMAND) | Some(parser::BREAK_COMMAND_SHORT) => HELP_BREAK,
            Some(parser::WATCH_COMMAND) | Some(parser::WATCH_COMMAND_SHORT) => HELP_WATCH,
            Some(parser::SYMBOL_COMMAND) => HELP_SYMBOL,
            Some(parser::MEMORY_COMMAND) | Some(parser::MEMORY_COMMAND_SHORT) => HELP_MEMORY,
            Some(parser::REGISTER_COMMAND) | Some(parser::REGISTER_COMMAND_SHORT) => HELP_REGISTER,
            Some(parser::THREAD_COMMAND) => HELP_THREAD,
            Some(parser::SHARED_LIB_COMMAND) => HELP_SHARED_LIB,
            Some(parser::SOURCE_COMMAND) => HELP_SOURCE,
            Some(parser::ORACLE_COMMAND) => self.oracle_help.get_or_insert_with(|| {
                let mut help = HELP_ORACLE.to_string();
                let oracles = debugger.all_oracles();
                oracles.for_each(|oracle| help = format!("{help}{}\n", oracle.help()));
                help
            }),
            Some("tui") => HELP_TUI,
            Some("q") | Some("quit") => HELP_QUIT,
            _ => "unknown command",
        }
    }
}
