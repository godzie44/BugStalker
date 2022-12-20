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

pub fn main() {
    scalar_types();
    compound_types();
    array();
    enums();
}
