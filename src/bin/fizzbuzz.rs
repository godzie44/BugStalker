//! Sure this debugee little bit overengineering, this was done intentionally
//! for test the debugger work with code with type polymorphism in it

enum FizzBuzz {
    Fizz,
    Buzz,
    FizzBuzz,
    None,
}

trait Printer {
    fn print(&self, fizzbuzz: FizzBuzz);
}

struct PrettyPrinter {}

impl Printer for PrettyPrinter {
    fn print(&self, fizzbuzz: FizzBuzz) {
        match fizzbuzz {
            FizzBuzz::Fizz => println!("fizz"),
            FizzBuzz::Buzz => println!("buzz"),
            FizzBuzz::FizzBuzz => println!("fizzbuzz"),
            FizzBuzz::None => {}
        }
    }
}

struct BrokenPrinter {}

impl Printer for BrokenPrinter {
    fn print(&self, fizzbuzz: FizzBuzz) {
        match fizzbuzz {
            FizzBuzz::Fizz => println!("???"),
            FizzBuzz::Buzz => println!("???"),
            FizzBuzz::FizzBuzz => println!("???"),
            FizzBuzz::None => println!("i'm broken :("),
        }
    }
}

trait Comparator {
    fn divisible(&self, number: u32) -> FizzBuzz;
}

struct GoodComparator {}

impl Comparator for GoodComparator {
    fn divisible(&self, number: u32) -> FizzBuzz {
        if number % 3 == 0 && number % 5 == 0 {
            FizzBuzz::FizzBuzz
        } else if number % 5 == 0 {
            FizzBuzz::Buzz
        } else if number % 3 == 0 {
            FizzBuzz::Fizz
        } else {
            FizzBuzz::None
        }
    }
}

struct BadComparator {}

impl Comparator for BadComparator {
    fn divisible(&self, _number: u32) -> FizzBuzz {
        FizzBuzz::None
    }
}

struct FizzBuzzSolver<P: Printer, CMP: Comparator> {
    printer: P,
    comparator: CMP,
}

impl<P: Printer, CMP: Comparator> FizzBuzzSolver<P, CMP> {
    fn new(printer: P, comparator: CMP) -> Self {
        Self {
            printer,
            comparator,
        }
    }

    fn solve(&self, number: u32) {
        let res = self.comparator.divisible(number);
        self.printer.print(res);
    }
}

pub fn main() {
    let ok_solver = FizzBuzzSolver::new(PrettyPrinter {}, GoodComparator {});
    ok_solver.solve(9);

    let not_ok_solver = FizzBuzzSolver::new(BrokenPrinter {}, GoodComparator {});
    not_ok_solver.solve(9);

    let not_ok_solver = FizzBuzzSolver::new(PrettyPrinter {}, BadComparator {});
    not_ok_solver.solve(9);
}
