use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

const WAT_ODD: &str = r#"
(module
  (import "even" "even" (func $even (param i32) (result i32)))
  (export "odd" (func $odd))
  (func $odd (param $0 i32) (result i32)
    local.get $0
    i32.eqz
    if
    i32.const 0
    return
    end
    local.get $0
    i32.const 1
    i32.sub
    call $even))
"#;

const WAT_EVEN: &str = r#"
(module
  (import "odd" "odd" (func $odd (param i32) (result i32)))
  (export "even" (func $even))
  (func $even (param $0 i32) (result i32)
    local.get $0
    i32.eqz
    if
    i32.const 1
    return
    end
    local.get $0
    i32.const 1
    i32.sub
    call $odd))
"#;

const WAT_EVEN_ODD: &str = r#"
(module
  (func $even (param $0 i32) (result i32)
    local.get $0
    i32.eqz
    if
    i32.const 1
    return
    end
    local.get $0
    i32.const 1
    i32.sub
    call $odd)
  (func $odd (param $0 i32) (result i32)
    local.get $0
    i32.eqz
    if
    i32.const 0
    return
    end
    local.get $0
    i32.const 1
    i32.sub
    call $even)
  (export "even" (func $even))
  (export "odd" (func $odd)))
"#;

#[test]
fn merge_even_odd() {
    let manual_merged = { parse_str(WAT_EVEN_ODD).unwrap() };
    let lib_merged = {
        let wat_even = parse_str(WAT_EVEN).unwrap();
        let wat_odd = parse_str(WAT_ODD).unwrap();

        let modules: &[&NamedModule<'_, &[u8]>] = &[
            &NamedModule::new("even", &wat_even),
            &NamedModule::new("odd", &wat_odd),
        ];

        MergeConfiguration::new(modules).merge().unwrap()
    };

    // Structural assertion
    {
        let manual_merged_len = manual_merged.len() as f64;
        let lib_merged_len = lib_merged.len() as f64;
        let ratio = manual_merged_len / lib_merged_len;
        const RATIO_ALLOWED_DELTA: f64 = 0.20; // 20% difference
        assert!(
            (1.0 - RATIO_ALLOWED_DELTA..=1.0 + RATIO_ALLOWED_DELTA).contains(&ratio),
            "Lengths differ by more than 50%: manual = {manual_merged_len}, lib = {lib_merged_len}",
        );
    }

    #[rustfmt::skip]
    fn r_even(v: i32) -> bool { v % 2 == 0 }
    #[rustfmt::skip]
    fn r_odd(v: i32) -> bool { !(r_even(v)) }

    // Behavioral assertion
    for merged_wasm in [lib_merged, manual_merged] {
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

        fn to_bool(v: i32) -> bool {
            assert!(v == 0 || v == 1);
            v == 1
        }

        for i in 0..1000 {
            assert_eq!(to_bool(even.call(&mut store, i).unwrap()), r_even(i));
            assert_eq!(to_bool(odd.call(&mut store, i).unwrap()), r_odd(i));
        }
    }
}
