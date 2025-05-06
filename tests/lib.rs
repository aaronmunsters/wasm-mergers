use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

const WAT_ODD: &str = include_str!("odd.wat");
const WAT_EVEN: &str = include_str!("even.wat");
const WAT_EVEN_ODD: &str = include_str!("even_odd.wat");

// A test would be:
// Have a set of 10 or more modules that have an interconnecting dependency.
// Then go over all partitions of this collection
// Per partition {Par}, let {Per} be a permutation of {Par}
// for every per, let result be the fold of merging
// test that all merges are correct
// INPUT {a,b,c}
// OUTPUT {({a,b,c}), ({a}, {b}, {c}), ({a, b}, {c}), ({a, c}, {b}), ({a}, {b, c})}

// Package of interest: partitions = { version = "0.2" }

// Another way would be to 'split' modules eg. the benchmark into multiple
// variants ...

/*
CASE 1: 🟠
(mod A
    (def 0 ...)
    (export 0 as "a" ...))

(mod B
    (import "a" as $a ...)
    (export $a as "z" ...))

(mod C
    (import "z" as $z ...)
    (call $z))
*/

/*
CASE 2: 🔴
(mod A
    (import "z" as $z ...)
    (def 0 ...)
    (export 0 as "a" ...)
    (call $z))

(mod B
    (import "a" as $a ...)
    (export $a as "z" ...))
*/

/*
CASE 3: 🟣
        (mod
            (import "a" as 0)
            (import "b" as 1)
            (import "c" as 2)
            (def 3 as "d" ...call-0...)
            (def 4 as "e" ...call-1...))

        (mod
            (def 0 as "a" ...)
            (def 1 as "a" ...)
            (export "a" as 0)
            (export "b" as 1))

        (mod
            (import "b" as 0)
            (def 1 as "f" ...call-0...))

        ==> Merged:

        (mod
            (def 0 as "a" ...)
            (def 1 as "b" ...)
            (import "c" as 2)
            (def 3 as "d" ...call-0...)
            (def 4 as "e ...call-1...)
            (def 5 as "f" ...call-1...))
        */

#[test]
fn merge_even_odd() {
    let wat_even = parse_str(WAT_EVEN).unwrap();
    let wat_odd = parse_str(WAT_ODD).unwrap();

    let modules: &[NamedModule<'_, &[u8]>] = &[
        NamedModule {
            name: "even",
            module: &wat_even,
        },
        NamedModule {
            name: "odd",
            module: &wat_odd,
        },
    ];

    // Merge even & odd
    let merged_wasm = MergeConfiguration::new(modules).merge().unwrap();

    // Interpret even & odd
    let mut store = Store::<()>::default();
    let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

    // Fetch `even` and `odd` export
    let even = instance
        .get_typed_func::<i32, i32>(&mut store, "even")
        .unwrap();

    let odd = instance
        .get_typed_func::<i32, i32>(&mut store, "odd")
        .unwrap();

    assert_eq!(even.call(&mut store, 12345).unwrap(), 0);
    assert_eq!(even.call(&mut store, 12346).unwrap(), 1);
    assert_eq!(odd.call(&mut store, 12345).unwrap(), 1);
    assert_eq!(odd.call(&mut store, 12346).unwrap(), 0);
}

#[test]
fn merge_even_odd_merged() {
    let wat_odd = parse_str(WAT_EVEN_ODD).unwrap();

    // Merge even & odd
    let merged_wasm = wat_odd;

    // Interpret even & odd
    let mut store = Store::<()>::default();
    let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
    let instance = Instance::new(&mut store, &module, &[]).unwrap();

    // Fetch `even` and `odd` export
    let even = instance
        .get_typed_func::<i32, i32>(&mut store, "even")
        .unwrap();

    let odd = instance
        .get_typed_func::<i32, i32>(&mut store, "odd")
        .unwrap();

    assert_eq!(even.call(&mut store, 12345).unwrap(), 0);
    assert_eq!(even.call(&mut store, 12346).unwrap(), 1);
    assert_eq!(odd.call(&mut store, 12345).unwrap(), 1);
    assert_eq!(odd.call(&mut store, 12346).unwrap(), 0);
}
