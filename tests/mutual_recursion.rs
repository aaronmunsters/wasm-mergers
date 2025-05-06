use wasm_mergers::{MergeConfiguration, NamedModule};
use wasmtime::*;
use wat::parse_str;

/*  +-----------+
    |   func_a  |<---.
    +-----+-----+    |
          |          |
          v          |
    +-----+-----+    |
    |   func_b  |    |
    +-----+-----+    |
          |          |
          v          |
    +-----+-----+    |
    |   func_c  |    | [Mutual Recursion]
    +-----+-----+    |
          |          |
          v          |
    +-----+-----+    |
    |   func_d  |    |
    +-----+-----+    |
          |          |
          v          |
    +-----+-----+    |
    |   func_e  | ---`
    +-----+-----+
*/

const WAT_MOD_ABCDE: &str = r#"
(module
  (func $func_a (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 100)) ;; Return 100 to signify done in A
      (else (call $func_b (i32.sub (local.get $n) (i32.const 1))))))

  (func $func_b (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 200)) ;; Done in B
      (else (call $func_c (i32.sub (local.get $n) (i32.const 1))))))

  (func $func_c (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 300)) ;; Done in C
      (else (call $func_d (i32.sub (local.get $n) (i32.const 1))))))

  (func $func_d (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 400)) ;; Done in D
      (else (call $func_e (i32.sub (local.get $n) (i32.const 1))))))

  (func $func_e (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 500)) ;; Done in E
      (else (call $func_a (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_a" (func $func_a)))
"#;

const WAT_MOD_A: &str = r#"
(module
  (import "WAT_MOD_B" "func_b" (func $func_b (param i32) (result i32)))

  (func $func_a (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 100))
      (else (call $func_b (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_a" (func $func_a))
)
"#;

const WAT_MOD_B: &str = r#"
(module
  (import "WAT_MOD_C" "func_c" (func $func_c (param i32) (result i32)))

  (func $func_b (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 200))
      (else (call $func_c (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_b" (func $func_b))
)
"#;

const WAT_MOD_C: &str = r#"
(module
  (import "WAT_MOD_D" "func_d" (func $func_d (param i32) (result i32)))

  (func $func_c (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 300))
      (else (call $func_d (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_c" (func $func_c))
)

"#;

const WAT_MOD_D: &str = r#"
(module
  (import "WAT_MOD_E" "func_e" (func $func_e (param i32) (result i32)))

  (func $func_d (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 400))
      (else (call $func_e (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_d" (func $func_d))
)
"#;

const WAT_MOD_E: &str = r#"
(module
  (import "WAT_MOD_A" "func_a" (func $func_a (param i32) (result i32)))

  (func $func_e (param $n i32) (result i32)
    (if (result i32)
      (i32.le_s (local.get $n) (i32.const 0))
      (then (i32.const 500))
      (else (call $func_a (i32.sub (local.get $n) (i32.const 1))))))
  (export "func_e" (func $func_e))
)
"#;

#[test]
fn merge_even_odd() {
    let manual_merged = { parse_str(WAT_MOD_ABCDE).unwrap() };
    let lib_merged = {
        let wat_mod_a = parse_str(WAT_MOD_A).unwrap();
        let wat_mod_b = parse_str(WAT_MOD_B).unwrap();
        let wat_mod_c = parse_str(WAT_MOD_C).unwrap();
        let wat_mod_d = parse_str(WAT_MOD_D).unwrap();
        let wat_mod_e = parse_str(WAT_MOD_E).unwrap();

        let modules: &[NamedModule<'_, &[u8]>] = &[
            NamedModule::new("WAT_MOD_A", &wat_mod_a),
            NamedModule::new("WAT_MOD_B", &wat_mod_b),
            NamedModule::new("WAT_MOD_C", &wat_mod_c),
            NamedModule::new("WAT_MOD_D", &wat_mod_d),
            NamedModule::new("WAT_MOD_E", &wat_mod_e),
        ];

        MergeConfiguration::new(modules).merge().unwrap()
    };

    // Structural assertion
    {
        let manual_merged_len = manual_merged.len() as f64;
        let lib_merged_len = lib_merged.len() as f64;
        let ratio = manual_merged_len / lib_merged_len;
        const RATIO_ALLOWED_DELTA: f64 = 0.1; // 10% difference
        assert!(
            (1.0 - RATIO_ALLOWED_DELTA..=1.0 + RATIO_ALLOWED_DELTA).contains(&ratio),
            "Lengths differ by more than 50%: manual = {manual_merged_len}, lib = {lib_merged_len}",
        );
    }

    // Behavioral assertion
    for merged_wasm in [lib_merged, manual_merged] {
        // Interpret even & odd
        let mut store = Store::<()>::default();
        let module = Module::from_binary(store.engine(), &merged_wasm).unwrap();
        let instance = Instance::new(&mut store, &module, &[]).unwrap();

        // Fetch `even` and `odd` export
        let func_a = instance
            .get_typed_func::<i32, i32>(&mut store, "func_a")
            .unwrap();

        for i in 0..100 {
            let result = func_a.call(&mut store, i).unwrap();
            let expected_result = match i % 5 {
                0 => 100, // func_a will return 100 when n == 0
                1 => 200, // func_b will return 200 when n == 1
                2 => 300, // func_c will return 300 when n == 2
                3 => 400, // func_d will return 400 when n == 3
                _ => 500, // func_e will return 500 when n == 4
            };

            assert_eq!(result, expected_result, "Failed for input {i}");
        }
    }
}
