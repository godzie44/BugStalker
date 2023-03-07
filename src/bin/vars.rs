#[allow(unused)]
fn scalar_types() {
    let int8 = 1_i8;
    let int16 = -1_i16;
    let int32 = 2_i32;
    let int64 = -2_i64;
    let int128 = 3_i128;
    let isize = -3_isize;

    let uint8 = 1_u8;
    let uint16 = 2_u16;
    let uint32 = 3_u32;
    let uint64 = 4_u64;
    let uint128 = 5_u128;
    let usize = 6_usize;

    let f32 = 1.1_f32;
    let f64 = 1.2_f64;

    let boolean_true = true;
    let boolean_false = false;

    let char_ascii = 'a';
    let char_non_ascii = '😊';

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn compound_types() {
    let tuple_0 = ();
    let tuple_1 = (0f64, 1.1f64);
    let tuple_2 = (1u64, -1i64, 'a', false);

    struct Foo {
        bar: i32,
        baz: char,
    };
    let foo = Foo { bar: 100, baz: '9' };

    struct Foo2 {
        foo: Foo,
        additional: bool,
    };
    let foo2 = Foo2 {
        foo,
        additional: true,
    };

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn array() {
    let arr_1 = [1, -1, 2, -2, 3];

    let arr_2 = [[1, -1, 2, -2, 3], [0, 1, 2, 3, 4], [0, -1, -2, -3, -4]];

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn enums() {
    enum EnumA {
        A,
        B,
    }
    let enum_1 = EnumA::B;

    enum EnumC {
        C(char),
        D(f64, f32),
        E,
    }
    let enum_2 = EnumC::C('b');
    let enum_3 = EnumC::D(1.1, 1.2);
    let enum_4 = EnumC::E;

    struct Foo {
        a: i32,
        b: char,
    }
    enum EnumF {
        F(EnumC),
        G(Foo),
        J(EnumA),
    }
    let enum_5 = EnumF::F(EnumC::C('f'));
    let enum_6 = EnumF::G(Foo { a: 1, b: '1' });
    let enum_7 = EnumF::J(EnumA::A);

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn references() {
    let a = 2;
    let ref_a = &a;
    let ptr_a: *const i32 = &a;
    let ptr_ptr_a: *const *const i32 = &ptr_a;
    let mut b = 2;
    let mut_ref_b = &mut b;
    let mut c = 2;
    let mut_ptr_c: *mut i32 = &mut b;
    let box_d = Box::new(2);

    struct Foo<'a> {
        bar: i32,
        baz: [i32; 2],
        foo: &'a i32,
    }
    let f = Foo {
        bar: 1,
        baz: [1, 2],
        foo: &a,
    };
    let ref_f = &f;

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn type_alias() {
    type I32Alias = i32;
    let a_alias: I32Alias = 1;

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn type_params() {
    struct Foo<T> {
        bar: T,
    };
    let a = Foo { bar: 1 };

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn vec_and_slice_types() {
    let vec1 = vec![1, 2, 3];

    struct Foo {
        foo: i32,
    }
    let vec2 = vec![Foo { foo: 1 }, Foo { foo: 2 }];

    let vec3 = vec![vec1.clone(), vec1.clone()];

    let slice1 = &[1, 2, 3];
    let slice2 = &[slice1, slice1];

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn string_types() {
    let s1 = "hello world".to_string();
    let s2 = s1.as_str();
    let s3 = "hello world";

    let nop: Option<u8> = None;
}

static GLOB_1: &str = "glob_1";
static GLOB_2: i32 = 2;

#[allow(unused)]
fn static_vars() {
    println!("{GLOB_1}");
    println!("{GLOB_2}");
    let nop: Option<u8> = None;
}

static GLOB_3: i32 = 3;
mod ns_1 {
    pub static GLOB_3: &str = "glob_3";
}

#[allow(unused)]
fn static_vars_same_name() {
    println!("{GLOB_3}");
    println!("{}", ns_1::GLOB_3);
    let nop: Option<u8> = None;
}

thread_local! {
    static THREAD_LOCAL_VAR_1: std::cell::Cell<i32> = std::cell::Cell::new(0);
    static THREAD_LOCAL_VAR_2: std::cell::Cell<&'static str> = std::cell::Cell::new("0");
}

#[allow(unused)]
fn thread_local() {
    THREAD_LOCAL_VAR_1.with(|tl1| tl1.set(1));
    THREAD_LOCAL_VAR_2.with(|tl2| tl2.set("1"));

    let t1 = std::thread::spawn(|| {
        THREAD_LOCAL_VAR_1.with(|tl1| tl1.set(2));
        THREAD_LOCAL_VAR_2.with(|tl2| tl2.set("2"));
        let nop: Option<u8> = None;
    });
    t1.join();

    let t2 = std::thread::spawn(|| {
        let nop: Option<u8> = None;
    });
    t2.join();

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn fn_and_closure() {
    let inc = |a: i32| -> i32 { a + 1 };
    let inc_mut = |a: &mut i32| *a += 1;

    let outer = "outer val".to_string();
    let closure = move || println!("{outer}");

    let (a, b, c) = ("a".to_string(), "b".to_string(), "c".to_string());
    let trait_once: Box<dyn FnOnce()> = Box::new(move || println!("{a}"));
    let trait_mut: Box<dyn FnMut()> = Box::new(move || println!("{b}"));
    let trait_fn: Box<dyn Fn()> = Box::new(move || println!("{c}"));

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn arguments(by_val: i32, by_ref: &i32, vec: Vec<u8>, box_arr: Box<[u8]>) {
    println!("{by_val}");
    println!("{by_ref}");
    println!("{vec:?}");
    println!("{box_arr:?}");

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn unions() {
    #[repr(C)]
    union Union1 {
        f1: f32,
        u2: u64,
        u3: u8,
    }
    let union = Union1 { f1: 1.1 };

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn hashmap() {
    use std::collections::HashMap;

    let hm1 = HashMap::from([(true, 3i64), (false, 5i64)]);
    let hm2 = HashMap::from([("abc", vec![1, 2, 3]), ("efg", vec![11, 12, 13])]);
    let mut hm3 = HashMap::new();
    for i in 0..100 {
        hm3.insert(i, i);
    }
    let hm4 = HashMap::from([
        ("1".to_string(), HashMap::from([(1, 1), (2, 2)])),
        ("3".to_string(), HashMap::from([(3, 3), (4, 4)])),
    ]);

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn hashset() {
    use std::collections::HashSet;

    let hs1 = HashSet::from([1, 2, 3, 4]);
    let mut hs2 = HashSet::new();
    for i in 0..100 {
        hs2.insert(i);
    }
    let hs3 = HashSet::from([vec![1, 2]]);

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn circular() {
    use std::cell::RefCell;
    use std::rc::Rc;

    enum List {
        Cons(i32, RefCell<Rc<List>>),
        Nil,
    }
    impl List {
        fn tail(&self) -> Option<&RefCell<Rc<List>>> {
            match self {
                List::Cons(_, item) => Some(item),
                List::Nil => None,
            }
        }
    }

    let a_circ = Rc::new(List::Cons(5, RefCell::new(Rc::new(List::Nil))));
    let b_circ = Rc::new(List::Cons(10, RefCell::new(Rc::clone(&a_circ))));

    if let Some(link) = a_circ.tail() {
        *link.borrow_mut() = Rc::clone(&b_circ);
    }

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn lexical_blocks() {
    let alpha = 1;
    {
        let beta = 2;
        {
            let mut gama = 3;
            gama += 1;
        }
    }
    let mut delta = 4;
    delta += 1;

    let nop: Option<u8> = None;
}

#[allow(unused)]
fn btree_map() {
    use std::collections::BTreeMap;

    let hm1 = BTreeMap::from([(true, 3i64), (false, 5i64)]);
    let hm2 = BTreeMap::from([("abc", vec![1, 2, 3]), ("efg", vec![11, 12, 13])]);
    let mut hm3 = BTreeMap::new();
    for i in 0..100 {
        hm3.insert(i, i);
    }

    let hm4 = BTreeMap::from([
        ("1".to_string(), BTreeMap::from([(1, 1), (2, 2)])),
        ("3".to_string(), BTreeMap::from([(3, 3), (4, 4)])),
    ]);

    let nop: Option<u8> = None;
}

pub fn main() {
    scalar_types();
    compound_types();
    array();
    enums();
    references();
    type_alias();
    type_params();
    vec_and_slice_types();
    string_types();
    static_vars();
    static_vars_same_name();
    thread_local();
    fn_and_closure();
    arguments(1, &2, vec![3, 4, 5], Box::new([6, 7, 8]));
    unions();
    hashmap();
    hashset();
    circular();
    lexical_blocks();
    btree_map();
}
