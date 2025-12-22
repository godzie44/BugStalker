#[derive(Debug)]
struct Inner {
    x: i32,
    y: i32,
}

#[derive(Debug)]
struct Outer {
    a: i32,
    inner: Inner,
    vec: Vec<Inner>,
}

fn main() {
    let outer = Outer {
        a: 10,
        inner: Inner { x: 1, y: 2 },
        vec: vec![
            Inner { x: 3, y: 4 },
            Inner { x: 5, y: 6 },
        ],
    };
    dbg!(&outer);
    println!("stop");
}
