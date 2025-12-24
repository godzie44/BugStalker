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
	let mut outer = Outer {
		a: 10,
		inner: Inner { x: 1, y: 2 },
		vec: vec![Inner { x: 3, y: 4 }, Inner { x: 5, y: 6 }],
	};

	outer.a += 1; // breakpoint #1
	outer.inner.x += 10;
	outer.vec[0].y += 100; // breakpoint #2

	println!("{outer:?}");
}
