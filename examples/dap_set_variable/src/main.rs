use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone)]
struct Container {
    point: Point,
    tuple: (i32, bool, char),
    bytes: [u8; 4],
    numbers: [i16; 3],
}

#[derive(Debug, Clone)]
enum Shape {
    Circle { radius: u32 },
    Rect(u32, u32),
}

fn main() {
    let mut container = Container {
        point: Point { x: 10, y: 20 },
        tuple: (7, true, 'Z'),
        bytes: [1, 2, 3, 4],
        numbers: [5, 6, 7],
    };

    let mut shape = Shape::Circle { radius: 12 };

    // breakpoint: inspect and set composite values in DAP (container/shape)
    container.point.x += 1;
    container.bytes[2] = 99;
    shape = Shape::Rect(3, 4);

    println!("{container:?} {shape:?}");
    thread::sleep(Duration::from_secs(60));
}
