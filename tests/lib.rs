use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

const WAT_ODD: &str = include_str!("odd.wat");
const WAT_EVEN: &str = include_str!("even.wat");
const WAT_EVEN_ODD: &str = include_str!("even_odd.wat");

#[test]
fn merge_even_odd() {
    let manual_merged = { parse_str(WAT_EVEN_ODD).unwrap() };
    let lib_merged = {
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

        MergeConfiguration::new(modules).merge().unwrap()
    };

    // Structural assertion
    {
        let manual_merged_len = manual_merged.len() as f64;
        let lib_merged_len = lib_merged.len() as f64;
        let ratio = manual_merged_len / lib_merged_len;
        assert!(
            ratio >= 0.5 && ratio <= 1.5,
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

        for i in 0..10000 {
            assert_eq!(to_bool(even.call(&mut store, i).unwrap()), r_even(i));
            assert_eq!(to_bool(odd.call(&mut store, i).unwrap()), r_odd(i));
        }
    }
}
